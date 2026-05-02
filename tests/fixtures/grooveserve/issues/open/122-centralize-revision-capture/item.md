---
created: 2026-05-02
updated: 2026-05-02
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#57", "#38"]
labels: [expert, refactor, maintenance]
---

# 122. Centralize receipt revision capture — eliminate duplicated column list

_Source: `crates/ops/src/expert.rs::capture_receipt_revision` and
`crates/ops/src/receipts/revision.rs`_

## Description

`capture_receipt_revision` in the expert module duplicates the
snapshot column list from `receipts/revision.rs`, using
`try_get`-on-untyped-rows which is fragile. The comment on the expert
copy says "Column list mirrors the revision capture in
receipts/revision.rs" — acknowledging the duplication.

If the `receipts` schema gains a column, one path will be updated and
the other silently drops the column from new revisions. This is a
silent data loss risk.

## Scope

- Extract a shared `capture_receipt_revision_tx(receipt_id, metadata)`
  function in `crates/ops/src/receipts/revision.rs`
- Call it from both the agent write path and the expert path
- Add `captured_by_actor_user_id` and `captured_by_agent_step_id`
  provenance metadata to the shared function signature
- Regression-test against existing receipt revision tests from both
  callers

## Quick Test

```bash
# Verify both paths produce identical revision rows for the same receipt state
cargo test -p grooveserve-ops receipts::revision
cargo test -p grooveserve-ops expert::tests
```
