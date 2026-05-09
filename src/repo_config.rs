//! Server-mode cache for `issues/.schema.yaml` and
//! `.issuectl/transitions.yaml`.
//!
//! Server mode parses both YAMLs on every PATCH/POST today. The CLI
//! parses them once per command, so the cost is only visible when the
//! same process serves many requests. This cache is therefore opt-in:
//! activated via a thread-local guard around the mutate call site.
//! When active, `schema::load` and `transitions::load` consult the
//! cache, comparing the file's freshness key (mtime + length) against
//! the cached value. If the file looks unchanged, the cached `Arc` is
//! reused; otherwise the cache re-parses, swaps, and returns the
//! fresh value.
//!
//! Invalidation is best-effort, not strict coherency. We compare
//! `(mtime, len)` per request — that catches ordinary edits but cannot
//! see same-mtime same-length replacements (e.g. `cp -p`, atomic
//! restore tools that preserve timestamps). For v1 that trade-off is
//! intentional: config files are human-edited, replacements are rare,
//! and the cost of being wrong is one stale parse until the next real
//! edit. If correctness matters more than throughput, callers should
//! restart the server after such replacements.

use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Result;
use parking_lot::RwLock;

use crate::schema::{self, Schema};
use crate::transitions::{self, TransitionRules};

/// Freshness fingerprint for a config file. `(mtime, len)` is cheap to
/// compute and catches any edit that changes either value — `mtime`
/// alone misses same-second rewrites on coarse-resolution filesystems,
/// `len` alone misses byte-for-byte content changes. `Missing` lets the
/// cache memoize "file does not exist" so a still-missing file does
/// not re-call `default_schema()` on every request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileStamp {
    Missing,
    Present { modified: SystemTime, len: u64 },
}

fn file_stamp(path: &Path) -> FileStamp {
    match std::fs::metadata(path) {
        Ok(meta) => match meta.modified() {
            Ok(modified) => FileStamp::Present {
                modified,
                len: meta.len(),
            },
            Err(_) => FileStamp::Missing,
        },
        Err(_) => FileStamp::Missing,
    }
}

struct Cached<T> {
    value: Arc<T>,
    stamp: FileStamp,
}

#[derive(Default)]
struct Inner {
    schema: Option<Cached<Schema>>,
    rules: Option<Cached<TransitionRules>>,
}

/// Per-server-process cache, bound to a single repository root. A new
/// cache is constructed alongside each `AppState`; the cache cannot be
/// reused across roots because the bound `root` is what the cache
/// stats and parses against.
pub struct RepoConfigCache {
    root: PathBuf,
    inner: RwLock<Inner>,
    /// Counts each call that actually re-parsed (cache miss or stale
    /// entry). Only compiled in test builds — production has no need
    /// to expose internal cache instrumentation.
    #[cfg(test)]
    refresh_count: AtomicUsize,
}

impl RepoConfigCache {
    pub fn new(root: PathBuf) -> Self {
        RepoConfigCache {
            root,
            inner: RwLock::new(Inner::default()),
            #[cfg(test)]
            refresh_count: AtomicUsize::new(0),
        }
    }

    /// Test-only: number of individual cache refreshes (schema or
    /// rules) since construction. Each schema or rules miss adds one;
    /// successive hits add zero.
    #[cfg(test)]
    pub fn refresh_count(&self) -> usize {
        self.refresh_count.load(Ordering::Relaxed)
    }

    /// Return the cached `Schema`, re-parsing if the file's freshness
    /// stamp changed since the last hit. The returned `Arc` is a
    /// snapshot — concurrent invalidations swap in a fresh `Arc` for
    /// later callers without disturbing this one.
    pub fn schema(&self) -> Result<Arc<Schema>> {
        let path = schema::schema_path(&self.root);
        let stamp = file_stamp(&path);
        if let Some(hit) = self.inner.read().schema.as_ref() {
            if hit.stamp == stamp {
                return Ok(hit.value.clone());
            }
        }
        let mut w = self.inner.write();
        if let Some(hit) = w.schema.as_ref() {
            if hit.stamp == stamp {
                return Ok(hit.value.clone());
            }
        }
        let parsed = Arc::new(schema::load_uncached(&self.root)?);
        #[cfg(test)]
        self.refresh_count.fetch_add(1, Ordering::Relaxed);
        w.schema = Some(Cached {
            value: parsed.clone(),
            stamp,
        });
        Ok(parsed)
    }

