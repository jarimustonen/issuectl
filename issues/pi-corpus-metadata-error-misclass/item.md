---
created: 2026-08-12
updated: 2026-08-12
type: bug
status: fixed
priority: high
related: ['@pi-manifest-locking']
closed: 2026-08-12
closed_by: pi-metadata-fix-agent
commits:
- hash: 60dcf2c
  summary: core Inaccessible fix
- hash: 65d3c3e
  summary: extend to dir-stat+content-read
- hash: d5f4278
  summary: llm-review apply+spinoffs
---

# pi-corpus: metadata errors misclassified as Missing → pi_prune drops the manifest row

_Source: crates/issuectl-core/src/skill.rs_

## Description

Spin-off from /llm-review of pi-manifest-locking (OpenAI #3/#4).

`classify_pi_corpus` does `let present = skill_md.symlink_metadata().is_ok();` — the doc says only NotFound counts as absent, but the code treats EVERY error (PermissionDenied, EIO, transient) as absent. An owned entry then classifies as `Missing`, and `pi_prune(apply=true)` removes its manifest row even though the file may still exist on disk → provenance loss.

`orphan_is_safely_removable` has the same defect (`Err(_) => {}` "no SKILL.md") and `pi_prune` uses `skill_md.exists()` (also collapses errors to false) before removing the row.

Fix: make classification fallible — match on `ErrorKind::NotFound` for absence and propagate/skip other errors (add an `Inaccessible` state for the non-failing `pi_status`); never use `Path::exists()` in destructive code. Pre-existing; out of scope for the locking change.

## Resolution

### 2026-08-12T17:28:02Z · @pi-metadata-fix-agent

Fixed. Root cause: classify_pi_corpus computed presence as skill_md.symlink_metadata().is_ok(), collapsing every non-NotFound stat error (permission/I/O/transient) to 'absent' → owned entry classified Missing → pi_prune(apply=true) dropped its manifest row (provenance loss).

Fix: added PiSkillState::Inaccessible. Only a genuine ErrorKind::NotFound counts as absent; any other error on an OWNED entry (dir stat, SKILL.md stat, or content read) → Inaccessible, checked before the Missing branch. pi_prune only acts on Orphan/Missing so Inaccessible is never pruned; pi_status surfaces it (rendered ?, counts as a finding). Audited the other metadata call sites (ensure_pi_mirror_target_within_corpus, orphan_is_safely_removable, prune remove_file/remove_dir) — all already fail-closed on non-NotFound; unchanged. NotFound entries still classify Missing and prune (true-prune path unchanged).

Also: content reads use raw bytes so an unreadable file → Inaccessible (was fabricated Modified/Stale) while non-UTF-8 stays real drift; entry-dir symlink gate no longer fail-opens on a non-NotFound dir-stat error; Inaccessible scoped to owned entries.

Regression tests (hermetic, cfg(unix)): ENOTDIR stat error and EACCES content-read failure both classify Inaccessible and survive pi_prune(apply=true) with the row intact. Green gate: cargo test 1014 pass, clippy no new warnings, fmt clean.

/llm-review (gemini/openai/anthropic/deepseek) CONFIRMED no remaining error path collapses into Missing. FIX findings applied; 2 observability spin-offs staged (@pi-prune-report-inaccessible, @pi-owned-symlink-unmanaged-hidden). Report: history/review-pi-corpus-metadata-error-misclass.md
