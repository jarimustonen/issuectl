---
created: 2026-05-10
updated: 2026-05-10
type: bug
status: open
priority: normal
epic: exorbitantly-ill-apples
---

# Doctor: rewrite_item_frontmatter ignores blocked_by — legacy numeric refs orphaned across NN-rename

## Description

Spin-off from @slightly-hellish-airport round-1 /llm-review (Gemini).

## Problem

`rewrite_item_frontmatter` in `crates/issuectl-core/src/doctor.rs`
rewrites `epic` and `related: ["#NN", ...]` legacy numeric refs to
`@<slug>` form, but does NOT touch `blocked_by`. After NN-rename:

- `epic: 7` → `epic: <slug-of-7>` ✓
- `related: ["#7"]` → `related: ["@<slug-of-7>"]` ✓
- `blocked_by: ["#7"]` → unchanged ✗

`populate_extended_validation::check_ref` classifies the surviving
`#7` as `(legacy numeric ref)`, which is INTENTIONALLY excluded
from `critical_blockers` (so legacy migrations aren't blocked by
the very refs they heal). The result: a `blocked_by` ref is silently
orphaned across the migration and `doctor --fix` re-runs do not heal
it.

## Acceptance criteria

- `rewrite_item_frontmatter` translates legacy numeric refs in
  `blocked_by` using the same `number_to_slug` map as `related`.
- Test: a legacy issue with `blocked_by: ["#7"]` migrated through
  NN-rename ends with `blocked_by: ["@<slug-of-7>"]`.
- Existing `related` rewrite tests stay green.

## Context

- Round-1 /llm-review on @slightly-hellish-airport (Gemini, finding #2)
- Pre-existing — present before @greatly-flat-sleet, surfaced
  during the post-migration safety review.
