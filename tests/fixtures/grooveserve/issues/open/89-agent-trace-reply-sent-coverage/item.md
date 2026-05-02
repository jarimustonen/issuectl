---
created: 2026-05-01
updated: 2026-05-01
type: task
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#60", "#57"]
labels: [agent-trace, observability]
---

# 89. agent_trace: reply_sent coverage for non-AssistantThreadReply paths

_Source: D4 (#60) /llm-review SPIN-OFF D_

## Description

`KnownDecisionType::ReplySent` is emitted only by
`pipeline::finalize_after_smtp::AssistantThreadReply` (and
`PolicyReplySent` by the StatusUpdate variant when
`policy_reply_lane` is `Some`). Five other SMTP-reply paths land
without producing any `*Sent` decision row:

1. **Healthcheck reply** — `pipeline::process_message` Step 4
   "healthcheck" recipient, `FinalizeAction::StatusUpdate`
2. **AI-not-available fallback** — `pipeline::run_assistant_reply`
   `ai_client.is_none()` branch, `FinalizeAction::StatusUpdate`
3. **Retry-exhausted error reply** — `pipeline::retry_failure_effect`
   "retries exhausted" branch, `FinalizeAction::StatusUpdate`
4. **LegacyConversationReply** — `pipeline::retry_message` no-thread
   path, `FinalizeAction::LegacyConversationReply`

Phase 4's "was a reply sent for this email?" SQL filter on
`agent_steps` is incomplete on these paths.

## Scope

For each path:
- Decide whether the reply is "the agent communicating with a user"
  (worth a decision row) or "infrastructure-level transport"
  (skip)
- Healthcheck and AI-not-available are infrastructure → skip,
  document the gap
- Retry-exhausted is user-facing → write a `reply_sent` row
- LegacyConversationReply is user-facing → write a `reply_sent` row

Implementation:
- Extend `LegacyFinalize` with `tenant_id`, `user_id`, `model`
  (currently only `sender_key` is carried)
- Extend `SimpleFinalize` shape if retry-exhausted needs a similar
  lane field
- Wire `record_inline_decision` calls in `finalize_after_smtp`'s
  LegacyConversationReply and StatusUpdate (retry-exhausted) branches

## Out of scope

- Healthcheck and AI-not-available decision rows (intentionally
  excluded — these are not user-facing replies)

## Acceptance criteria

- LegacyConversationReply produces `reply_sent` decision row on
  successful SMTP
- Retry-exhausted produces a `reply_sent` decision row (or a new
  `reply_failed_final` variant — discuss before coding)
- Phase 4 query "all replies sent for message_id X" returns
  complete results across all assistant lanes

## Päätös

Not MVP-blocking. The existing implementation honors the original
`#60` acceptance criterion ("reply_sent rivi happy path:lla"). File
before Phase 4 dashboards make completeness an issue.
