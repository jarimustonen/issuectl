//! Server-mode cache for `issues/.schema.yaml` and
//! `.issuectl/transitions.yaml`.
//!
//! Server mode parses both YAMLs on every PATCH/POST today. The CLI
//! parses them once per command, so the cost is only visible when the
//! same process serves many requests. This cache is therefore opt-in:
//! activated via a thread-local guard around the mutate call site.
//! When active, `schema::load` and `transitions::load` consult the
//! cache, comparing the file's mtime against the cached value. If the
//! file has not advanced, the cached parse is reused; otherwise the
//! cache re-parses, swaps, and returns the fresh value.
//!
//! Invalidation is mtime-on-request. We don't `notify`-watch the
//! files — for v1 the stat-then-maybe-read flow is enough, and it
//! keeps the cache trivially correct under external edits.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Result;
use parking_lot::RwLock;

use crate::schema::{self, Schema};
use crate::transitions::{self, TransitionRules};

/// Cached parse + the mtime it was parsed from. `mtime: None` means
/// the file did not exist at last stat — so a "still missing" stat
/// avoids re-reading the default. Stored as `Arc<T>` so cache hits
/// hand out cheap clones rather than re-cloning the parsed struct.
#[derive(Clone)]
struct Cached<T> {
    value: Arc<T>,
    mtime: Option<SystemTime>,
}

#[derive(Default)]
struct Inner {
    schema: Option<Cached<Schema>>,
    rules: Option<Cached<TransitionRules>>,
}

/// Per-server-process cache. Cheap to clone (Arc internally) but
/// usually held inside `AppState` as `Arc<RepoConfigCache>`.
pub struct RepoConfigCache {
    inner: RwLock<Inner>,
    /// Counts each call that actually re-parsed (i.e. cache miss or
    /// stale entry). Exposed for tests that assert "two PATCHes
    /// share one parse".
    parse_count: AtomicUsize,
}

impl Default for RepoConfigCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RepoConfigCache {
    pub fn new() -> Self {
        RepoConfigCache {
            inner: RwLock::new(Inner::default()),
            parse_count: AtomicUsize::new(0),
        }
    }

    /// How many full re-parses (schema + rules combined) the cache
    /// has performed since construction. Test-facing.
    pub fn parse_count(&self) -> usize {
        self.parse_count.load(Ordering::Relaxed)
    }

    /// Return the cached `Schema` for `root`, re-parsing if the file's
    /// mtime advanced since the last hit. The returned `Arc` is a
    /// snapshot — concurrent invalidations swap in a fresh `Arc` for
    /// later callers without disturbing this one.
    pub fn schema(&self, root: &Path) -> Result<Arc<Schema>> {
        let path = schema::schema_path(root);
        let mtime = file_mtime(&path);
        if let Some(hit) = self.inner.read().schema.as_ref() {
            if hit.mtime == mtime {
                return Ok(hit.value.clone());
            }
        }
        let mut w = self.inner.write();
        if let Some(hit) = w.schema.as_ref() {
            if hit.mtime == mtime {
                return Ok(hit.value.clone());
            }
        }
        let parsed = Arc::new(schema::load_uncached(root)?);
        self.parse_count.fetch_add(1, Ordering::Relaxed);
        w.schema = Some(Cached {
            value: parsed.clone(),
            mtime,
        });
        Ok(parsed)
    }

    /// Same shape as `schema`, for `.issuectl/transitions.yaml`.
    pub fn rules(&self, root: &Path) -> Result<Arc<TransitionRules>> {
        let path = transitions::rules_path(root);
        let mtime = file_mtime(&path);
        if let Some(hit) = self.inner.read().rules.as_ref() {
            if hit.mtime == mtime {
                return Ok(hit.value.clone());
            }
        }
        let mut w = self.inner.write();
        if let Some(hit) = w.rules.as_ref() {
            if hit.mtime == mtime {
                return Ok(hit.value.clone());
            }
        }
        let parsed = Arc::new(transitions::load_uncached(root)?);
        self.parse_count.fetch_add(1, Ordering::Relaxed);
        w.rules = Some(Cached {
            value: parsed.clone(),
            mtime,
        });
        Ok(parsed)
    }
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

thread_local! {
    static ACTIVE: std::cell::RefCell<Option<Arc<RepoConfigCache>>> =
        const { std::cell::RefCell::new(None) };
}

/// RAII guard for the thread-local `ACTIVE` cache slot. Activating
/// the cache via `enter` returns a guard that clears the slot on
/// drop, even on panic — important because `tokio::task::spawn_blocking`
/// reuses worker threads.
pub struct ActiveGuard {
    prev: Option<Arc<RepoConfigCache>>,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let prev = self.prev.take();
        ACTIVE.with(|slot| *slot.borrow_mut() = prev);
    }
}

