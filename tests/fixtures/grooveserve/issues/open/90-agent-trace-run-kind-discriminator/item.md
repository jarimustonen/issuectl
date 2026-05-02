---
created: 2026-05-01
updated: 2026-05-01
type: task
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#60", "#62", "#82", "#57"]
labels: [agent-trace, schema]
---

# 90. agent_trace: `run_kind` discriminator column

_Source: D4 (#60) /llm-review SPIN-OFF S1_

## Description

`agent_runs` is currently used as three distinct entity types
distinguished by the implicit pattern of three column values:

| Run kind         | actor_user_id | iterations | model     | message_id | idempotency_key |
|------------------|---------------|------------|-----------|------------|-----------------|
| LLM run (#59)    | NULL          | ≥ 1        | NOT NULL  | NOT NULL   | NULL            |
| Inline decision (#60) | NULL     | 0          | NOT NULL  | NOT NULL   | NOT NULL        |
| Manual touch (#62)    | NOT NULL | 0          | NULL      | nullable   | NULL            |

Phase 4 dashboards must remember the disambiguator pattern:
- LLM run filter: `WHERE actor_user_id IS NULL AND iterations >= 1`
- Inline decision filter: `WHERE actor_user_id IS NULL AND iterations = 0`
- Manual touch filter: `WHERE actor_user_id IS NOT NULL`

Three column values acting as a discriminator is fragile:
- `aborted_max_iterations` runs that abort at iteration 1+ have
  `iterations >= 1` and pass the LLM-run filter, but the cohort
  semantics differ
- A new run kind in the future (web-initiated LLM rerun, system-
  initiated manual correction) cannot be added without rewriting
  the cohort logic in every consumer

## Scope

- New migration: `ALTER TABLE agent_runs ADD COLUMN run_kind text
  NOT NULL DEFAULT 'llm_run' CHECK (run_kind IN ('llm_run',
  'inline_decision', 'manual'))`
- Backfill: derive `run_kind` from existing column nullability
- Update writers:
  - `start_run` → 'llm_run'
  - `record_inline_decision_run` → 'inline_decision'
  - `record_manual_run` → 'manual'
- Drop the implicit-discriminator caveat from
  `crates/ops/AGENTS.md`
- Update Phase 4 query examples

## Out of scope

- Removing the redundant column-pattern checks (anchor_check,
  model_required_for_llm) — those become belt-and-braces, but
  removing them in the same migration risks rollback complexity

## Acceptance criteria

- Phase 4 SQL becomes `WHERE run_kind = 'inline_decision' AND
  decision_type = 'permanent_skip'`
- Existing tests pass (writer changes are mechanical)
- Migration is reversible

## Päätös

Not MVP-blocking. Schema cleanup. Best done before Phase 4 dashboards
codify the cohort filter against three column values. The current
docs flag this as a known limitation (`crates/ops/AGENTS.md`
"MVP-konvention varaus") — this issue fulfills that note.
