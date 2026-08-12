---
created: 2026-08-12
updated: 2026-08-12
type: bug
status: open
priority: normal
labels: [test]
---

# flock write-lock test is flaky under the parallel suite

## Description

## Comments

### 2026-08-12T17:05:51Z · @jari

Introduced by flock-write-test-coverage (Wave 2, this stint). `mutate::tests::write_lock_released_after_failed_mutation` (crates/issuectl-core/src/mutate/mod.rs:6739) passes 5/5 in isolation and most full-suite runs, but INTERMITTENTLY fails when the whole ~1011-test lib suite runs in parallel (timing/FS-lock contention with other tests). Product code is fine — this is a nondeterministic TEST. Fix like rate-limit-test-flaky: make it deterministic (inject the lock/clock, or serialize it / use a unique tempdir + explicit acquire-order assertion instead of racing). Must be green before the next release cut.
