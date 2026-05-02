---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#115", "#37"]
labels: [ops, server, concurrency]
---

# 119. Optimistic concurrency for edit forms

_Source: `crates/server/src/http/routes/receipt_edit.rs` + `crates/ops/src/receipts/update.rs`_

## Description

The edit form submits ALL pre-filled fields as `Some(...)` values. If another process (agent, another user) updates the receipt between form load and submit, the stale form silently reverts those concurrent changes. Example: user opens form showing vendor "A", agent updates vendor to "B", user changes only payment_method and submits — the form sends vendor "A" as `Some("A")`, COALESCE overwrites the agent's "B" back to "A".

The fix requires optimistic concurrency control:
1. Include a version token (`updated_at` timestamp or revision version number) in the edit form as a hidden field
2. On submit, compare the token against the current row state (under the lock)
3. If they differ, reject the update with a "this receipt was modified since you loaded the form" error, re-rendering the form with the new values and the user's changes preserved

This touches the edit form template, the update route, and the `UpdateReceiptInput` type. Combined with the tri-state `Patch<T>` redesign (#115 finding #1), the stale-overwrite fix is part of a broader form-state refactor.

## Related review findings

From #115 review assessment finding #27 (SPIN-OFF). Also relates to finding #1 (patch semantics — `Option<T>` = "leave unchanged" design exhaustion) and #37 (update tools cannot clear fields).
