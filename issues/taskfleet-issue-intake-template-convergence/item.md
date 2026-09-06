---
created: 2026-09-06
updated: 2026-09-06
type: task
status: done
priority: high
closed: 2026-09-06
commits:
- hash: 3d374e1
  summary: point issue-intake analysis prerequisite to Taskfleet
---

# Converge issue-intake templates on Taskfleet

## Goal

Update issuectl's canonical `/issue-intake` template and every byte/hash-guarded dogfood copy to name Taskfleet as the external `/worktree-bug-analysis` prerequisite.

## Authorizing evidence

Taskfleet ADR 0002 E1 owner map: https://github.com/jarimustonen/taskfleet/blob/8b8652a964a1353dc869e89fd541e8cf5b30f1e6/issues/taskfleet-dependent-owner-discovery/owner-map.md (Q1-Q3).

## Required work

- Change the canonical issue-intake skill and prompt templates from the old product/command prerequisite to canonical `taskfleet` wording.
- Regenerate/synchronize `.claude`, `.pi`, and `.codex` dogfood copies through issuectl's supported generation path; do not hand-fork generated files.
- Update exact template/hash/snapshot tests and documentation that represents current prerequisites.
- Preserve predecessor compatibility fixtures and protocol identifiers where intentional. This bounded compatibility strategy was later superseded by the repository-wide clean break.
- Run the repository's full green gate and follow its normal release cadence so downstream repositories can refresh from an available canonical template.

## Acceptance Criteria

- [x] Canonical template and all dogfood copies consistently identify `taskfleet`.
- [x] Generated-copy integrity tests pass and intentional compatibility fixtures remain unchanged.
- [x] Full repository gate passes.
- [ ] A normal issuectl release is cut if required by repository policy, and the downstream refresh coordinate is recorded.

## Resolution

### 2026-09-06T11:04:39Z · @issuectl

Canonical templates, all generated dogfood copies, prerequisite guard, changelog, and the exact full repository green gate are complete at 3d374e1. Per run policy, the normal release remains a post-merge conductor action after exact-main CI is green; no release was cut from this worker branch.
