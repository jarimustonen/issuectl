//! Filesystem → EventHub bridge.
//!
//! Watches `<root>/issues/` via `notify-debouncer-full`, filters tempfiles
//! and unrelated paths, parses changed `item.md` files in
//! `spawn_blocking`, and publishes `IssueUpserted` / `IssueRemoved` /
//! `IssueInvalid` to the hub.
//!
//! Bulk windows that touch more than `bulk_threshold` distinct slugs (e.g.
//! a `git checkout` across feature branches) coalesce into a single
//! `Resync { reason: "bulk_change" }` instead of fanning out per-issue.
//!
//! On panic, the supervising loop restarts the watcher with exponential
//! backoff up to 3 attempts; each successful (re)start emits `Resync {
//! reason: "watcher_restart" }`. After 3 failures, `Degraded { reason:
//! "watcher_unavailable" }` lands and the task exits.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, DebouncedEvent};
use tokio::sync::mpsc;

/// Backend selection for the watcher. `Recommended` picks the platform's
/// native backend (inotify/FSEvents/etc.); `Poll(interval)` forces the
/// polling backend, the documented workaround for network filesystems
/// where `notify` events are unreliable. See design doc §8.1.
#[derive(Debug, Clone, Copy)]
pub enum WatcherBackend {
    Recommended,
    Poll(Duration),
}

/// `run_once` failure modes. `Transient` failures retry with backoff
/// up to `MAX_CONSECUTIVE_FAILURES`; `Terminal` failures skip retry
/// and Degrade immediately. The classic Terminal case is
/// `MaxFilesWatch` — burning 3 retries against an exhausted inotify
/// limit is pointless and just delays the user-visible banner.
enum RunFailure {
    Transient(String),
    Terminal(String),
}

use super::events::{EventHub, EventPayload};
use crate::repo::IssueSummary;
use crate::slug;

/// Watcher configuration. All defaults match the design doc §5–§5.7.
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    /// Repo root (the directory containing `issues/`).
    pub root: PathBuf,
    /// Debounce window. Design doc range: 100–200 ms.
    pub debounce: Duration,
    /// Distinct slug count within a single debounce window above which
    /// per-issue events collapse into one `Resync`.
    pub bulk_threshold: usize,
    /// Backend selection. Defaults to `Recommended`; `Poll` forces the
    /// polling backend when `--watch-poll-ms` is passed.
    pub backend: WatcherBackend,
}

/// Latched degradation reason. Shared between the supervisor (writer
/// on terminal Degraded) and the `/api/session` handler (reader). Lets
/// fresh clients connecting after the SSE Degraded event aged out of
/// replay still see the banner.
pub type WatchDegraded = Arc<parking_lot::Mutex<Option<String>>>;

/// Spawn the watcher supervisor. The returned handle lives for as long
/// as the server; aborting it stops the watcher.
pub fn spawn(
    hub: Arc<EventHub>,
    watch_degraded: WatchDegraded,
    cfg: WatcherConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move { supervisor(hub, watch_degraded, cfg).await })
}

async fn supervisor(hub: Arc<EventHub>, watch_degraded: WatchDegraded, cfg: WatcherConfig) {
    const MAX_CONSECUTIVE_FAILURES: u32 = 3;
    /// Minimum healthy runtime before failures are considered "consecutive".
    /// A watcher that ran successfully for ≥ this duration before failing
    /// has earned a fresh failure budget — months of uptime should not
    /// accumulate a Degraded.
    const HEALTHY_THRESHOLD: Duration = Duration::from_secs(30);

    let mut consecutive_failures: u32 = 0;
    loop {
        let started_at = Instant::now();
        // F5: spawn run_once as a child task so panics surface as
        // JoinError instead of unwinding the supervisor itself.
        let result = tokio::spawn(run_once(hub.clone(), cfg.clone())).await;

        let ran_healthy = started_at.elapsed() >= HEALTHY_THRESHOLD;
        if ran_healthy {
            consecutive_failures = 0;
        }

        let (err_msg, terminal) = match result {
            Ok(Ok(())) => {
                // F4: run_once returns Ok(()) only on graceful shutdown
                // (currently never — the loop only exits via Err). If we
                // ever add an explicit shutdown signal the path is here.
                return;
            }
            Ok(Err(RunFailure::Transient(m))) => (m, false),
            Ok(Err(RunFailure::Terminal(m))) => (m, true),
            Err(join_err) => {
                if join_err.is_cancelled() {
                    // Task aborted (server shutdown). Stop cleanly.
                    return;
                }
                (format!("watcher panicked: {join_err}"), false)
            }
        };

        consecutive_failures += 1;
        log_warn(&format!(
            "watcher attempt failed (consecutive={consecutive_failures}, terminal={terminal}): {err_msg}"
        ));

        // M8: a Terminal classification short-circuits the retry budget.
        // For inotify watch-limit exhaustion (`MaxFilesWatch`) further
        // retries cannot succeed without operator action; emit Degraded
        // immediately rather than burning ~600ms of useless backoff
        // while the board sits silent.
        if terminal || consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            *watch_degraded.lock() = Some("watcher_unavailable".to_string());
            hub.publish(EventPayload::Degraded {
                reason: "watcher_unavailable".to_string(),
            });
            return;
        }

        let backoff = Duration::from_millis(200 * (1u64 << (consecutive_failures - 1)));
        tokio::time::sleep(backoff).await;
    }
}

