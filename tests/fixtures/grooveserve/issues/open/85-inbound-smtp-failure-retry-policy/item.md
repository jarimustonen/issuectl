---
created: 2026-05-01
updated: 2026-05-01
type: task
reporter: jari
assignee: jari
status: open
priority: normal
epic: 56
related: ["#78"]
labels: [foundation, ingest]
---

# 85. Inbound (first-attempt) SMTP failure: retry-policy or IDLE-reclaim?

_Source: A5b `/llm-review` U1 (DISCUSS). Surfaced once C5's retry-side fix landed and exposed the asymmetry on the inbound path._

## Description

A5b's `/llm-review` round 2 raised an unresolved product question:
should an inbound (**first-attempt**) SMTP failure go through the
retry-policy queue (same shape as the retry-side fix), or stay on
the current "leave UNSEEN, IDLE re-fetches" model?

### Today's behaviour

`runner::dispatch_inbound_effect` (handles the IDLE path) does:

```rust
PipelineEffect::ReplyThen { reply, finalize, folder } => {
    let reply_message_id = smtp::send_reply(...).await?;  // <-- ? bubbles SMTP errors
    pipeline::finalize_after_smtp(...).await;
    imap::move_message(session, uid, folder.as_str()).await?;
}
```

If `smtp::send_reply` returns `Err`, the `?` bubbles to `handle_uid`,
which only `tracing::error!`s and exits. The IMAP message is **never
marked seen**, the message stays UNSEEN in INBOX, and the next IDLE
wake-up re-fetches the same UID. `claim_with_thread` returns
`Reclaimed` (status was `processing`), the agent loop runs again
(billing Anthropic again), and SMTP is retried.

On a **sustained** SMTP outage this loops on every IDLE wakeup
until either SMTP recovers or operator intervention. The same shape
as the retry-side regression C5 fixed — but on the inbound path.

This is **pre-existing** behaviour, not a regression introduced by
A5b.

### The question

Choose one:

**Option A: Apply retry-policy to inbound SMTP failure too.**
On `Err` from `smtp::send_reply`, call `retry_policy::schedule(...)`
to flip `email_processing.status = 'retryable'`, then `imap::move_message(session, uid, "Processed")` so IMAP doesn't refetch. Retry-poller picks it up later with backoff.
- Pro: Centralises retry semantics; bounds Anthropic spend on outage.
- Con: Adds a second source of `email_processing.status='retryable'`
  rows (today they only come from agent failures).

**Option B: Keep "leave UNSEEN, IDLE reclaim".**
Current behaviour. Add a UNSEEN-reclaim counter in `email_processing`
(or a `reclaim_count` column) so the runner can bound it after N
attempts.
- Pro: Preserves the IMAP-driven model (retries are visible in IMAP).
- Con: Two retry mechanisms (DB queue + IMAP UNSEEN); operator has
  to grok both.

## Trade-offs to weigh

- Anthropic re-billing cost during sustained SMTP outage: A bounds
  it; B doesn't unless we add the counter.
- Operator visibility: A surfaces in `email_processing` queries
  alongside agent retries; B leaves it implicit in IMAP UID state.
- Test coverage: A is easier to test (deterministic DB state); B
  requires IMAP fixtures.

## Decision needed

Lead suggestion: **Option A**, because it dovetails with C5's fix
(same code shape applied symmetrically) and the retry-poller already
exists. But the product call is yours.

## Out of scope

- C5 (retry-side SMTP failure) — already fixed in A5b.
- WAL-style recovery for persist failures — see #82.

## Related

- A5b `/llm-review` U1 (DISCUSS).
- A5b's C5 fix in `runner::run_retry`.
- `runner::dispatch_inbound_effect` (the call site to change).
