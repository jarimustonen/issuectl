---
created: 2026-08-12
updated: 2026-08-14
type: bug
status: fixed
priority: normal
labels: [test]
closed: 2026-08-14
---

# flock write-lock test is flaky under the parallel suite

## Description

## Comments

### 2026-08-12T17:05:51Z · @jari

Introduced by flock-write-test-coverage (Wave 2, this stint). `mutate::tests::write_lock_released_after_failed_mutation` (crates/issuectl-core/src/mutate/mod.rs:6739) passes 5/5 in isolation and most full-suite runs, but INTERMITTENTLY fails when the whole ~1011-test lib suite runs in parallel (timing/FS-lock contention with other tests). Product code is fine — this is a nondeterministic TEST. Fix like rate-limit-test-flaky: make it deterministic (inject the lock/clock, or serialize it / use a unique tempdir + explicit acquire-order assertion instead of racing). Must be green before the next release cut.

### 2026-08-14T06:14:53Z · @jari

Fixed in 118a1ae. Root cause: transient EWOULDBLOCK from macOS/BSD flock(LOCK_EX|LOCK_NB) under concurrent test-suite load — write_lock_is_free's one-shot non-blocking probe read that as 'lock held'. Confirmed empirically: failing inode matched the test's own single acquire (no cross-test contention) and an immediate fresh-fd retry succeeded (spurious probe, not a real leak). Both write_lock_released_after_{failed,successful}_mutation were affected. Fix: bounded fresh-fd try_lock retry loop (2s deadline, panic on non-WouldBlock, file-existence assertion preserved). Verified 0/80 failures at --test-threads 32 (was 7/40); full workspace green; clippy/fmt clean. Reviewed via /llm-review (4 models) + /assess-findings; bounded-retry is their consensus over a threaded blocking acquire.