fn log_warn(msg: &str) {
    eprintln!("issuectl[watcher]: {msg}");
}

async fn run_once(hub: Arc<EventHub>, cfg: WatcherConfig) -> Result<(), RunFailure> {
    let issues_root = cfg.root.join("issues");
    let issues_root_canon = std::fs::canonicalize(&issues_root).map_err(|e| {
        RunFailure::Transient(format!(
            "cannot canonicalize {}: {e}",
            issues_root.display()
        ))
    })?;

    // Channel from the (sync) debouncer callback into our async loop.
    // Unbounded: the debouncer already coalesces into windows, and a
    // brief stall must not drop events.
    let (tx, mut rx) = mpsc::unbounded_channel::<DebounceEventResult>();

    // F12: explicit notify config. Symlinks must NOT be followed —
    // otherwise `ln -s /etc issues/open/foo` makes the watcher observe
    // /etc, violating §5.1/§9.5.
    let mut notify_config = notify::Config::default().with_follow_symlinks(false);
    let cb = move |res: DebounceEventResult| {
        let _ = tx.send(res);
    };

    // The debouncer is generic over the watcher type. Forced polling
    // (network FS workaround per §8.1) uses `notify::PollWatcher` and
    // applies the configured tick interval; the recommended backend
    // (inotify/FSEvents/ReadDirectoryChanges) handles the common case.
    // Two parallel ownership types are unavoidable because of the
    // generic parameter; we drive each through the same event-pump
    // loop afterwards.
    enum AnyDebouncer {
        Recommended(
            notify_debouncer_full::Debouncer<
                notify::RecommendedWatcher,
                notify_debouncer_full::RecommendedCache,
            >,
        ),
        Poll(
            notify_debouncer_full::Debouncer<
                notify::PollWatcher,
                notify_debouncer_full::RecommendedCache,
            >,
        ),
    }

    let mut debouncer = match cfg.backend {
        WatcherBackend::Recommended => {
            let d = notify_debouncer_full::new_debouncer_opt::<
                _,
                notify::RecommendedWatcher,
                notify_debouncer_full::RecommendedCache,
            >(
                cfg.debounce,
                None,
                cb,
                notify_debouncer_full::RecommendedCache::new(),
                notify_config,
            )
            .map_err(|e| RunFailure::Transient(format!("debouncer init: {e}")))?;
            AnyDebouncer::Recommended(d)
        }
        WatcherBackend::Poll(interval) => {
            notify_config = notify_config.with_poll_interval(interval);
            let d = notify_debouncer_full::new_debouncer_opt::<
                _,
                notify::PollWatcher,
                notify_debouncer_full::RecommendedCache,
            >(
                cfg.debounce,
                None,
                cb,
                notify_debouncer_full::RecommendedCache::new(),
                notify_config,
            )
            .map_err(|e| RunFailure::Transient(format!("poll debouncer init: {e}")))?;
            AnyDebouncer::Poll(d)
        }
    };

    let watch_err_map = |e: notify::Error| {
        // Hand back a Terminal so the supervisor stops burning the
        // retry budget on cases where the next attempt cannot succeed
        // without operator intervention.
        let msg = format!("watch {}: {}", issues_root_canon.display(), e);
        if is_terminal_notify_error(&e) {
            RunFailure::Terminal(msg)
        } else {
            RunFailure::Transient(msg)
        }
    };
    match &mut debouncer {
        AnyDebouncer::Recommended(d) => d
            .watch(&issues_root_canon, RecursiveMode::Recursive)
            .map_err(watch_err_map)?,
        AnyDebouncer::Poll(d) => d
            .watch(&issues_root_canon, RecursiveMode::Recursive)
            .map_err(watch_err_map)?,
    }

    // F10: publish (re)start Resync only AFTER the watch is hooked.
    // F11: always "watcher_restart" — spec only enumerates this reason
    // for both first start and subsequent restarts.
    hub.publish(EventPayload::Resync {
        reason: "watcher_restart".to_string(),
    });

    // Pump events until the channel closes (F4: closure means failure,
    // not graceful shutdown — debouncer thread death drops `tx`).
    while let Some(res) = rx.recv().await {
        match res {
            Ok(events) => {
                handle_batch(&hub, &cfg, &issues_root_canon, events).await;
            }
            Err(errs) => {
                for e in &errs {
                    log_warn(&format!("notify error: {e}"));
                }
                // F18: classify. Watch-removal/backend death → restart.
                // Transient errors (queue overflow, IO blip) → Resync
                // and keep running.
                if let Some(err) = errs.iter().find(|e| is_fatal_notify_error(e)) {
                    let msg = format!("notify backend failed: {} errors", errs.len());
                    return Err(if is_terminal_notify_error(err) {
                        RunFailure::Terminal(msg)
                    } else {
                        RunFailure::Transient(msg)
                    });
                }
                hub.publish(EventPayload::Resync {
                    reason: "gap".to_string(),
                });
            }
        }
    }

    // F4: receiver returned None means tx side dropped without us
    // dropping the debouncer first. The debouncer is still owned here,
    // so its callback closure shouldn't have dropped — yet rx is empty.
    // Treat as failure so supervisor retries. Actual graceful shutdown
    // happens via task abort, not via this path.
    drop(debouncer);
    Err(RunFailure::Transient(
        "watcher event channel closed unexpectedly".to_string(),
    ))
}