    /// Same shape as `schema`, for `.issuectl/transitions.yaml`.
    pub fn rules(&self) -> Result<Arc<TransitionRules>> {
        let path = transitions::rules_path(&self.root);
        let stamp = file_stamp(&path);
        if let Some(hit) = self.inner.read().rules.as_ref() {
            if hit.stamp == stamp {
                return Ok(hit.value.clone());
            }
        }
        let mut w = self.inner.write();
        if let Some(hit) = w.rules.as_ref() {
            if hit.stamp == stamp {
                return Ok(hit.value.clone());
            }
        }
        let parsed = Arc::new(transitions::load_uncached(&self.root)?);
        #[cfg(test)]
        self.refresh_count.fetch_add(1, Ordering::Relaxed);
        w.rules = Some(Cached {
            value: parsed.clone(),
            stamp,
        });
        Ok(parsed)
    }
}

thread_local! {
    static ACTIVE: std::cell::RefCell<Option<Arc<RepoConfigCache>>> =
        const { std::cell::RefCell::new(None) };
}

/// RAII guard for the thread-local `ACTIVE` cache slot. The guard is
/// `!Send` and `!Sync` by construction (via the `PhantomData<*const ()>`
/// field): holding it across `.await` would cause a compile error.
/// That's deliberate — `tokio::task::spawn_blocking` reuses worker
/// threads, so a guard that survived an async boundary on one thread
/// could leak the cache to another task scheduled on the same worker.
pub struct ActiveGuard {
    prev: Option<Arc<RepoConfigCache>>,
    _not_send_sync: std::marker::PhantomData<*const ()>,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let prev = self.prev.take();
        ACTIVE.with(|slot| *slot.borrow_mut() = prev);
    }
}

/// Install `cache` as the active cache for the current thread. The
/// returned guard restores the previous slot on drop — including
/// across panics, since `Drop` runs during unwind.
pub fn enter(cache: Arc<RepoConfigCache>) -> ActiveGuard {
    let prev = ACTIVE.with(|slot| slot.borrow_mut().replace(cache));
    ActiveGuard {
        prev,
        _not_send_sync: std::marker::PhantomData,
    }
}

