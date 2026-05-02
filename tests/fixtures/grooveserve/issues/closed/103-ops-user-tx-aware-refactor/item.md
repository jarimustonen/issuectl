---
created: 2026-05-01
updated: 2026-05-01
closed: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: done
priority: high
related: ["#26", "#41", "#67"]
labels: [ops, refactor, tech-debt, multi-tenant]
epic: 26
---

# 103. Refactor `ops::user::*` + `ops::invitation::*` into tx-aware primitives

_Source: #67 v1.1 LLM review consensus (4/4 reviewers): inline SQL
duplication in `pending_admin.rs`._

## Description

`crates/ops/src/pending_admin.rs::confirm_pending` re-implements the
full body of `disable_user`, `enable_user`, `update_role`, and
`invite_user` because the public `ops::user::*` / `ops::invitation::*`
helpers each open their own `db.begin()` and would deadlock on the row
already locked by `confirm_pending` (`pending_admin_actions` `FOR
UPDATE`).

The duplication is real, the deadlock argument is correct, but the
solution adopted (inline copies labelled `apply_*_inline`) is wrong:

- Future changes to `ops::user::*` (new audit columns, new side
  effects, role rules) will silently skip the inline copies.
- The `via_pending` audit shape will diverge from direct ops audit.
- When #41 (approval queue) lands, `set_expense_status` from
  `EmailAgent` will go through the same pending path and we'll fork
  that too.

## Required change

Refactor each canonical ops fn into a transaction-aware primitive plus
a thin pool wrapper:

```rust
// in ops/src/user.rs
pub async fn disable_user(
    db: &PgPool, ctx: &OpContext, target_user_id: i64,
) -> Result<(), OpError> {
    let mut tx = db.begin().await?;
    disable_user_tx(&mut tx, ctx, target_user_id).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn disable_user_tx(
    tx: &mut Transaction<'_, Postgres>, ctx: &OpContext, target_user_id: i64,
) -> Result<(), OpError> { /* current body */ }
```

Same pattern for `enable_user`, `update_role`, and
`invitation::invite_user` / `invitation::accept_invitation` if needed.

`pending_admin::confirm_pending` then drops the four `apply_*_inline`
helpers and calls the `*_tx` primitives directly. The `via_pending`
audit metadata becomes a parameter on the tx primitives so the
canonical audit row carries it without duplication.

## Why now (and why before #41)

- #41 will add `set_expense_status` as a fifth pending action; doing
  the refactor first avoids forking that op too.
- The current duplication is a slow-moving drift risk; tests pass
  today but every future user-ops change has to be mirrored in two
  places.

## Acceptance criteria

- `apply_*_inline` helpers in `pending_admin.rs` are deleted.
- `confirm_pending` calls `user::disable_user_tx` etc. directly.
- All existing tests pass; no behavioural change.
- AGENTS.md note in `crates/ops/AGENTS.md` documents the
  pool-vs-tx convention so future ops authors know which to add.
- Apply same pattern to `invitation::invite_user_tx`.

## Out of scope

- Splitting other ops modules (`receipts`, `expenses`, …) into
  tx-aware variants. Do those in their own issues when their first
  pending-action consumer lands.
