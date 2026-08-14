---
created: 2026-08-12
updated: 2026-08-14
type: improvement
status: wontfix
priority: low
related: ['@pi-manifest-locking']
closed: 2026-08-14
closed_by: jari
---

# pi-corpus: pi_status reads lock-free and can report a torn snapshot

_Source: crates/issuectl-core/src/skill.rs_

## Description

Spin-off from /llm-review of pi-manifest-locking (consensus, low severity).

`pi_status` loads the manifest and reads skill dirs without holding the corpus lock, so against a concurrent `install`/`pi_prune` it can combine an old manifest with new files (or vice versa) and transiently report Unmanaged/Missing/Stale/Modified incorrectly. Harmless for an advisory readout, but misleading if scripted.

Fix (optional): take a SHARED flock (`FileExt::lock_shared`) for the duration of the status scan — blocks against a writer (prune/install) but not against other readers — or explicitly document that pi-status is a non-snapshot diagnostic. Requires a shared-lock affordance; deliberately left out of the write-path locking change.

## Resolution

### 2026-08-14T03:41:55Z · @jari

Wontfix: shared-lock for a non-snapshot advisory readout; the issue itself offers 'or just document it's non-snapshot'. Low value.