/// Install `cache` as the active cache for the current thread. Drops
/// to the previous value when the returned guard goes out of scope.
pub fn enter(cache: Arc<RepoConfigCache>) -> ActiveGuard {
    let prev = ACTIVE.with(|slot| slot.borrow_mut().replace(cache));
    ActiveGuard { prev }
}

/// Active cache for the current thread, if any. `schema::load` and
/// `transitions::load` consult this so that callers (CLI vs. server)
/// don't need different signatures — the server flips the slot on
/// before delegating to `mutate::*`, the CLI never does.
pub fn current() -> Option<Arc<RepoConfigCache>> {
    ACTIVE.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::thread;

    /// Two PATCHes in a row trigger one parse pair, not two; touching
    /// `.schema.yaml` invalidates and forces the next request to
    /// re-parse. This is the documented success criterion for the
    /// server cache.
    #[test]
    fn cache_reuses_until_mtime_advances() {
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

        let cache = Arc::new(RepoConfigCache::new());

        // First request: cold miss for both files.
        let _ = cache.schema(root).unwrap();
        let _ = cache.rules(root).unwrap();
        assert_eq!(
            cache.parse_count(),
            2,
            "first request should parse schema + rules once each"
        );

        // Second request: hit on both. mtime unchanged.
        let _ = cache.schema(root).unwrap();
        let _ = cache.rules(root).unwrap();
        assert_eq!(
            cache.parse_count(),
            2,
            "consecutive requests with unchanged mtimes must reuse the cache",
        );

        // Touch the schema file with a later mtime, then a request
        // re-parses schema (rules stay cached). Sleep long enough to
        // outrun the host fs's mtime resolution — APFS is sub-ms but
        // tmpfs/CI-FS is sometimes 1 s.
        bump_mtime(&root.join("issues/.schema.yaml"));
        let _ = cache.schema(root).unwrap();
        let _ = cache.rules(root).unwrap();
        assert_eq!(
            cache.parse_count(),
            3,
            "touching schema must invalidate the schema entry only",
        );

        bump_mtime(&root.join(".issuectl/transitions.yaml"));
        let _ = cache.schema(root).unwrap();
        let _ = cache.rules(root).unwrap();
        assert_eq!(
            cache.parse_count(),
            4,
            "touching transitions must invalidate the rules entry only",
        );
    }

    /// `enter` activates the thread-local cache so `schema::load` and
    /// `transitions::load` route through it, and the guard clears the
    /// slot on drop — including across worker-thread reuse.
    #[test]
    fn enter_guard_routes_load_through_cache_and_clears_on_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        fs::create_dir_all(root.join("issues")).unwrap();

        let cache = Arc::new(RepoConfigCache::new());

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
            cache.parse_count(),
            2,
            "two schema + two transitions loads should yield one parse each",
        );
    }

    /// The thread-local must be per-thread: spawning a worker without
    /// `enter` must see no active cache, even while the parent has one
    /// installed. This protects the CLI assumption "no cache unless
    /// the server installs one explicitly".
    #[test]
    fn active_slot_is_per_thread() {
        let cache = Arc::new(RepoConfigCache::new());
        let _g = enter(cache);
        let saw_active = thread::spawn(|| current().is_some()).join().unwrap();
        assert!(!saw_active);
    }

    fn bump_mtime(path: &Path) {
        // `set_modified` would be cleaner but is unstable on stable
        // Rust; rewriting the file with a small sleep advances mtime
        // reliably across filesystems.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let prev = fs::read_to_string(path).unwrap();
        fs::write(path, prev).unwrap();
    }
}
