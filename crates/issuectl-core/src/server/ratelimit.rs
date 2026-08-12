//! Token-bucket rate limiter for write-side body endpoints.
//!
//! Per design §6.6: `PUT /body` and `POST /preview` share a per-slug
//! bucket at 4 req/sec, burst 10. The design names a `(slug, session)`
//! key but the M0/M1 server scrapped per-client session cookies (the
//! loopback threat model treats every same-machine caller as the same
//! identity); we degrade to per-slug for `PUT /body` and a single
//! global bucket for `POST /preview` (no slug in the URL). That still
//! prevents one runaway tab from saturating the markdown sanitizer
//! while leaving every other slug responsive.
//!
//! Stale buckets are pruned on every acquire whose target bucket is
//! itself idle, keeping the map size bounded under churn.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// Monotonic time source for the limiter. Production reads the OS
/// clock; tests inject a frozen clock so token refill is deterministic
/// regardless of how long each request takes under load (the source of
/// the `put_body_rate_limit_fires_with_retry_after` CI flake).
pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now(&self) -> Instant;
}

/// Real monotonic clock used by `serve()`.
#[derive(Debug)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Clock frozen at construction: every `now()` returns the same
/// instant, so `check` observes zero elapsed time on each call and no
/// tokens refill mid-burst. With capacity `C`, the first `C` calls are
/// allowed and call `C + 1` rejects — deterministically, no matter how
/// slow the surrounding request path is.
#[cfg(test)]
#[derive(Debug)]
pub struct FrozenClock(Instant);

#[cfg(test)]
impl FrozenClock {
    pub fn new() -> Self {
        FrozenClock(Instant::now())
    }
}

#[cfg(test)]
impl Clock for FrozenClock {
    fn now(&self) -> Instant {
        self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Decision {
    pub allowed: bool,
    /// Seconds the caller should wait before retrying. Always `Some`
    /// when `allowed == false`, ceil-rounded to the next whole second
    /// so the `Retry-After` header is HTTP-compliant.
    pub retry_after_secs: u64,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

pub struct TokenBucketLimiter {
    inner: Mutex<HashMap<String, Bucket>>,
    capacity: f64,
    refill_per_sec: f64,
    /// Buckets idle this long are pruned on access. 5 minutes is far
    /// longer than any realistic burst pattern but short enough that
    /// `serve` doesn't accumulate buckets indefinitely.
    idle_ttl: Duration,
    clock: Arc<dyn Clock>,
}

impl TokenBucketLimiter {
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        Self::with_clock(capacity, refill_per_sec, Arc::new(SystemClock))
    }

    /// Construct with an explicit clock. Production calls `new` (real
    /// clock); tests pass a `FrozenClock` to make refill deterministic.
    pub fn with_clock(capacity: f64, refill_per_sec: f64, clock: Arc<dyn Clock>) -> Self {
        assert!(capacity > 0.0);
        assert!(refill_per_sec > 0.0);
        TokenBucketLimiter {
            inner: Mutex::new(HashMap::new()),
            capacity,
            refill_per_sec,
            idle_ttl: Duration::from_secs(300),
            clock,
        }
    }

    /// Try to consume one token from the bucket identified by `key`.
    /// Returns `allowed=true` on success and `allowed=false` with a
    /// `Retry-After` hint otherwise.
    pub fn check(&self, key: &str) -> Decision {
        let now = self.clock.now();
        let mut g = self.inner.lock();
        // Cheap O(n) prune. With at most a few hundred slugs in a real
        // repo this stays well under a millisecond per call.
        g.retain(|_, b| now.duration_since(b.last) < self.idle_ttl);

        let bucket = g.entry(key.to_string()).or_insert(Bucket {
            tokens: self.capacity,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Decision {
                allowed: true,
                retry_after_secs: 0,
            }
        } else {
            let needed = 1.0 - bucket.tokens;
            let wait = needed / self.refill_per_sec;
            // ceil to whole seconds so Retry-After is integer-valued.
            let retry = wait.ceil() as u64;
            Decision {
                allowed: false,
                retry_after_secs: retry.max(1),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn burst_allows_capacity_then_blocks() {
        let lim = TokenBucketLimiter::new(3.0, 4.0);
        for _ in 0..3 {
            assert!(lim.check("k").allowed);
        }
        let d = lim.check("k");
        assert!(!d.allowed);
        assert!(d.retry_after_secs >= 1);
    }

    #[test]
    fn separate_keys_have_independent_buckets() {
        let lim = TokenBucketLimiter::new(1.0, 0.1);
        assert!(lim.check("a").allowed);
        assert!(lim.check("b").allowed);
        assert!(!lim.check("a").allowed);
        assert!(!lim.check("b").allowed);
    }

    #[test]
    fn refill_after_wait() {
        let lim = TokenBucketLimiter::new(1.0, 50.0);
        assert!(lim.check("k").allowed);
        assert!(!lim.check("k").allowed);
        sleep(Duration::from_millis(40));
        assert!(lim.check("k").allowed);
    }
}
