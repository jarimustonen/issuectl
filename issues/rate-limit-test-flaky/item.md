---
created: 2026-08-12
updated: 2026-08-12
type: bug
status: fixed
priority: high
labels: [ci]
commits:
- hash: '2924529'
  summary: make rate-limit test deterministic via injectable clock seam
closed: 2026-08-12
---

# CI: put_body_rate_limit_fires_with_retry_after flaky (expected 429 after burst)

## Description

`main` CI (run 31509473175, commit 75670fa) is red on a single test; 1080 passed, 1 failed.

## Failure
```
test server::tests::put_body_rate_limit_fires_with_retry_after ... FAILED
thread '...' panicked at crates/issuectl-core/src/server/mod.rs:1802:9:
assertion `left == right` failed: expected 429 after burst
  left: 200  right: 429
```

## Reading
The test fires a burst of PUT-body requests expecting the rate limiter to return `429 Too Many Requests` with a `Retry-After`, but the last request still returned `200`. This is almost certainly a **timing/ordering flake**: the limiter's window or token accounting depends on wall-clock or request timing that does not hold under CI load, so the burst does not deterministically cross the threshold.

## Fix direction
Make the test deterministic rather than time-dependent: drive the limiter with an injected/mock clock or a fixed token budget, and assert on the exact request index that must be rejected, instead of relying on real-time burst timing. If the limiter itself is timing-correct, tighten the test's burst count so the threshold is unambiguously exceeded.
