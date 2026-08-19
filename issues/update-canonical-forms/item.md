---
created: 2026-08-17
updated: 2026-08-19
type: feature
status: in-progress
priority: normal
labels: [architecture]
lane: verb-surface
lane_seq: 5
collision: [crates/issuectl/src/cmd/write.rs, crates/issuectl-core/src/mutate/update.rs, crates/issuectl/src/cmd/mod.rs, crates/issuectl/src/cmd/runtime.rs, crates/issuectl/src/cmd/views_extra.rs]
commits:
- hash: fc2d44c
  summary: add canonical update patch and query forms
- hash: 1b95604
  summary: preserve update compatibility and complete parity tests
---

# Add canonical update forms per ADR 0004 (0.15.0 prep)

## Description

## Goal

The 0.15.0 preparation slice of [ADR 0004](../../docs/decisions/0004-cli-verb-surface.md):
give `update` every canonical form the 0.16.0 folds will alias to, WITHOUT touching the
commands being folded (they keep working unchanged until 0.16.0).

## Scope

Add to `issuectl update`:

- `--patch-file <path>` — the one-transaction YAML patch `apply` performs today (same parser,
  same `expected-version` compare-and-swap semantics, same `body_ops`). `apply` itself is not
  modified.
- `--query "<q>"` — batch selection with the same query syntax and dry-run/diff semantics
  `bulk` has today, combinable with the same mutation flags. `bulk` itself is not modified.
- Confirm the existing flags already cover the other folds and close any gap found:
  `--status <closing>` (replaces `close` — verify closing-status side effects like `closed:`
  stamping and auto-archive parity), `--add-label/--remove-label` (replaces `label`),
  `--assignee`/`--no-assignee` (replaces `assign`), `--add-blocked-by/--remove-blocked-by`
  (replaces `depend`), `--field`/`--clear-field` (replaces `set`).

All new forms route through the existing locked, schema-validated mutation path in
`issuectl-core/src/mutate/` (AGENTS.md rule); handlers stay thin. Mutually-exclusive
combinations (`--patch-file` with field flags; `--query` with a positional slug) are rejected
with informative errors. `--json` output for the new forms mirrors the folded commands'
envelopes.

## Out of scope

Hiding/aliasing the folded commands, deprecation warnings, help/skill canonical-only rewrites,
`pick`/`new`/`ls`/`comment` treatment — all of that is the 0.16.0 deprecation release, a
separate issue filed once this lands.

## Acceptance

- Each new/verified form has tests proving parity with its folded counterpart (same repo
  mutation, same JSON result shape, same error on invalid input).
- Skill templates updated in the same commit ONLY where they must change (sync rule).
- Green gate passes.
