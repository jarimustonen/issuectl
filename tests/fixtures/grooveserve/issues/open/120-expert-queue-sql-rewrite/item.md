---
created: 2026-05-02
updated: 2026-05-02
type: improvement
reporter: jari
assignee: jari
status: in-progress
priority: high
epic: 56
related: ["#57"]
labels: [expert, performance, correctness]
---

# 120. Expert queue SQL rewrite — push reason filtering and N+1 EXISTS checks into main query

_Source: `crates/ops/src/expert.rs::list_queue`_

## Description

The expert dashboard's `list_queue` has two problems discovered in
`/llm-review` (2026-05-02, `history/review-expert-dashboard.md`):

1. **2 of 3 QueueReason variants are dead code.** The SQL hardcodes
   `WHERE ar.status NOT IN ('completed', 'running')`, which means
   completed/running runs with low-confidence extractions or
   permanent_skip decisions **never appear in the queue**. These are
   exactly the subtle cases the dashboard should surface.

2. **N+1 queries with swallowed errors.** `compute_reasons` issues 2
   separate `EXISTS` queries per queue row (up to 200 extra
   round-trips per page with limit=100) and silently swallows DB
   errors with `.unwrap_or(false)`.

Both should be fixed by rewriting the queue query:

- Compute all three reason booleans via SQL `EXISTS` subqueries in
  the main query
- Filter on the disjunction of all three reasons
- Compute `COUNT(*) OVER ()` **after** filtering
- Remove the `compute_reasons` helper entirely
- Split `include_reviewed` into two query shapes so PostgreSQL can
  reliably use the partial index `idx_agent_runs_unreviewed_attention`
  when `include_reviewed=false` (avoid bind-parameter `OR` that
  confuses the planner)

## Scope

- Rewrite `list_queue` SQL query
- Remove `compute_reasons` helper
- Update tests to cover completed runs with low-confidence and
  permanent_skip in the queue
- Verify partial index usage with `EXPLAIN`

## Quick Test

```sql
-- Before: completed run with permanent_skip is invisible
-- After: it appears in the queue with QueueReason::PermanentSkip
```
