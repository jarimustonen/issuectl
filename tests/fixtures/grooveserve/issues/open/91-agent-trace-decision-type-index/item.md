---
created: 2026-05-01
updated: 2026-05-01
type: task
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#60", "#57"]
labels: [agent-trace, schema, performance]
---

# 91. agent_trace: covering index for Phase 4 decision queries

_Source: D4 (#60) /llm-review SPIN-OFF S2_

## Description

`#60`'s acceptance criterion is:

> Asiantuntijakysymys "kaikki tämän viikon permanent_skipit"
> vastattavissa yhdellä SQL-kyselyllä:
> `SELECT * FROM agent_steps WHERE kind='decision' AND
>  decision_type='permanent_skip' AND created_at > '...'`

Today this query has no covering index. As `agent_steps` accumulates
production rows, the query degrades to a sequential scan.

## Scope

Add a partial index in a new migration:

```sql
CREATE INDEX CONCURRENTLY idx_agent_steps_tenant_decision_created
    ON agent_steps (tenant_id, decision_type, created_at DESC)
    WHERE kind = 'decision';
```

Tenant-scoped because Phase 4 queries are always filtered by
`(tenant_id, user_id)` for multi-tenant correctness.

## Acceptance criteria

- `EXPLAIN` on the Phase 4 query shows index scan on the new index
- No regression in INSERT throughput (partial index excludes
  non-decision steps)

## Päätös

Not MVP-blocking — `agent_steps` is empty / negligibly small in
the PoC. Land when Phase 4 implementation begins or when row
counts justify (~10k rows, where seq-scan latency becomes
noticeable on realistic queries).
