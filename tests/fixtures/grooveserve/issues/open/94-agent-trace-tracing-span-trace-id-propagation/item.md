---
created: 2026-05-01
updated: 2026-05-01
type: task
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#60", "#57", "#93"]
labels: [agent-trace, observability]
---

# 94. agent_trace: trace_id propagation from tracing::Span for inline decisions

_Source: D4 (#60) /llm-review SPIN-OFF A (trace_id half)_

## Description

`record_inline_decision_run` reads `ctx.trace_id` (parsed as UUID)
and writes it to `agent_runs.trace_id` so the row joins back to the
caller's tracing span. The fallback when `ctx.trace_id` is `None`
is a fresh `Uuid::new_v4()`.

Production callers always pass `trace_id: None`:
`inline_decision_op_context` hardcodes `trace_id: None`, so every
production inline-decision row has a fresh UUID with no log
correspondent.

The `inline_decision_uses_ctx_trace_id_when_provided` ops test
proves the pipe works end-to-end, but no production caller
exercises it.

## Scope

- Pipeline-level: extract a UUID-shaped trace_id from the
  surrounding tracing span (`tracing::Span::current()`), pass it to
  `inline_decision_op_context(tenant_id, user_id, trace_id)`
- Helper: extend `inline_decision_op_context` to accept the
  trace_id (or remove the helper and inline its construction)
- Tracing setup: ensure each per-message span carries a UUID-shaped
  trace_id field (the agent loop's `process_with_tools` already
  does this via `tracing::field::Empty` + later `record`; the
  pipeline-level pre-LLM stages do not)

## Out of scope

- `audit_events.trace_id` schema column — separate issue (#93)
- Real distributed-tracing integration (OpenTelemetry, Jaeger) —
  this issue is just about the existing `agent_runs.trace_id` join
  key being populated meaningfully

## Acceptance criteria

- Production inline-decision rows have `trace_id` matching the
  per-message tracing span's UUID field
- Log lines and DB rows can be joined by trace_id
- Manual test: tail a log line, grep its trace_id, find the row in
  `agent_runs`

## Päätös

Not MVP-blocking. The fresh-UUID fallback writes a valid row; the
log/DB correlation gap is operational nice-to-have, not a
correctness concern. File for when log-driven debugging needs the
join.
