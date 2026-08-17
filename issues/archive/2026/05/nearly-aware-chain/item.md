---
created: 2026-05-10
updated: 2026-05-10
type: bug
status: fixed
priority: normal
epic: exorbitantly-ill-apples
commits:
- hash: 04ee376
  summary: preserve partial flat_layout_migrated on phase-5 mid-loop failure (apply_error field)
- hash: 5fca048
  summary: round-1 review fixes — exit non-zero on apply_error; mark aborted summary
closed: 2026-05-10
---

# Doctor: execute_migrate_layout_plan Err discards partial flat_layout_migrated outcome

## Description

Spin-off from @slightly-hellish-airport round-1 /llm-review (Gemini, DeepSeek).

## Problem

In `crates/issuectl-core/src/doctor.rs::apply`:

\`\`\`rust
let exec_outcome = execute_migrate_layout_plan(plan, lock);
outcome.flat_layout_migrated = exec_outcome.migrated;
crate::migrate_layout::prune_empty_legacy_parents(&repo_root.join(\"issues\"));
if let Some(err) = exec_outcome.error {
    return Err(err);
}
\`\`\`

`ExecuteOutcome` is designed to carry partial progress on mid-loop
failure (the `migrated` list is populated up to the failing rename
before `error` is set). But on `Err`, `apply` returns `Err(err)` —
which propagates out of `run` past the `render_text` /
`render_json` calls. JSON consumers that trust AGENTS.md's
\"always `--json` when scripting\" rule receive an anyhow-formatted
stderr blob instead of a structured envelope, and the partial
moves on disk are invisible.

This breaks both the structured-output contract AND the partial-
progress contract that `ExecuteOutcome` was built to support.

## Why this needs its own design

The natural fix is `outcome.blockers.push(err.to_string()); return
Ok(outcome);` — but that puts an EXECUTION error into `blockers`,
which after @rather-abhorrent-edge becomes a phase-aware structure.
Should `flat_layout_migrated_partial`, `flat_layout_migration_error`,
and `blockers` be separate fields? The shape is entangled with the
output-contract redesign in @rather-abhorrent-edge — best resolved
together.

## Acceptance criteria

- `--json --fix` on a phase-5 mid-loop failure emits a structured
  envelope with the partial `flat_layout_migrated` list AND the
  failure cause.
- Test: a phase-5 plan whose second move would fail (read-only
  destination) leaves the first move on disk and the JSON envelope
  reports it.

## Context

- Round-1 /llm-review on @slightly-hellish-airport (Gemini #1, DeepSeek #4c)
- Blocked-by adjacency: @rather-abhorrent-edge (output contract)
