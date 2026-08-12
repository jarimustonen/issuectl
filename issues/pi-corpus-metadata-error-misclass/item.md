---
created: 2026-08-12
updated: 2026-08-12
type: bug
status: open
priority: high
related: ['@pi-manifest-locking']
---

# pi-corpus: metadata errors misclassified as Missing → pi_prune drops the manifest row

_Source: crates/issuectl-core/src/skill.rs_

## Description

Spin-off from /llm-review of pi-manifest-locking (OpenAI #3/#4).

`classify_pi_corpus` does `let present = skill_md.symlink_metadata().is_ok();` — the doc says only NotFound counts as absent, but the code treats EVERY error (PermissionDenied, EIO, transient) as absent. An owned entry then classifies as `Missing`, and `pi_prune(apply=true)` removes its manifest row even though the file may still exist on disk → provenance loss.

`orphan_is_safely_removable` has the same defect (`Err(_) => {}` "no SKILL.md") and `pi_prune` uses `skill_md.exists()` (also collapses errors to false) before removing the row.

Fix: make classification fallible — match on `ErrorKind::NotFound` for absence and propagate/skip other errors (add an `Inaccessible` state for the non-failing `pi_status`); never use `Path::exists()` in destructive code. Pre-existing; out of scope for the locking change.