/// Classify notify errors. Fatal kinds require a watcher restart;
/// non-fatal kinds (overflow, transient IO) only warrant a Resync.
fn is_fatal_notify_error(err: &notify::Error) -> bool {
    use notify::ErrorKind;
    matches!(
        err.kind,
        ErrorKind::PathNotFound
            | ErrorKind::WatchNotFound
            | ErrorKind::MaxFilesWatch
            | ErrorKind::InvalidConfig(_)
    )
}

/// Subset of fatal kinds where a retry can never succeed without
/// operator action. The supervisor short-circuits the retry budget on
/// these and Degrades immediately. `MaxFilesWatch` (inotify limit
/// exhausted) and `InvalidConfig` (programmer error) are the canonical
/// cases. `PathNotFound` / `WatchNotFound` stay transient — the watch
/// dir might come back (e.g. brief unmount/remount).
fn is_terminal_notify_error(err: &notify::Error) -> bool {
    use notify::ErrorKind;
    matches!(
        err.kind,
        ErrorKind::MaxFilesWatch | ErrorKind::InvalidConfig(_)
    )
}

/// One debounced batch. Resolves every relevant path to a slug, then
/// either coalesces (>= bulk_threshold distinct slugs) or fans out.
async fn handle_batch(
    hub: &Arc<EventHub>,
    cfg: &WatcherConfig,
    issues_root_canon: &Path,
    events: Vec<DebouncedEvent>,
) {
    let mut affected: HashSet<String> = HashSet::new();
    for evt in &events {
        // Only act on events that could change issue content. Access
        // events and metadata-only changes don't affect rendered state.
        if !is_relevant_kind(&evt.event.kind) {
            continue;
        }
        for path in &evt.event.paths {
            if let Some(slug) = issue_slug_from_event(issues_root_canon, path) {
                affected.insert(slug);
            }
        }
    }

    if affected.is_empty() {
        return;
    }

    if affected.len() >= cfg.bulk_threshold {
        hub.publish(EventPayload::Resync {
            reason: "bulk_change".to_string(),
        });
        return;
    }

    // F16: parse all slugs concurrently via spawn_blocking, then
    // publish in a deterministic order. Sequential await would stall
    // a 49-slug batch on 49 disk reads in a row.
    let root = cfg.root.clone();
    let mut tasks = Vec::with_capacity(affected.len());
    for slug in affected {
        let r = root.clone();
        tasks.push((
            slug.clone(),
            tokio::task::spawn_blocking(move || parse_slug_state(&r, &slug)),
        ));
    }

    let mut removed = Vec::new();
    let mut invalid = Vec::new();
    let mut upserted = Vec::new();
    for (slug, task) in tasks {
        match task.await {
            Ok(ParseOutcome::Vanished) => removed.push(slug),
            Ok(ParseOutcome::Invalid { warnings }) => invalid.push((slug, warnings)),
            Ok(ParseOutcome::Loaded { summary, version }) => {
                upserted.push((slug, summary, version))
            }
            Err(join_err) => {
                // F8: A panic in parse_slug_state lands here. Don't
                // silently drop the slug — log and emit a Resync so the
                // client refetches authoritative state.
                log_warn(&format!("parse task failed for {slug}: {join_err}"));
                hub.publish(EventPayload::Resync {
                    reason: "parse_task_failed".to_string(),
                });
                return;
            }
        }
    }

    // F20: publish Removed before Upserted so a slug rename (Remove
    // old + Upsert new in one batch) lands in the right order on the
    // client. Within each class we sort by slug so that interleaving
    // is also deterministic across runs (helps testing).
    removed.sort();
    invalid.sort_by(|a, b| a.0.cmp(&b.0));
    upserted.sort_by(|a, b| a.0.cmp(&b.0));

    for slug in removed {
        hub.publish(EventPayload::IssueRemoved { slug });
    }
    for (slug, warnings) in invalid {
        hub.publish(EventPayload::IssueInvalid { slug, warnings });
    }
    for (slug, summary, version) in upserted {
        hub.publish(EventPayload::IssueUpserted {
            slug,
            version,
            issue: summary,
        });
    }
}

