---
created: 2026-08-12
updated: 2026-08-12
type: improvement
status: open
priority: low
related: ['@pi-corpus-metadata-error-misclass']
---

# pi_prune should report owned Inaccessible entries instead of a silent no-op

_Source: crates/issuectl-core/src/skill.rs_

## Description

Spin-off from /llm-review of pi-corpus-metadata-error-misclass (DeepSeek).

After the metadata-error fix, an issuectl-OWNED entry whose SKILL.md can't be stat'd/read (permission, I/O) classifies `Inaccessible` and `pi_prune` correctly leaves it alone (falls through `_ => {}`). But the returned `PiPruneOutcome` then has `removed` empty, `skipped` empty, `applied=false` — operationally indistinguishable from a clean corpus. A user who just ran `skill pi-prune --force` gets no signal that there was an owned entry it could not act on. `pi-status` shows it, but prune is the command they ran.

Fix: report owned `Inaccessible` entries in `PiPruneOutcome.skipped` (or add a distinct `PiPruneKind::Blocked`) so scripts/users can tell 'nothing to do' from 'blocked on a permission/I/O problem'. Add a test asserting an owned inaccessible entry appears in `skipped`. Pure observability — no change to what prune deletes.