/// Active cache for the current thread, if any. `schema::load` and
/// `transitions::load` consult this so that callers (CLI vs. server)
/// don't need different signatures — the server installs a guard
/// before delegating to `mutate::*`, the CLI never does.
pub fn current() -> Option<Arc<RepoConfigCache>> {
    ACTIVE.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::{set_file_mtime, FileTime};
    use std::fs;
    use std::path::Path;
    use std::thread;

    /// Two PATCHes in a row trigger one parse pair, not two; touching
    /// `.schema.yaml` invalidates and forces the next request to
    /// re-parse. This is the documented success criterion for the
    /// server cache.
    #[test]
    fn cache_reuses_until_freshness_stamp_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("issues")).unwrap();
        fs::create_dir_all(root.join(".issuectl")).unwrap();

        // Both files present so we exercise the file-exists path.
        fs::write(root.join("issues/.schema.yaml"), "version: 1\nfields: {}\n").unwrap();
        fs::write(
            root.join(".issuectl/transitions.yaml"),
            "version: 1\nstatus_rules: {}\n",
        )
        .unwrap();

        let cache = RepoConfigCache::new(root.to_path_buf());

        // First request: cold miss for both files.
        let _ = cache.schema().unwrap();
        let _ = cache.rules().unwrap();
        assert_eq!(
            cache.refresh_count(),
            2,
            "first request should parse schema + rules once each"
        );

        // Second request: hit on both. Stamps unchanged.
        let _ = cache.schema().unwrap();
        let _ = cache.rules().unwrap();
        assert_eq!(
            cache.refresh_count(),
            2,
            "consecutive requests with unchanged files must reuse the cache",
        );

        // Bump the schema's mtime explicitly. `filetime` sidesteps the
        // host filesystem's mtime resolution and avoids the 1s+ sleeps
        // that would otherwise be needed for the test to be reliable.
        bump_mtime(&root.join("issues/.schema.yaml"));
        let _ = cache.schema().unwrap();
        let _ = cache.rules().unwrap();
        assert_eq!(
            cache.refresh_count(),
            3,
            "touching schema must invalidate the schema entry only",
        );

        bump_mtime(&root.join(".issuectl/transitions.yaml"));
        let _ = cache.schema().unwrap();
        let _ = cache.rules().unwrap();
        assert_eq!(
            cache.refresh_count(),
            4,
            "touching transitions must invalidate the rules entry only",
        );
    }

    /// Length-only invalidation works even when an external rewrite
    /// happens to land at the same mtime tick. Belt-and-suspenders for
    /// the `(mtime, len)` key.
    #[test]
    fn same_mtime_different_length_invalidates() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("issues")).unwrap();
        let path = root.join("issues/.schema.yaml");
        fs::write(&path, "version: 1\nfields: {}\n").unwrap();

        let cache = RepoConfigCache::new(root.to_path_buf());
        let _ = cache.schema().unwrap();
        let stamp_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(cache.refresh_count(), 1);

        // Rewrite content with different length but pin the original
        // mtime back. The cache must still see the change via `len`.
        fs::write(&path, "version: 1\nfields:\n  type:\n    required: true\n").unwrap();
        set_file_mtime(&path, FileTime::from_system_time(stamp_mtime)).unwrap();

        let _ = cache.schema().unwrap();
        assert_eq!(
            cache.refresh_count(),
            2,
            "length change must invalidate even when mtime is pinned",
        );
    }

    /// `enter` activates the thread-local cache so `schema::load` and
    /// `transitions::load` route through it, and the guard clears the
    /// slot on drop.
    #[test]
    fn enter_guard_routes_load_through_cache_and_clears_on_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        fs::create_dir_all(root.join("issues")).unwrap();

        let cache = Arc::new(RepoConfigCache::new(root.clone()));

        {
            let _g = enter(cache.clone());
            // Default-on-missing path still warms the cache.
            crate::schema::load(&root).unwrap();
            crate::schema::load(&root).unwrap();
            crate::transitions::load(&root).unwrap();
            crate::transitions::load(&root).unwrap();
        }
        assert!(
            current().is_none(),
            "active cache must be cleared when the guard drops",
        );
        assert_eq!(
            cache.refresh_count(),
            2,
            "two schema + two transitions loads should yield one parse each",
        );
    }

    /// `std::thread::spawn` proves OS-level thread-local isolation,
    /// which is guaranteed by the platform — keep it as a smoke test
    /// against `static` accidentally being introduced in place of
    /// `thread_local!`.
    #[test]
    fn active_slot_is_per_thread() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Arc::new(RepoConfigCache::new(tmp.path().to_path_buf()));
        let _g = enter(cache);
        let saw_active = thread::spawn(|| current().is_some()).join().unwrap();
        assert!(!saw_active);
    }

    /// The real risk for the thread-local design is `tokio::task::
    /// spawn_blocking` worker reuse: the same OS thread serves multiple
    /// blocking tasks in succession. Verify that even when the first
    /// task panics mid-handler, the guard's `Drop` clears the slot
    /// before the next task on the reused worker observes it.
    #[test]
    fn guard_clears_after_panic_on_reused_blocking_worker() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let cache = Arc::new(RepoConfigCache::new(root));

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();

        rt.block_on(async {
            let c = cache.clone();
            let first = tokio::task::spawn_blocking(move || {
                let _g = enter(c);
                assert!(current().is_some());
                panic!("force unwind to exercise Drop on panic");
            })
            .await;
            assert!(first.is_err(), "first task must surface the panic");

            let saw_leaked = tokio::task::spawn_blocking(|| current().is_some())
                .await
                .unwrap();
            assert!(
                !saw_leaked,
                "a reused blocking worker must not see a leaked active cache",
            );
        });
    }

    fn bump_mtime(path: &Path) {
        let prev = std::fs::metadata(path).unwrap().modified().unwrap();
        let next = FileTime::from_system_time(prev + std::time::Duration::from_secs(10));
        set_file_mtime(path, next).unwrap();
    }
}
