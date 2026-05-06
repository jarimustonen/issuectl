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
use crate::repo::{self, IssueSummary};
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

    for slug in affected {
        let root = cfg.root.clone();
        let parsed = tokio::task::spawn_blocking(move || parse_slug_state(&root, &slug))
            .await
            .unwrap_or_else(|_| ParseOutcome::Vanished {
                slug: String::new(),
            });
        match parsed {
            ParseOutcome::Loaded {
                slug,
                summary,
                version,
            } => {
                hub.publish(EventPayload::IssueUpserted {
                    slug,
                    version,
                    issue: summary,
                });
            }
            ParseOutcome::Invalid { slug, warnings } => {
                hub.publish(EventPayload::IssueInvalid { slug, warnings });
            }
            ParseOutcome::Vanished { slug } if !slug.is_empty() => {
                hub.publish(EventPayload::IssueRemoved { slug });
            }
            ParseOutcome::Vanished { .. } => {}
        }
    }
}

enum ParseOutcome {
    Loaded {
        slug: String,
        summary: Box<IssueSummary>,
        version: String,
    },
    Invalid {
        slug: String,
        warnings: Vec<crate::repo::LoadWarning>,
    },
    Vanished {
        slug: String,
    },
}

/// Re-derive the on-disk state of one slug. Runs in `spawn_blocking`.
fn parse_slug_state(root: &Path, slug: &str) -> ParseOutcome {
    if !slug::is_valid(slug) {
        return ParseOutcome::Vanished {
            slug: slug.to_string(),
        };
    }
    let issue = match repo::load_issue(root, slug) {
        Ok(i) => i,
        Err(_) => {
            return ParseOutcome::Vanished {
                slug: slug.to_string(),
            };
        }
    };

    // Read item.md raw to (a) detect parse-warning issues and (b) compute
    // a content hash that uniquely identifies this on-disk state for
    // client-side echo suppression once writes ship in M1.
    let item_path = root
        .join("issues")
        .join(&issue.folder)
        .join(&issue.slug)
        .join("item.md");
    let raw = match std::fs::read(&item_path) {
        Ok(b) => b,
        Err(_) => {
            return ParseOutcome::Vanished {
                slug: slug.to_string(),
            };
        }
    };

    // Re-run the warning-collecting parser; if anything is malformed
    // surface IssueInvalid rather than Upserted with default fields.
    let parsed = crate::parser::parse_item_md_with_warnings(&item_path, slug, &issue.folder);
    if !parsed.warnings.is_empty() {
        let warnings = parsed
            .warnings
            .into_iter()
            .map(|w| crate::repo::LoadWarning {
                slug: slug.to_string(),
                folder: issue.folder.clone(),
                message: w,
            })
            .collect();
        return ParseOutcome::Invalid {
            slug: slug.to_string(),
            warnings,
        };
    }

    let version = content_version(&raw);
    ParseOutcome::Loaded {
        slug: slug.to_string(),
        summary: Box::new(IssueSummary::from(issue)),
        version,
    }
}

/// Quick file-bytes SHA-256 used as the M0 version. M1 replaces this
/// with `mutate.rs::canonical_hash` so that no-op edits don't churn the
/// version. For now clients only use it as an opaque equality token.
fn content_version(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("sha256:{}", hex::encode(h.finalize()))
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
    // Filter our own atomic-write tempfiles (§3.3) and any nested temp.
    for c in path.components() {
        let name = c.as_os_str().to_string_lossy();
        if name.starts_with(".issuectl-tmp-") {
            return None;
        }
    }
    let rel = path.strip_prefix(issues_root).ok()?;
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
}
