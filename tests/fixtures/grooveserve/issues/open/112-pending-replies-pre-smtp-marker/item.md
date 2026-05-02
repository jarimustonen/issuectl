---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
epic: 56
related: ["#84"]
labels: [foundation, ingest, durability, post-poc]
---

# 112. Close the post-SMTP-pre-finalize crash window in pending_replies WAL

_Source: `/llm-review` of #84 commit `ff8dd6e` — consensus across all four reviewers (Gemini/OpenAI/Anthropic/DeepSeek)._

## Description

#84 introduced a write-ahead-log (`pending_replies`) that recovers from
the **post-SMTP-success / persist-tx-failed** scenario: SMTP went out,
`persist_successful_reply_tx` crashed, the fallback flipped
`email_processing.status='reply_sent'` (stamping `reply_message_id`),
and the sweeper picks up the stale `pending` row and promotes.

The review surfaced a **separate residual gap** the WAL does not cover:
**the process can die between `smtp::send_reply` returning `Ok(reply_message_id)`
and `pipeline::finalize_after_smtp` even *beginning* its work.**

Sequence:

1. `pipeline::run_assistant_reply` writes `pending_replies` row,
   `state='pending'`, `reply_message_id=NULL`.
2. Runner calls `smtp::send_reply` → SMTP server accepts the email.
3. Runner is about to call `pipeline::finalize_after_smtp(...)`.
4. **Process dies** (SIGKILL, OOM, panic in unrelated task that takes
   the runtime down, host reboot).
5. `pending_replies.reply_message_id` still NULL.
6. `email_processing.status` still `'processing'`, no
   `reply_message_id` stamped (the fallback never ran).
7. Sweeper has no signal that SMTP succeeded → defers indefinitely
   (per the post-#84 fix in this same review round).
8. IMAP IDLE eventually reclaims the inbound (still UNSEEN or in
   INBOX since the runner's `imap::move_message` also never ran).
9. `claim_with_thread` returns `Reclaimed`.
10. Pipeline runs the agent loop **again**, gets a different reply.
11. `insert_pending_reply` upserts the existing row (state was
    `pending`, WHERE clause permits), overwriting `loop_messages_json`.
12. New SMTP send → **user receives a duplicate reply**.

The original `loop_messages` from step 1 are permanently lost; the
visible failure is the user receiving two related but possibly
contradictory emails.

## Reproduction

Synthetic: kill -9 the binary in the millisecond window between
`smtp::send_reply` returning and the next `await` in
`runner::dispatch_inbound_effect`. Easier in production: long
GC pause / OS scheduler stall coinciding with an unrelated panic
that brings down the runtime.

## Suggested approach (needs design)

### Pre-generate the outbound `Message-Id` at staging time

1. **Pre-SMTP**: `pipeline::run_assistant_reply` mints the outbound
   Message-Id (`<{uuid}@{outbound_domain}>` shape, matching what
   lettre would have generated) and persists it on the
   `pending_replies` row at insert time.
2. **SMTP send**: `smtp::send_reply` is changed to accept an
   explicit `Message-Id` header rather than letting lettre auto-mint
   one. lettre supports this via the `MessageBuilder::message_id`
   API.
3. **Schema**: add a `pending_replies.state='sent'` value (or a
   dedicated `smtp_sent_at` timestamp) so the sweeper can tell
   "we attempted SMTP" from "we never reached SMTP."
4. **Sweeper**: the recovery decision tree gains an explicit branch:
   row has `reply_message_id` set but `email_processing.status` is
   not `'reply_sent'` → SMTP-attempt status uncertain. Operator
   alert + manual reconciliation; do NOT auto-promote (the SMTP
   send may have failed mid-way) and do NOT auto-resend (we don't
   know).
5. **Insert idempotency**: `insert_pending_reply`'s `ON CONFLICT
   DO UPDATE` must refuse to overwrite a row whose `reply_message_id`
   is set — that's the durable "SMTP was attempted" marker. Reclaim
   that hits this state should treat the inbound as "already
   replied" and skip processing.

### Why this is post-PoC

The fix crosses the SMTP transport boundary, requires a new schema
state, and introduces a failure mode (`smtp_attempted_but_unconfirmed`)
that needs an operator runbook. The window itself is rare — it
requires the process to die in a sub-millisecond gap between a
socket close and the next async tick. For the PoC / MVP phase the
known-and-documented limitation is acceptable; we revisit before
real customer traffic.

The existing #84 WAL still closes the original transaction-failure
window it was scoped for — this issue is strictly an additional
durability tightening.

## Out of scope

- The original transaction-failure window (#84 — landed).
- Multi-replica deployment concerns (no current need).
- Pre-generated Message-Id as a *general* outbound feature
  (only the WAL needs it; broader use is out of scope).

## Related

- `/llm-review` of `ff8dd6e` — `history/review-issue-84-pending-replies-wal.md`
- `crates/server/AGENTS.md` "Post-SMTP durability (#84)" — document the
  residual gap as a known MVP limitation with a forward pointer to
  this issue.
- `crates/server/src/ingest/pipeline.rs::run_assistant_reply` /
  `retry_message` — both sites would write the pre-generated
  Message-Id.
- `crates/server/src/ingest/runner.rs::dispatch_inbound_effect` — SMTP
  call site; would pass the pre-generated id to the SMTP layer.
- `crates/server/src/ingest/smtp.rs` — would gain a parameter for
  the explicit Message-Id header.
