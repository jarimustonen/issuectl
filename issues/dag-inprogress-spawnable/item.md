---
created: 2026-08-10
updated: 2026-08-10
type: bug
status: in-progress
priority: normal
---

# issuectl dag reports spawnable=true for in-progress issues

## Description

`issuectl dag` marks an issue `spawnable: true` even when its `status` is `in-progress`.

## Observed
An `in-progress`, unlaned issue shows `spawnable=true` in `issuectl dag --json`. Discovered while adopting `issuectl dag` in the homebase repo (the `adopt-issuectl-dag` issue was `in-progress` in its own worktree yet reported spawnable).

## Expected
The execution-DAG eligibility rule (the convention `issuectl dag` implements) treats an already `in-progress` issue as **not** eligible to spawn — otherwise a scheduler could launch a *second* worker on work already underway. Either:
- derive non-spawnability from `status == in-progress` directly, or
- document explicitly that in-progress exclusion is delegated entirely to the caller's `--reservations` input (and that `spawnable` means "deps satisfied", not "safe to launch").

## Impact
A caller that trusts `spawnable` without also cross-checking status can double-spawn an in-progress issue. Low severity today (callers also read status), but it makes the `spawnable` contract ambiguous.

## Context
Filed from homebase `adopt-issuectl-dag`. See `issues/adopt-issuectl-dag/dag-comparison.md` there for the full reproduction.
