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
}

/// Spawn the watcher supervisor. The returned handle lives for as long
/// as the server; aborting it stops the watcher.
pub fn spawn(hub: Arc<EventHub>, cfg: WatcherConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move { supervisor(hub, cfg).await })
}

async fn supervisor(hub: Arc<EventHub>, cfg: WatcherConfig) {
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

        let err_msg = match result {
            Ok(Ok(())) => {
                // F4: run_once returns Ok(()) only on graceful shutdown
                // (currently never — the loop only exits via Err). If we
                // ever add an explicit shutdown signal the path is here.
                return;
            }
            Ok(Err(err)) => err,
            Err(join_err) => {
                if join_err.is_cancelled() {
                    // Task aborted (server shutdown). Stop cleanly.
                    return;
                }
                format!("watcher panicked: {join_err}")
            }
        };

        consecutive_failures += 1;
        log_warn(&format!(
            "watcher attempt failed (consecutive={consecutive_failures}): {err_msg}"
        ));

        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
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

async fn run_once(hub: Arc<EventHub>, cfg: WatcherConfig) -> Result<(), String> {
    let issues_root = cfg.root.join("issues");
    let issues_root_canon = std::fs::canonicalize(&issues_root)
        .map_err(|e| format!("cannot canonicalize {}: {e}", issues_root.display()))?;

    // Channel from the (sync) debouncer callback into our async loop.
    // Unbounded: the debouncer already coalesces into windows, and a
    // brief stall must not drop events.
    let (tx, mut rx) = mpsc::unbounded_channel::<DebounceEventResult>();

    // F12: explicit notify config. Symlinks must NOT be followed —
    // otherwise `ln -s /etc issues/open/foo` makes the watcher observe
    // /etc, violating §5.1/§9.5.
    let notify_config = notify::Config::default().with_follow_symlinks(false);
    let mut debouncer = notify_debouncer_full::new_debouncer_opt::<
        _,
        notify::RecommendedWatcher,
        notify_debouncer_full::RecommendedCache,
    >(
        cfg.debounce,
        None,
        move |res: DebounceEventResult| {
            let _ = tx.send(res);
        },
        notify_debouncer_full::RecommendedCache::new(),
        notify_config,
    )
    .map_err(|e| format!("debouncer init: {e}"))?;

    debouncer
        .watch(&issues_root_canon, RecursiveMode::Recursive)
        .map_err(|e| format!("watch {}: {}", issues_root_canon.display(), e))?;

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
                if errs.iter().any(is_fatal_notify_error) {
                    return Err(format!("notify backend failed: {} errors", errs.len()));
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
    Err("watcher event channel closed unexpectedly".to_string())
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

/// Re-derive the on-disk state of one slug. Runs in `spawn_blocking`.
///
/// The returned outcome distinguishes three filesystem states:
/// - **Vanished**: neither `issues/open/<slug>/` nor `issues/closed/<slug>/`
///   exists, or the slug shape is invalid. Watcher emits `IssueRemoved`.
/// - **Invalid**: the issue dir exists but `item.md` cannot be parsed
///   cleanly (missing, invalid YAML, both folders contain it). Watcher
///   emits `IssueInvalid` so the card stays visible with an error
///   badge — never silently disappear (§5.6, §8.6).
/// - **Loaded**: parsed successfully. Watcher emits `IssueUpserted`.
///
/// Read-once-then-parse so the published `IssueSummary` and `version`
/// always describe the same byte image of `item.md` (no TOCTOU between
/// a "read for parse" and "read for hash" pair of syscalls).
fn parse_slug_state(root: &Path, slug: &str) -> ParseOutcome {
    if !slug::is_valid(slug) {
        return ParseOutcome::Vanished;
    }

    let open_dir = root.join("issues").join("open").join(slug);
    let closed_dir = root.join("issues").join("closed").join(slug);
    let open_exists = is_real_dir(&open_dir);
    let closed_exists = is_real_dir(&closed_dir);

    let folder = match (open_exists, closed_exists) {
        (false, false) => return ParseOutcome::Vanished,
        (true, true) => {
            // Both folders present is the "ambiguous slug" state from
            // §3.4. Don't pick a side here; surface as Invalid so the
            // user is forced to resolve it.
            return ParseOutcome::Invalid {
                warnings: vec![crate::repo::LoadWarning {
                    slug: slug.to_string(),
                    folder: "open+closed".to_string(),
                    message: "ambiguous slug: present in both open/ and closed/ — \
                         resolve manually before issuectl serve will trust this issue"
                        .to_string(),
                }],
            };
        }
        (true, false) => "open",
        (false, true) => "closed",
    };

    let dir = if folder == "open" {
        open_dir
    } else {
        closed_dir
    };
    let item_path = dir.join("item.md");
    let text = match std::fs::read_to_string(&item_path) {
        Ok(t) => t,
        Err(e) => {
            // Folder exists but item.md doesn't (or is unreadable):
            // partial-write or `mkdir` without a save. Surface as
            // Invalid, not Vanished — operator should see the error.
            return ParseOutcome::Invalid {
                warnings: vec![crate::repo::LoadWarning {
                    slug: slug.to_string(),
                    folder: folder.to_string(),
                    message: format!("cannot read {}: {}", item_path.display(), e),
                }],
            };
        }
    };

    let parsed = crate::parser::parse_item_md_text_with_warnings(&text, slug, folder, &item_path);
    if !parsed.warnings.is_empty() {
        let warnings = parsed
            .warnings
            .into_iter()
            .map(|w| crate::repo::LoadWarning {
                slug: slug.to_string(),
                folder: folder.to_string(),
                message: w,
            })
            .collect();
        return ParseOutcome::Invalid { warnings };
    }

    let version = crate::canonical::canonical_hash(&parsed.issue);
    ParseOutcome::Loaded {
        summary: Box::new(IssueSummary::from(parsed.issue)),
        version,
    }
}

/// Real (non-symlink) directory check. Mirrors `repo::locate_issue`'s
/// stance: a symlinked entry is treated as not-a-real-dir to keep
/// notify-driven watching consistent with the read path.
fn is_real_dir(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(m) => m.is_dir() && !m.file_type().is_symlink(),
        Err(_) => false,
    }
}

fn is_relevant_kind(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Other
    )
}

/// Map a filesystem event path back to an issue slug, or `None` if the
/// path is irrelevant (outside the issues tree, a tempfile, an unknown
/// folder, an invalid slug shape).
pub fn issue_slug_from_event(issues_root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(issues_root).ok()?;
    // F17: filter atomic-write tempfiles by basename (final component
    // *of the path*, after strip_prefix). Filtering all components
    // before strip_prefix would over-filter when the repo path itself
    // contains `.issuectl-tmp-…` (e.g. `/tmp/.issuectl-tmp-test/`).
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.starts_with(".issuectl-tmp-") {
            return None;
        }
    }
    let mut comps = rel.components();
    let folder = comps.next()?.as_os_str().to_str()?;
    if folder != "open" && folder != "closed" {
        return None;
    }
    let slug = comps.next()?.as_os_str().to_str()?;
    if !slug::is_valid(slug) {
        return None;
    }
    Some(slug.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_resolver_strips_open_prefix() {
        let root = Path::new("/repo/issues");
        let p = Path::new("/repo/issues/open/quiet-brave-otter/item.md");
        assert_eq!(
            issue_slug_from_event(root, p).as_deref(),
            Some("quiet-brave-otter")
        );
    }

    #[test]
    fn slug_resolver_handles_closed() {
        let root = Path::new("/repo/issues");
        let p = Path::new("/repo/issues/closed/tiny-wild-comet/item.md");
        assert_eq!(
            issue_slug_from_event(root, p).as_deref(),
            Some("tiny-wild-comet")
        );
    }

    #[test]
    fn slug_resolver_rejects_unknown_folder() {
        let root = Path::new("/repo/issues");
        let p = Path::new("/repo/issues/draft/foo/item.md");
        assert!(issue_slug_from_event(root, p).is_none());
    }

    #[test]
    fn slug_resolver_rejects_invalid_slug() {
        let root = Path::new("/repo/issues");
        let p = Path::new("/repo/issues/open/INVALID_SLUG/item.md");
        assert!(issue_slug_from_event(root, p).is_none());
    }

    #[test]
    fn slug_resolver_filters_tempfiles() {
        let root = Path::new("/repo/issues");
        let p = Path::new("/repo/issues/open/quiet-brave-otter/.issuectl-tmp-abc.md");
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
        // F17: the tempfile filter must not over-match when the repo
        // root itself contains a `.issuectl-tmp-` component.
        let root = Path::new("/tmp/.issuectl-tmp-test/issues");
        let p = Path::new("/tmp/.issuectl-tmp-test/issues/open/quiet-brave-otter/item.md");
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
        std::fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        std::fs::create_dir_all(tmp.path().join("issues/closed")).unwrap();

        let hub = std::sync::Arc::new(EventHub::new());
        let mut rx = hub.tx_subscribe_for_test();

        let cfg = WatcherConfig {
            root: tmp.path().to_path_buf(),
            debounce: Duration::from_millis(50),
            bulk_threshold: 50,
        };
        let handle = spawn(hub.clone(), cfg);

        // First wait out the synthetic Resync{watcher_restart} the
        // watcher emits once it hooks the watch.
        let first = timeout(Duration::from_secs(2), rx.recv()).await;
        assert!(matches!(first, Ok(Ok(_))), "expected Resync, got {first:?}");

        // Now drop a real issue on disk and watch for the event.
        let issue_dir = tmp.path().join("issues/open/quiet-brave-otter");
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
}
