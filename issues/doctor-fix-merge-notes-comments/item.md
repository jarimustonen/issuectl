---
created: 2026-08-11
updated: 2026-08-12
type: feature
status: done
priority: low
commits:
- hash: 4ce8055
  summary: 'auto-merge ## Notes into ## Comments in doctor --fix'
closed: 2026-08-12
---

# doctor --fix: auto-merge ## Notes into ## Comments when both exist

_Source: crates/issuectl-core/src/doctor_

## Description

Follow-up scoped OUT of @warn-reserved-notes-section.

## Description
`## Notes` is a legacy alias for `## Comments`. Today when an issue body contains BOTH `## Notes` and `## Comments`, `issuectl doctor` reports "manual merge required" and the pre-commit hook blocks the commit. The warn-reserved-notes-section round added an authoring-time WARNING (surfaces the collision early) but deliberately did not touch doctor's merge behavior.

## Suggestion
Let `doctor --fix` auto-merge `## Notes` into `## Comments` even when both exist, preserving section/entry order, instead of demanding a manual merge. Aligns with doctor's forward-progress-only `--fix` model.

## Origin
Suggestion #2 in the @warn-reserved-notes-section issue body; left out of scope so that round stayed tight.
