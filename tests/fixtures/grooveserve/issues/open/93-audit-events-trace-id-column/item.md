---
created: 2026-05-01
updated: 2026-05-01
type: task
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#60", "#57", "#62"]
labels: [audit, schema]
---

# 93. audit_events: first-class trace_id column

_Source: D4 (#60) /llm-review SPIN-OFF S8_

## Description

`OpContext.trace_id` is plumbed through every `ops::*` call but
`audit_events` has no symmetric column. Today the agent_run ↔
audit_event correlation is via convention:

- `record_manual_run` callers stamp
  `audit_events.metadata.agent_run_id = run.run_uuid()` per the
  module-level docstring example
- LLM tool calls inside `process_with_tools` write audit_events
  with `OpContext.trace_id = run.run_uuid()` set, but the writer
  in `crates/ops/src/audit.rs` does not read this field

A schema-level `audit_events.trace_id UUID` column would make the
join work without per-writer convention.

## Scope

Migration: `ALTER TABLE audit_events ADD COLUMN trace_id UUID`.
Update `audit::record` and `audit::record_with_email` to bind
`ctx.trace_id` (parsed as UUID, fallback NULL). Add an index on
`(tenant_id, trace_id) WHERE trace_id IS NOT NULL` for the
"all audit events for run X" lookup pattern.

## Out of scope

- Backfill existing audit_events rows (they have no trace_id to
  recover; document NULL as the legacy state)
- Removing the `metadata.agent_run_id` convention from
  `record_manual_run` — `metadata` carries other domain-specific
  context, the column-level trace_id is for the schema-level join

## Acceptance criteria

- New audit_events rows written through `OpContext.trace_id != None`
  populate the column
- Phase 4 query "all audit events for agent_run X" works via
  `WHERE audit_events.trace_id = (SELECT run_uuid FROM agent_runs
  WHERE id = ?)`
- `cargo test --workspace` clean

## Päätös

Not MVP-blocking. The metadata-key convention works today. File
when Phase 4 needs the schema-level join (or when an analytics
dashboard wants to scan audit_events by run without parsing JSON).
