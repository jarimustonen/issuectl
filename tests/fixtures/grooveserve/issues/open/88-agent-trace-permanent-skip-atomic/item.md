---
created: 2026-05-01
updated: 2026-05-01
type: task
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#60", "#56", "#57"]
labels: [agent-trace, schema, technical-debt]
---

# 88. agent_trace: atomic permanent_skip via PgExecutor wrapper

_Source: D4 (#60) /llm-review SPIN-OFF #2_

## Description

`crates/server/src/ingest/extraction.rs::persist_permanent_skip`
writes three things in sequence:

1. `attachments::save_attachment(pool, ...)` — already committed
2. `extractions::record_extraction(pool, ...)` — already committed
3. `agent_trace::record_inline_decision(pool, ...)` — best-effort

Each call uses a separate auto-commit transaction. If the
inline-decision insert fails (rare DB hiccup), the system has an
orphan stub-extraction row with no audit row, and Phase 4's
acceptance criterion ("`decision_type='permanent_skip'` row per
skipped liite") is silently violated.

The ops-layer `record_inline_decision_run` already takes
`db: impl PgExecutor<'_>`, but:
- `crates/server/src/ingest/agent/trace.rs::record_inline_decision`
  hardcodes `pool: &PgPool` so callers cannot pass a transaction
- `attachments::save_attachment` and `extractions::record_extraction`
  also take `&PgPool` only

## Scope

- Refactor `attachments::save_attachment` to take `db: impl PgExecutor<'_>`
- Refactor `extractions::record_extraction` to take `db: impl PgExecutor<'_>`
- Refactor `agent_trace::trace::record_inline_decision` (server wrapper)
  to take `db: impl PgExecutor<'_>`
- Wrap `persist_permanent_skip` body in `pool.begin()`, commit at end
- Verify all existing call sites pass `&pool` cleanly

## Out of scope

- Same refactor for `spam_skip` / `policy_reject` / `reply_sent` —
  those have no surrounding ops writes that need atomicity (the
  decision row is the only DB effect besides best-effort
  status updates which are post-SMTP and already best-effort)

## Acceptance criteria

- `persist_permanent_skip` is atomic: extraction stub + decision row
  commit together or roll back together
- `cargo test --workspace` clean
- One test that simulates a record_inline_decision failure mid-tx
  and asserts no orphan extraction stub is left behind

## Päätös

Best-effort policy is documented today (`crates/server/AGENTS.md`
"Trace writes are best-effort"). This issue tightens the contract
specifically for permanent_skip where the orphan-stub state has
visible consequences (Phase 4 inflation rather than just a missing
audit row).

Not MVP-blocking — the gap exists in the existing code and is
self-bounded (rare DB hiccup window). File before Phase 4 dashboards
ship to consumers.
