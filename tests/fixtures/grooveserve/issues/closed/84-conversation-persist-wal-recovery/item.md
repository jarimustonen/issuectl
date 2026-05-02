---
created: 2026-05-01
updated: 2026-05-01
closed: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: done
priority: normal
epic: 56
related: ["#78"]
labels: [foundation, ingest, durability]
commits:
  - hash: ff8dd6e
    summary: "feat(ingest): pending_replies WAL recovery for post-SMTP persist failures (#84)"
---

# 84. Recover lost conversation rows when post-SMTP persist fails

_Source: A5b `/llm-review` (Anthropic Opus C6, DeepSeek confirms). Pre-existing in pre-A5b runner code; surfaced again in A5b pipeline review._

## Description

`pipeline::finalize_after_smtp` runs after `smtp::send_reply` succeeds.
On the assistant-thread path it calls
`ops::ingest::conversation::persist_successful_reply` to persist:

1. The agent loop's `loop_messages` (one row per turn into
   `conversations`).
2. An outbound `thread_messages` row.
3. Thread activity bump (`UPDATE threads SET last_activity_at`).
4. `email_processing.status = 'reply_sent'` flip.

All four operations run inside a single Postgres transaction. If the
transaction fails (connection pool exhaustion, network blip, deadlock,
crash mid-commit) the fallback only flips `email_processing.status`
to `'reply_sent'` via `update_status`. **The
`Vec<llm::types::Message>` content the user just received is dropped
from memory** and never reconstructed — the next inbound from this
thread loads conversation history without those turns and the model
can repeat or contradict its own earlier reply.

This is **pre-existing** behaviour (identical pattern in pre-A5b
commit `72cf289`); A5b preserved it exactly. The likelihood is RARE
(requires DB blip in the narrow window between SMTP success and
`tx.commit()`) but the consequence is silent model drift that's hard
to debug — the user sees a model that's "lost its memory" without
any operator-visible signal.

## Reproduction

Synthetic: kill the Postgres connection mid-`tx.commit()` after a
successful SMTP send. The user receives the email; the next inbound
from the same thread re-runs the agent loop with truncated history.

## Suggested approach (needs design)

Two reasonable paths, neither trivial:

### Option A: Write-Ahead Log (WAL) staging table

1. Before `smtp::send_reply`, write the prepared `loop_messages` to a
   `pending_replies` staging table along with the inbound message id
   and the recipient.
2. After SMTP succeeds, `finalize_after_smtp` promotes the staging
   row into `conversations` + the rest of the persist work.
3. A sweeper task (cron / startup-side) inspects `pending_replies`
   rows older than threshold and either retries the persist or
   alerts.

Pros: Recoverable. Bounded staging table.
Cons: New schema. Sweeper to design (idempotency, cleanup cadence).
Adds one extra DB write before SMTP — not a hot path concern.

### Option B: Best-effort logging with manual recovery

`tracing::error!` the full `loop_messages` JSON (under `debug` for
PII grounds; or hash-only) so they can be reconstructed from logs.
Same operator action as today, but the data is recoverable from log
shipping.

Pros: Zero schema change. Localised diff.
Cons: Operator burden. PII risk in logs (user receipts).

## Out of scope

- A5b's seam (the persist *signature* is fine; the failure mode is
  about durability, not boundary).
- Retry-queue rescheduling for the SMTP-failure case (that's #83).

## Related

- A5b `/llm-review` Anthropic C6, DeepSeek finding R2.
- Pre-A5b commit `72cf289` shows identical fallback shape.
- `crates/server/src/ingest/pipeline.rs::finalize_after_smtp`
- `crates/ops/src/ingest/conversation.rs::persist_successful_reply`
