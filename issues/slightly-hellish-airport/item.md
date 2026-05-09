---
created: 2026-05-09
updated: 2026-05-09
type: chore
status: open
priority: normal
epic: exorbitantly-ill-apples
---

# Doctor: post-flat-layout migration must re-check critical_blockers before NN-rename

## Description

Spin-off from @greatly-flat-sleet round-2 /llm-review (Claude#11). Latent
safety hole in `doctor::apply`'s pipeline: after
`execute_migrate_layout_plan` runs, the function re-scans the repo to
pick up frontmatter-only legacy issues that just moved into the flat
layout, then proceeds straight into NN-rename and ref-rewrite.

## Problem

The post-migration rescan does NOT re-check `critical_blockers`. If
the flat-layout migration produces a new critical condition that
wasn't visible pre-migration (for example: two issues whose post-flat
slug collides — pre-migration they lived in different folders so
`duplicate_slugs` didn't fire), apply pushes through into the
NN-rename phase. The `number_to_slug` map was built against the
pre-rescan `legacy_dirs`, so cross-references can be rewritten with
stale or wrong target slugs and `fs::rename` silently overwrites or
half-moves directories.

## Why this needs its own design

The fix is more than a one-liner: the second-stage critical_blockers
check must be able to bail cleanly with `outcome.blockers` populated
+ `outcome.flat_layout_migrated` already capturing the partial
progress. That is a new early-return shape inside `apply` and needs
its own test fixtures (currently no test simulates a migration that
creates a new critical finding). Owner should also decide whether
the partial flat-layout migration should be rolled back or kept —
both sides have user-facing impact.

## Acceptance criteria

- After `execute_migrate_layout_plan` succeeds, `apply` runs
  `critical_blockers` against the fresh scan.
- If non-empty, `apply` returns `Ok(outcome)` with `outcome.blockers`
  populated and the partial `flat_layout_migrated` list intact —
  NN-rename phase does not run.
- New test simulates a migration that creates a duplicate slug; the
  test verifies apply bails before NN-rename and outcome carries
  both the migrated moves and the new blocker.

## Related

- Round-2 /llm-review on @greatly-flat-sleet (Claude#11)
- `crates/issuectl-core/src/doctor.rs` `apply` function
