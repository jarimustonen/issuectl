---
created: 2026-08-12
updated: 2026-08-14
type: improvement
status: wontfix
priority: normal
related: ['@pi-manifest-locking']
closed: 2026-08-14
closed_by: jari
---

# pi-corpus: save_pi_manifest lacks fsync durability

_Source: crates/issuectl-core/src/skill.rs_

## Description

Spin-off from /llm-review of pi-manifest-locking (DeepSeek #5, OpenAI).

`save_pi_manifest` writes a temp file and renames, but fsyncs neither the temp file nor the parent directory. The atomic rename guarantees no TORN file, but not durability: after a power loss the rename (or the new bytes) can be lost despite a success return, and a later `pi_prune` may then act on stale ownership data. For a global provenance manifest that governs deletions this is a real durability gap.

Fix: mirror `write_item_atomic` — `sync_all()` the temp before rename, fsync the parent dir after (Unix). Pre-existing (atomic rename landed in the lifecycle issue); out of scope for the locking change.

## Resolution

### 2026-08-14T03:41:55Z · @jari

Wontfix: power-loss durability hardening; improbable, and the atomic rename already prevents torn files. Review-cascade polish, not worth it now.
