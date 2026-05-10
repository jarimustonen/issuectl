//! Server-mode cache for `issues/.schema.yaml` and
//! `.issuectl/transitions.yaml`.
//!
//! In server mode the same process serves many PATCH/POST requests,
//! and re-parsing both YAMLs on every request is pure overhead. The
//! CLI parses them once per command, so the cost is only visible on
//! the server. This cache is therefore opt-in: callers activate it
//! via a thread-local guard around the request handler. When active,
//! `schema::load` and `transitions::load` consult the cache, comparing
//! the file's freshness key against the cached value. If the file
//! looks unchanged, the cached `Arc` is reused; otherwise the cache
//! re-parses, swaps, and returns the fresh value.
//!
//! Invalidation is best-effort, not strict coherency. The freshness
//! key is `(mtime, len)`: cheap to compute, catches any edit that
//! changes either value. It does not catch byte-for-byte replacements
//! that preserve both — `cp -p`, mtime-pinning restore tools, or
//! same-second in-place edits on filesystems with coarse timestamp
//! resolution. Config files are human-edited and these patterns are
//! rare; if a deployment hits one, restarting the server clears the
//! cache. Stat errors that aren't `NotFound` (permission denied, I/O
//! error) propagate up to the caller — silently caching them as
//! "missing" would let a transient error pin the server to the
//! built-in defaults.

use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context, Result};
use parking_lot::RwLock;

use crate::schema::{self, Schema};
use crate::transitions::{self, TransitionRules};

/// Read-side abstraction over "where do `schema` and `transitions`
/// come from for this call?". The two implementations are
/// [`UncachedConfig`] (re-parse on every call — what the CLI wants:
/// each command parses both YAMLs at most once) and
/// [`RepoConfigCache`] (re-parse only on freshness-stamp change —
/// what the long-running server wants).
///
/// The trait exists so future mutate-side / read-side APIs can grow
/// an explicit `&dyn ConfigSource` parameter and stop relying on
/// the thread-local [`enter`] / [`current`] activation. The
/// activation mechanism is documented at length below; its removal
/// is tracked under `@hugely-madly-haircut` follow-up work — the
/// type-level migration plan is to take `&dyn ConfigSource` on
/// every mutate entry point so the server's cache reaches the load
/// site through the type signature instead of through a
/// thread-local slot. Until that lands, the trait is callable
/// directly by any new code path that wants explicit injection
/// without participating in the thread-local dance.
pub trait ConfigSource: Send + Sync {
    /// Return a snapshot of the parsed schema for `root`. Implementations
    /// may cache (`RepoConfigCache`) or always re-parse (`UncachedConfig`).
    fn schema(&self, root: &Path) -> Result<Arc<Schema>>;
    /// Return a snapshot of the parsed transition rules for `root`.
    fn rules(&self, root: &Path) -> Result<Arc<TransitionRules>>;
}

/// Always-re-parse implementation of [`ConfigSource`]. The CLI's
/// natural fit: each command runs once, so parsing the YAMLs once
/// is fine and a cache would just be dead code.
///
/// Zero-sized — construct via `UncachedConfig` directly. Cheap to
/// pass by reference; safe to share.
pub struct UncachedConfig;

impl ConfigSource for UncachedConfig {
    fn schema(&self, root: &Path) -> Result<Arc<Schema>> {
        Ok(Arc::new(schema::load_uncached(root)?))
    }
    fn rules(&self, root: &Path) -> Result<Arc<TransitionRules>> {
        Ok(Arc::new(transitions::load_uncached(root)?))
    }
}

impl ConfigSource for RepoConfigCache {
    fn schema(&self, root: &Path) -> Result<Arc<Schema>> {
        debug_assert_eq!(
            root,
            self.root(),
            "RepoConfigCache::schema called with a root that disagrees with the cache's bound root",
        );
        RepoConfigCache::schema(self)
    }
    fn rules(&self, root: &Path) -> Result<Arc<TransitionRules>> {
        debug_assert_eq!(
            root,
            self.root(),
            "RepoConfigCache::rules called with a root that disagrees with the cache's bound root",
        );
        RepoConfigCache::rules(self)
    }
}

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