enum ParseOutcome {
    Loaded {
        summary: Box<IssueSummary>,
        version: String,
    },
    Invalid {
        warnings: Vec<crate::repo::LoadWarning>,
    },
    Vanished,
}

/// Maximum size of `item.md` the watcher will load into memory. Files
/// past this cap are surfaced as `IssueInvalid` rather than parsed —
/// stops a single accidental commit (or hostile local write) from
/// OOM-ing the server. (M3.)
const MAX_ITEM_MD_BYTES: u64 = 16 * 1024 * 1024;

/// Re-derive the on-disk state of one slug. Runs in `spawn_blocking`.
/// Delegates layout classification to `repo::resolve_layout` (the single
/// source of truth shared with loader/locator/mutate/migrate).
///
/// Post-flat-layout, legacy `issues/{open,closed}/<slug>/` paths are
/// surfaced as `IssueInvalid` with `legacy_layout` — the card stays
/// visible with a warning badge but is not treated as healthy until the
/// user runs `issuectl doctor --fix` (or any write triggers in-line
/// migration under the flock).
fn parse_slug_state(root: &Path, slug: &str) -> ParseOutcome {
    if !slug::is_valid(slug) {
        return ParseOutcome::Vanished;
    }

    let item_path = match crate::repo::resolve_layout(root, slug) {
        crate::repo::LayoutState::Absent => return ParseOutcome::Vanished,
        crate::repo::LayoutState::Flat { item_path } => item_path,
        crate::repo::LayoutState::Legacy { folder, .. } => {
            // M5 / user decision #17: legacy paths are surfaced as
            // Invalid. Card remains visible with a warning until the
            // user migrates; mutate-path writes will migrate it in-line.
            return ParseOutcome::Invalid {
                warnings: vec![crate::repo::LoadWarning {
                    slug: slug.to_string(),
                    folder: folder.to_string(),
                    message: format!(
                        "found at legacy path issues/{folder}/{slug}/ — run `issuectl doctor --fix`"
                    ),
                    code: Some(crate::repo::LoadWarningCode::LegacyLayout),
                }],
            };
        }
        crate::repo::LayoutState::Ambiguous { paths } => {
            return ParseOutcome::Invalid {
                warnings: vec![crate::repo::LoadWarning {
                    slug: slug.to_string(),
                    folder: "ambiguous".to_string(),
                    message: format!(
                        "ambiguous slug — present at: {}",
                        paths
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    code: Some(crate::repo::LoadWarningCode::AmbiguousSlug),
                }],
            };
        }
        crate::repo::LayoutState::Invalid { reason, .. } => {
            return ParseOutcome::Invalid {
                warnings: vec![crate::repo::LoadWarning {
                    slug: slug.to_string(),
                    folder: "open".to_string(),
                    message: reason,
                    code: None,
                }],
            };
        }
    };

    // M3: cap item.md size. `symlink_metadata` is fine here because
    // `resolve_layout` already verified the path isn't a symlink.
    if let Ok(meta) = std::fs::symlink_metadata(&item_path) {
        if meta.len() > MAX_ITEM_MD_BYTES {
            return ParseOutcome::Invalid {
                warnings: vec![crate::repo::LoadWarning {
                    slug: slug.to_string(),
                    folder: "open".to_string(),
                    message: format!(
                        "{} is too large ({} bytes; cap = {} bytes)",
                        item_path.display(),
                        meta.len(),
                        MAX_ITEM_MD_BYTES
                    ),
                    code: Some(crate::repo::LoadWarningCode::TooLarge),
                }],
            };
        }
    }

    let text = match std::fs::read_to_string(&item_path) {
        Ok(t) => t,
        Err(e) => {
            return ParseOutcome::Invalid {
                warnings: vec![crate::repo::LoadWarning {
                    slug: slug.to_string(),
                    folder: "open".to_string(),
                    message: format!("cannot read {}: {}", item_path.display(), e),
                    code: None,
                }],
            };
        }
    };

    let parsed = crate::parser::parse_item_md_text_with_warnings(&text, slug, "open", &item_path);
    // Schema load failure must not stay silent — the readers' fallback
    // to `default_schema()` would otherwise classify a custom
    // `archived: closing` status as active and render the issue in the
    // wrong column with no log trail. Doctor surfaces the same error
    // for offline diagnosis; here we make sure the live server logs
    // scream about it on every event until it's fixed.
    let schema = match crate::schema::load(root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "Warning: schema parse failed at {}; lifecycle classification falls back to built-in defaults until fixed: {e:#}",
                slug
            );
            std::sync::Arc::new(crate::schema::default_schema())
        }
    };
    if !parsed.warnings.is_empty() {
        let derived_folder =
            crate::repo::folder_for_status(&schema, &parsed.issue.status).to_string();
        let warnings = parsed
            .warnings
            .into_iter()
            .map(|w| crate::repo::LoadWarning {
                slug: slug.to_string(),
                folder: derived_folder.clone(),
                message: w,
                code: Some(crate::repo::LoadWarningCode::ParseWarning),
            })
            .collect();
        return ParseOutcome::Invalid { warnings };
    }

    let mut issue = parsed.issue;
    issue.folder = crate::repo::folder_for_status(&schema, &issue.status).to_string();
    let version = crate::canonical::canonical_hash(&issue);
    ParseOutcome::Loaded {
        summary: Box::new(IssueSummary::from(issue)),
        version,
    }
}

