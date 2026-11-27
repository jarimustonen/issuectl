---
created: 2026-11-27
updated: 2026-11-27
type: bug
status: fixed
priority: normal
provenance: other
provenance_detail: observed while running a conductor
source_ref: taskfleet:01m8f62x3qkdqdzdspmzxyn72g/observed:dag-intake-spawnable
originating_run: 01m8f62x3qkdqdzdspmzxyn72g
originating_run_kind: spinoff
closed: 2026-11-27
commits:
- hash: 48b3c8f
  summary: 'fix(dag): never report intake items as heads of line or spawnable'
---

# dag reports untriaged, needs-info and deferred issues as spawnable heads

## Description

## Summary

`issuectl dag` reports issues in intake holding states (`untriaged`,
`needs-info`, `deferred`) as spawnable heads. An orchestrator that spawns
workers from `spawnable_heads` can therefore start work on an item nobody has
accepted yet.

## Reproduction (issuectl 0.22.0)

In a fresh repository after `issuectl init`:

1. `issuectl intake file` three items; move one to `needs-info`
   (`intake need-info`) and one to `deferred` (`intake defer`).
2. `issuectl create` one `open` task with no lane and one with `--lane unlaned`.
3. `issuectl dag` prints `spawnable heads: 5` and marks every row `▶`;
   `issuectl --json dag` has `spawnable: true` on all five `unscheduled` rows.

Expected: `spawnable heads: 2` (the two `open` tasks). The three intake items
stay listed under `unscheduled` for visibility but are neither head of line nor
spawnable.

## Why this is a defect, not intended behaviour

- The intake-flow design (`docs/design/intake-flow.md`) defines these statuses
  as reception/holding states that await a triage decision; acceptance
  (`intake accept`, `→ open`) is what makes an item backlog work.
- The default transition matrix denies `untriaged|needs-info|deferred →
  in-progress` ("start work from untriaged" / "start from parked").
- `scheduling: excluded` already refuses these statuses because they are not
  accepted.
- Nothing in `dag.rs`, `docs/design/lane-design.md`, or the preparation-gate
  design describes intake items as spawnable.

The same applies inside a real lane: an untriaged lane member can become the
lane's head of line and either be reported spawnable or block accepted work
behind it.

## Acceptance Criteria

- [x] Rows whose status is an intake holding state are never `spawnable` and
      never `is_head_of_line`, in lanes and in `unscheduled`.
- [x] A lane's work-conserving head skips such members.
- [x] `spawnable_heads` excludes them; they remain listed for visibility.
- [x] Tests cover each status (`untriaged`, `needs-info`, `deferred`, `open`
      without lane, `open` with `lane: unlaned`).
- [x] Docs and the `[Unreleased]` changelog describe the JSON contract change.