/// Read the freshness stamp for `path`. `NotFound` collapses to
/// `Missing` — the cache memoizes absence on purpose. Any other I/O
/// error (permission denied, stale NFS handle, mtime unsupported)
/// propagates so the caller fails loudly rather than serving a
/// fabricated default. Round-1 review identified silent error-to-
/// `Missing` collapse as a real correctness hole.
fn file_stamp(path: &Path) -> Result<FileStamp> {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let modified = meta
                .modified()
                .with_context(|| format!("cannot read mtime of {}", path.display()))?;
            Ok(FileStamp::Present {
                modified,
                len: meta.len(),
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(FileStamp::Missing),
        Err(e) => Err(anyhow::Error::new(e).context(format!("cannot stat {}", path.display()))),
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

    /// The repository root this cache is bound to. Exposed so
    /// `schema::load` / `transitions::load` can debug-assert their
    /// caller-supplied root agrees with the cache.
    pub fn root(&self) -> &Path {
        &self.root
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
        // Fast-path read using a stamp captured outside the lock. If
        // it agrees with the cached entry, return the cached `Arc`
        // without taking the writer.
        let stamp = file_stamp(&path)?;
        if let Some(hit) = self.inner.read().schema.as_ref() {
            if hit.stamp == stamp {
                return Ok(hit.value.clone());
            }
        }
        // Slow path: re-stat under the write lock so the cached
        // entry's stamp reflects the parsed bytes, not whatever the
        // file looked like before we queued. Round-1 review caught
        // that reusing the pre-lock stamp could record a stale
        // fingerprint when a concurrent writer raced ahead of us.
        let mut w = self.inner.write();
        let stamp_under_lock = file_stamp(&path)?;
        if let Some(hit) = w.schema.as_ref() {
            if hit.stamp == stamp_under_lock {
                return Ok(hit.value.clone());
            }
        }
        let parsed = Arc::new(schema::load_uncached(&self.root)?);
        #[cfg(test)]
        self.refresh_count.fetch_add(1, Ordering::Relaxed);
        w.schema = Some(Cached {
            value: parsed.clone(),
            stamp: stamp_under_lock,
        });
        Ok(parsed)
    }

    /// Same shape as `schema`, for `.issuectl/transitions.yaml`.
    pub fn rules(&self) -> Result<Arc<TransitionRules>> {
        let path = transitions::rules_path(&self.root);
        let stamp = file_stamp(&path)?;
        if let Some(hit) = self.inner.read().rules.as_ref() {
            if hit.stamp == stamp {
                return Ok(hit.value.clone());
            }
        }
        let mut w = self.inner.write();
        let stamp_under_lock = file_stamp(&path)?;
        if let Some(hit) = w.rules.as_ref() {
            if hit.stamp == stamp_under_lock {
                return Ok(hit.value.clone());
            }
        }
        let parsed = Arc::new(transitions::load_uncached(&self.root)?);
        #[cfg(test)]
        self.refresh_count.fetch_add(1, Ordering::Relaxed);
        w.rules = Some(Cached {
            value: parsed.clone(),
            stamp: stamp_under_lock,
        });
        Ok(parsed)
    }
}

thread_local! {
    static ACTIVE: std::cell::RefCell<Option<Arc<RepoConfigCache>>> =
        const { std::cell::RefCell::new(None) };
}

/// RAII guard for the thread-local `ACTIVE` cache slot. The guard is
/// `!Send` and `!Sync` by construction so it cannot cross an `.await`
/// in an `async fn`: the future containing it would be `!Send` and
/// fail to compile with `tokio::spawn`. That's deliberate — the
/// thread-local belongs to the worker thread that installed it, and
/// migrating across threads would either lose the cache or leak it
/// onto a different worker.
///
/// **Do not remove the `_not_send_sync` field.** `Rc<()>` is the
/// idiomatic Rust marker for "stays on its thread"; replacing it with
/// `()` would silently make the guard `Send` and reintroduce the leak
/// described above.
pub struct ActiveGuard {
    prev: Option<Arc<RepoConfigCache>>,
    _not_send_sync: std::marker::PhantomData<std::rc::Rc<()>>,
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
    use std::sync::atomic::AtomicBool;
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
    /// happens to land at the same mtime tick. The post-pin assertion
    /// guards against this test silently passing on a filesystem where
    /// `set_file_mtime` is a no-op (in which case the rewrite's own
    /// mtime change would invalidate the cache, masking the bug this
    /// test is meant to catch).
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

        // Rewrite content with a different length, then pin the mtime
        // back to the pre-rewrite value.
        fs::write(&path, "version: 1\nfields:\n  type:\n    required: true\n").unwrap();
        set_file_mtime(&path, FileTime::from_system_time(stamp_mtime)).unwrap();

        let pinned = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            pinned, stamp_mtime,
            "set_file_mtime did not pin the mtime; this test would otherwise pass for the wrong reason",
        );

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
    /// blocking tasks in succession. This test pins both halves of the
    /// claim:
    /// - the second task runs on the same OS thread as the first
    ///   (otherwise the test would pass for the wrong reason);
    /// - the slot is empty when that thread is reused, even though
    ///   the first task panicked before its scope ended.
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
            let first_tid = Arc::new(parking_lot::Mutex::new(None::<thread::ThreadId>));
            let first_tid_w = first_tid.clone();
            let c = cache.clone();
            let first = tokio::task::spawn_blocking(move || {
                *first_tid_w.lock() = Some(thread::current().id());
                let _g = enter(c);
                assert!(current().is_some());
                panic!("force unwind to exercise Drop on panic");
            })
            .await;
            assert!(first.is_err(), "first task must surface the panic");

            let (saw_leaked, second_tid) =
                tokio::task::spawn_blocking(|| (current().is_some(), thread::current().id()))
                    .await
                    .unwrap();
            assert_eq!(
                Some(second_tid),
                *first_tid.lock(),
                "second blocking task must run on the same OS thread as the first; otherwise this test \
                 proves nothing about Drop on a reused worker",
            );
            assert!(
                !saw_leaked,
                "a reused blocking worker must not see a leaked active cache",
            );
        });
    }

    /// `schema::load` falls back to the built-in default when the file
    /// is absent. The cache must memoize that absence — repeated calls
    /// against a still-missing file should not re-parse the embedded
    /// default.
    #[test]
    fn missing_file_is_memoized_and_invalidates_when_file_appears() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("issues")).unwrap();

        let cache = RepoConfigCache::new(root.to_path_buf());
        let _ = cache.schema().unwrap();
        assert_eq!(
            cache.refresh_count(),
            1,
            "absent file is one parse (default)"
        );
        let _ = cache.schema().unwrap();
        let _ = cache.schema().unwrap();
        assert_eq!(
            cache.refresh_count(),
            1,
            "subsequent requests with the file still missing must hit the cache",
        );

        // File appears: stamp transitions Missing → Present, refresh.
        fs::write(root.join("issues/.schema.yaml"), "version: 1\nfields: {}\n").unwrap();
        let _ = cache.schema().unwrap();
        assert_eq!(
            cache.refresh_count(),
            2,
            "file appearing must invalidate the memoized Missing entry",
        );
    }

    /// Inverse of the above: a file that disappears between requests
    /// must invalidate the Present entry, not silently keep serving
    /// the last-parsed copy.
    #[test]
    fn deleted_file_invalidates_present_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("issues")).unwrap();
        let path = root.join("issues/.schema.yaml");
        fs::write(&path, "version: 1\nfields: {}\n").unwrap();

        let cache = RepoConfigCache::new(root.to_path_buf());
        let _ = cache.schema().unwrap();
        assert_eq!(cache.refresh_count(), 1);

        fs::remove_file(&path).unwrap();
        let _ = cache.schema().unwrap();
        assert_eq!(
            cache.refresh_count(),
            2,
            "Present → Missing transition must trigger a refresh",
        );
    }

    /// A failed parse must not poison the cache. The next call after
    /// the YAML is fixed should succeed; it would not if the cache
    /// stored the stamp of the broken file alongside no value, or
    /// stamped the broken read as a successful one.
    #[test]
    fn parse_error_does_not_poison_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("issues")).unwrap();
        let path = root.join("issues/.schema.yaml");
        fs::write(&path, "this: is: not: valid: yaml\n").unwrap();

        let cache = RepoConfigCache::new(root.to_path_buf());
        assert!(
            cache.schema().is_err(),
            "broken YAML must surface as an error, not a cached default",
        );
        // refresh_count is incremented after a successful parse only,
        // so a failing call leaves it at zero.
        assert_eq!(cache.refresh_count(), 0);

        // Repair the file; the cache should now succeed.
        fs::write(&path, "version: 1\nfields: {}\n").unwrap();
        let _ = cache.schema().unwrap();
        assert_eq!(
            cache.refresh_count(),
            1,
            "after the YAML is fixed, the next call must parse cleanly",
        );
    }

    /// Concurrent readers racing on a cold cache must not all parse.
    /// The double-checked locking pattern guarantees exactly one
    /// successful parse no matter how many threads queue on the write
    /// lock, because the second check inside the lock sees the
    /// freshly-cached entry.
    #[test]
    fn concurrent_readers_share_one_parse() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("issues")).unwrap();
        fs::write(root.join("issues/.schema.yaml"), "version: 1\nfields: {}\n").unwrap();

        let cache = Arc::new(RepoConfigCache::new(root.to_path_buf()));
        // Barrier synchronises N threads to all call `schema()` as
        // close together as possible.
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let stop = Arc::new(AtomicBool::new(false));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let c = cache.clone();
                let b = barrier.clone();
                let s = stop.clone();
                thread::spawn(move || {
                    b.wait();
                    if !s.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = c.schema().unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);

        assert_eq!(
            cache.refresh_count(),
            1,
            "8 concurrent first-callers must share one parse via DCL",
        );
    }

    fn bump_mtime(path: &Path) {
        let prev = std::fs::metadata(path).unwrap().modified().unwrap();
        let next = FileTime::from_system_time(prev + std::time::Duration::from_secs(10));
        set_file_mtime(path, next).unwrap();
    }

    #[test]
    fn uncached_config_parses_schema_and_rules_on_every_call() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("issues")).unwrap();
        let cfg = UncachedConfig;

        // Two consecutive calls return distinct `Arc`s — UncachedConfig
        // does not memoise. Same root, but a fresh parse each time.
        let a = cfg.schema(root).unwrap();
        let b = cfg.schema(root).unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        let r1 = cfg.rules(root).unwrap();
        let r2 = cfg.rules(root).unwrap();
        assert!(!Arc::ptr_eq(&r1, &r2));
    }

    #[test]
    fn cache_impl_of_config_source_reuses_arc_when_files_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("issues")).unwrap();
        fs::create_dir_all(root.join(".issuectl")).unwrap();
        fs::write(root.join("issues/.schema.yaml"), "version: 1\nfields: {}\n").unwrap();
        fs::write(
            root.join(".issuectl/transitions.yaml"),
            "version: 1\nstatus_rules: {}\n",
        )
        .unwrap();

        let cache: Arc<dyn ConfigSource> =
            Arc::new(RepoConfigCache::new(root.to_path_buf()));
        let a = cache.schema(root).unwrap();
        let b = cache.schema(root).unwrap();
        // Same Arc reused → cache is doing its job through the trait.
        assert!(Arc::ptr_eq(&a, &b));
        let r1 = cache.rules(root).unwrap();
        let r2 = cache.rules(root).unwrap();
        assert!(Arc::ptr_eq(&r1, &r2));
    }
}
