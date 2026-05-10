---
created: 2026-05-10
updated: 2026-05-10
type: chore
status: open
priority: normal
epic: exorbitantly-ill-apples
---

# Doctor: run rename_notes_to_comments AFTER flat-layout migration so freshly-lifted dirs get one-shot Notes rename

## Description

Spin-off from @slightly-hellish-airport round-1 /llm-review (Gemini).

## Problem

In `crates/issuectl-core/src/doctor.rs::apply`, `rename_notes_to_comments`
runs in phase 3 — BEFORE the flat-layout migration in phase 5. The
`populate_notes_migration` scan only inspects `folder == "flat"`
dirs, so an issue still under `issues/{open,closed}/` whose body has
`## Notes` is invisible at scan time, and phase 3 has nothing to do
for it.

After phase 5 lifts the dir to `issues/<slug>/`, the Notes-→-Comments
rename is now applicable but never runs in this `--fix` invocation.
The user must run `issuectl doctor --fix` a SECOND time for the
rename to land. The "one-shot doctor" promise rakes a step.

## Why this needs its own design

Moving phase 3 after phase 5 is a cross-cutting change:
- `populate_notes_migration` must run on a fresh scan, so its
  classification needs to happen post-migration (not at the original
  `scan()` boundary that feeds `DoctorActions`).
- `rename_notes_to_comments` currently consumes
  `actions.notes_to_rename`; this list is built pre-migration and
  would need re-deriving from the post-migration scan.
- Conflict signalling (`notes_conflicts`) interaction with the
  post-migration `critical_blockers` re-check needs to stay
  consistent — currently a flat-lifted notes-conflict bails phase 6;
  if the rename runs first, conflict flagging shifts.

## Acceptance criteria

- A pre-migration body with `## Notes` under `issues/open/<slug>/`
  is renamed to `## Comments` in a single `--fix` invocation.
- Test: pre-migration `issues/open/foo/item.md` with `## Notes` →
  one `--fix` produces `issues/foo/item.md` with `## Comments`.
- Existing `notes_conflicts` blocker semantics preserved.

## Context

- Round-1 /llm-review on @slightly-hellish-airport (Gemini, finding #4)
- @greatly-flat-sleet
- @slightly-hellish-airport
