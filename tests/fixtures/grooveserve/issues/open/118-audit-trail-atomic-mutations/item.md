---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#115", "#57", "#81"]
labels: [ops, server, audit]
---

# 118. Audit trail atomic mutations

_Source: `crates/ops/src/receipts/` + `crates/server/src/http/routes/receipt_edit.rs`_

## Description

`audit::record(&state.db, ...)` is called AFTER the mutation transaction commits in `edit_submit` and `restore` (and likely other route handlers). If the audit INSERT fails (network blip, connection pool exhaustion, disk full), the receipt mutation is persisted with no audit trail. This is a systemic pattern — the `audit::record` function takes `&PgPool` and opens its own transaction internally, making it impossible to include in the caller's transaction.

The fix is structural: add an `audit::record_tx(&mut PgConnection, ...)` variant that callers with an open transaction can use. Then convert call sites that already hold a transaction. Keep the existing `audit::record(&PgPool, ...)` wrapper for call sites that don't have one.

## Scope

- Add `audit::record_tx` accepting `&mut PgConnection` (or `&mut Transaction`)
- Convert `edit_submit` and `restore` in `receipt_edit.rs` to call audit inside the ops transaction
- Convert `pending_admin` flow if it has the same pattern
- Keep the pool-wrapper for call sites without a transaction
- Verify no other mutation routes have the same split

## Related review findings

From #115 review: all 4 reviewers flagged this. The assessment classifies it as SPIN-OFF because the fix touches every audit call site.
