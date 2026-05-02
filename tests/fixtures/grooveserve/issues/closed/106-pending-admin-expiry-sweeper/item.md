---
created: 2026-05-01
updated: 2026-05-01
closed: 2026-05-01
type: task
reporter: jari
assignee: jari
status: done
priority: normal
related: ["#26", "#67"]
labels: [ops, sweeper, multi-tenant]
epic: 26
---

# 106. Expiry sweeper for `pending_admin_actions`

_Source: #67 v1.1 LLM review consensus (3/4 reviewers): migration 026
declares the index "for the expiry sweeper" but no sweeper exists._

## Description

`crates/ops/migrations/026_create_pending_admin_actions.sql:55` adds:

```sql
CREATE INDEX idx_pending_admin_actions_expires
    ON pending_admin_actions (expires_at)
    WHERE status = 'pending';
```

with the comment "for the expiry sweeper". No sweeper exists. Today
`inspect_pending` and `confirm_pending` both reject expired tokens
correctly so functional impact is bounded — but the table grows
unbounded as `status='pending'` rows past their `expires_at` accumulate,
and a future admin-list page (`/admin/pending`) would need to filter
expired rows out manually.

## Required change

Add `crates/server/src/ingest/pending_expiry_sweeper.rs` modeled on
`agent_runs_sweeper.rs`:

```rust
pub async fn sweep_expired_pendings(pool: &PgPool) -> sqlx::Result<u64> {
    let result = sqlx::query(
        "UPDATE pending_admin_actions
            SET status = 'expired'
          WHERE status = 'pending' AND expires_at < NOW()",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub async fn run_pending_expiry_sweeper(pool: PgPool) -> ! {
    let mut interval = tokio::time::interval(Duration::from_secs(600));
    loop {
        interval.tick().await;
        match sweep_expired_pendings(&pool).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(swept = n, "expired pending admin actions"),
            Err(e) => tracing::error!(?e, "pending expiry sweep failed"),
        }
    }
}
```

Fold into `main.rs`'s `tokio::select!` supervisor next to the
agent-runs sweeper. Cadence: 10 minutes is fine (no urgency — rows
are already rejected at use time).

## Acceptance criteria

- New `pending_expiry_sweeper` task in the supervisor.
- Tests:
  - Insert 5 pending rows with `expires_at < NOW()` → sweep marks them
    `expired`.
  - Insert 5 not-yet-expired → sweep skips them.
  - `confirm_pending` on an expired row still returns `InvalidToken`.
- `inspect_pending`, `confirm_pending`, `cancel_pending` keep rejecting
  expired rows on use (defense in depth — sweeper is just for table
  hygiene).

## Out of scope

- Cleanup/archival of confirmed/cancelled rows (likely retain forever
  for audit; revisit if storage is a concern).
- A pending-list `/admin/pending` UI page — separate work, not blocked
  by the sweeper.