fn is_relevant_kind(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Other
    )
}

/// Map a filesystem event path back to an issue slug, or `None` if the
/// path is irrelevant (outside the issues tree, a tempfile, an invalid
/// slug shape).
///
/// Accepts both the canonical flat layout (`issues/<slug>/item.md`)
/// and the legacy `issues/{open,closed}/<slug>/item.md` paths so a
/// repo mid-migration still produces SSE events. `parse_slug_state`
/// then surfaces the legacy or ambiguous state to the client.
pub fn issue_slug_from_event(issues_root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(issues_root).ok()?;
    // F17: filter atomic-write tempfiles by basename.
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.starts_with(".issuectl-tmp-") {
            return None;
        }
    }
    let mut comps = rel.components();
    let first = comps.next()?.as_os_str().to_str()?;
    let slug = if first == "open" || first == "closed" {
        // Legacy compat path: skip the kanban-folder component.
        comps.next()?.as_os_str().to_str()?
    } else {
        // Flat layout: first component is the slug itself.
        first
    };
    if !slug::is_valid(slug) {
        return None;
    }
    Some(slug.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_resolver_extracts_flat_slug() {
        let root = Path::new("/repo/issues");
        let p = Path::new("/repo/issues/quiet-brave-otter/item.md");
        assert_eq!(
            issue_slug_from_event(root, p).as_deref(),
            Some("quiet-brave-otter")
        );
    }

    #[test]
    fn slug_resolver_strips_legacy_open_prefix() {
        let root = Path::new("/repo/issues");
        let p = Path::new("/repo/issues/open/quiet-brave-otter/item.md");
        assert_eq!(
            issue_slug_from_event(root, p).as_deref(),
            Some("quiet-brave-otter")
        );
    }

    #[test]
    fn slug_resolver_handles_legacy_closed() {
        let root = Path::new("/repo/issues");
        let p = Path::new("/repo/issues/closed/tiny-wild-comet/item.md");
        assert_eq!(
            issue_slug_from_event(root, p).as_deref(),
            Some("tiny-wild-comet")
        );
    }

    #[test]
    fn slug_resolver_rejects_invalid_slug() {
        let root = Path::new("/repo/issues");
        let p = Path::new("/repo/issues/INVALID_SLUG/item.md");
        assert!(issue_slug_from_event(root, p).is_none());
    }

    #[test]
    fn slug_resolver_filters_tempfiles() {
        let root = Path::new("/repo/issues");
        let p = Path::new("/repo/issues/quiet-brave-otter/.issuectl-tmp-abc.md");
        assert!(issue_slug_from_event(root, p).is_none());
    }

    #[test]
    fn slug_resolver_rejects_outside_root() {
        let root = Path::new("/repo/issues");
        let p = Path::new("/elsewhere/file.md");
        assert!(issue_slug_from_event(root, p).is_none());
    }

    #[test]
    fn slug_resolver_unaffected_by_temp_components_in_repo_path() {
        let root = Path::new("/tmp/.issuectl-tmp-test/issues");
        let p = Path::new("/tmp/.issuectl-tmp-test/issues/quiet-brave-otter/item.md");
        assert_eq!(
            issue_slug_from_event(root, p).as_deref(),
            Some("quiet-brave-otter")
        );
    }

    /// End-to-end: spawn the watcher against a tempdir, create an
    /// `item.md` on disk, and assert an `IssueUpserted` arrives via
    /// the EventHub broadcast within a few debounce windows. Catches
    /// regressions in debouncer integration, slug resolution, parse
    /// path, and publish ordering.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watcher_publishes_issue_upserted_for_new_file() {
        use std::time::Duration;
        use tokio::time::timeout;

        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("issues")).unwrap();

        let hub = std::sync::Arc::new(EventHub::new());
        let mut rx = hub.tx_subscribe_for_test();

        let cfg = WatcherConfig {
            root: tmp.path().to_path_buf(),
            debounce: Duration::from_millis(50),
            bulk_threshold: 50,
            backend: WatcherBackend::Recommended,
        };
        let watch_degraded: WatchDegraded = std::sync::Arc::new(parking_lot::Mutex::new(None));
        let handle = spawn(hub.clone(), watch_degraded, cfg);

        // First wait out the synthetic Resync{watcher_restart} the
        // watcher emits once it hooks the watch.
        let first = timeout(Duration::from_secs(2), rx.recv()).await;
        assert!(matches!(first, Ok(Ok(_))), "expected Resync, got {first:?}");

        // Now drop a real issue on disk and watch for the event.
        let issue_dir = tmp.path().join("issues/quiet-brave-otter");
        std::fs::create_dir_all(&issue_dir).unwrap();
        std::fs::write(
            issue_dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: high\n---\n\n# It works\n",
        )
        .unwrap();

        // We may receive other events first (e.g. Resync from any
        // notify error during temp-dir setup); loop until we see the
        // expected IssueUpserted or run out of patience.
        let upserted = timeout(Duration::from_secs(5), async {
            loop {
                match rx.recv().await {
                    Ok(evt) => {
                        if let EventPayload::IssueUpserted { ref slug, .. } = evt.payload {
                            if slug == "quiet-brave-otter" {
                                return evt;
                            }
                        }
                    }
                    Err(e) => panic!("broadcast recv error: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for IssueUpserted");

        if let EventPayload::IssueUpserted { version, issue, .. } = &upserted.payload {
            assert!(version.starts_with("sha256:"), "version: {version}");
            assert_eq!(issue.title, "It works");
        } else {
            unreachable!()
        }

        handle.abort();
        let _ = handle.await;
    }

    /// Same end-to-end probe as `watcher_publishes_issue_upserted_for_new_file`,
    /// but with the polling backend. A tempfs smoke test — proves the
    /// `WatcherBackend::Poll` plumbing produces events end-to-end, not
    /// that polling is robust on NFS/SMB / coarse-mtime filesystems
    /// (those need separate environment-specific testing). The poll
    /// interval is short so the test completes within the same
    /// timeout budget as the native test.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watcher_poll_backend_publishes_issue_upserted() {
        use std::time::Duration;
        use tokio::time::timeout;

        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("issues")).unwrap();

        let hub = std::sync::Arc::new(EventHub::new());
        let mut rx = hub.tx_subscribe_for_test();

        let cfg = WatcherConfig {
            root: tmp.path().to_path_buf(),
            debounce: Duration::from_millis(100),
            bulk_threshold: 50,
            backend: WatcherBackend::Poll(Duration::from_millis(100)),
        };
        let watch_degraded: WatchDegraded = std::sync::Arc::new(parking_lot::Mutex::new(None));
        let handle = spawn(hub.clone(), watch_degraded, cfg);

        let first = timeout(Duration::from_secs(2), rx.recv()).await;
        assert!(matches!(first, Ok(Ok(_))), "expected Resync, got {first:?}");

        let issue_dir = tmp.path().join("issues/poll-brave-otter");
        std::fs::create_dir_all(&issue_dir).unwrap();
        std::fs::write(
            issue_dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: high\n---\n\n# Polled\n",
        )
        .unwrap();

        let upserted = timeout(Duration::from_secs(8), async {
            loop {
                match rx.recv().await {
                    Ok(evt) => {
                        if let EventPayload::IssueUpserted { ref slug, .. } = evt.payload {
                            if slug == "poll-brave-otter" {
                                return evt;
                            }
                        }
                    }
                    Err(e) => panic!("broadcast recv error: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for IssueUpserted on poll backend");

        if let EventPayload::IssueUpserted { issue, .. } = &upserted.payload {
            assert_eq!(issue.title, "Polled");
        } else {
            unreachable!()
        }

        handle.abort();
        let _ = handle.await;
    }

    /// C4: lock the supervisor's actually-load-bearing M3 behaviour.
    /// Pointing the watcher at a non-existent root makes
    /// `canonicalize` fail in `run_once` on every attempt; after 3
    /// transient failures the supervisor must publish exactly one
    /// `Degraded`, latch the reason into `watch_degraded`, and
    /// terminate. Without this test the failure-budget logic and the
    /// `WatchDegraded` plumbing rely on manual verification.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn supervisor_emits_degraded_after_three_start_failures() {
        use std::time::Duration;
        use tokio::time::timeout;

        let tmp = tempfile::tempdir().unwrap();
        // Path doesn't contain `issues/`, so canonicalize fails.
        let nonexistent_root = tmp.path().join("not-a-repo");

        let hub = std::sync::Arc::new(EventHub::new());
        let mut rx = hub.tx_subscribe_for_test();
        let watch_degraded: WatchDegraded = std::sync::Arc::new(parking_lot::Mutex::new(None));

        let cfg = WatcherConfig {
            root: nonexistent_root,
            debounce: Duration::from_millis(50),
            bulk_threshold: 50,
            backend: WatcherBackend::Recommended,
        };
        let handle = spawn(hub.clone(), watch_degraded.clone(), cfg);

        let evt = timeout(Duration::from_secs(5), async {
            loop {
                match rx.recv().await {
                    Ok(evt) => {
                        if matches!(evt.payload, EventPayload::Degraded { .. }) {
                            return evt;
                        }
                    }
                    Err(e) => panic!("recv: {e}"),
                }
            }
        })
        .await
        .expect("Degraded never arrived after 3 failed starts");

        if let EventPayload::Degraded { reason } = &evt.payload {
            assert_eq!(reason, "watcher_unavailable");
        } else {
            unreachable!()
        }

        // Supervisor must terminate after publishing Degraded.
        timeout(Duration::from_secs(2), handle)
            .await
            .expect("supervisor should exit after Degraded")
            .ok();

        // The latched reason is what the /api/session handler reads.
        assert_eq!(
            watch_degraded.lock().clone(),
            Some("watcher_unavailable".to_string())
        );
    }
}
