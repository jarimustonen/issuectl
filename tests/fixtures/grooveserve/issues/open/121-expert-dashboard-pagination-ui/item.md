---
created: 2026-05-02
updated: 2026-05-02
type: bug
reporter: jari
assignee: jari
status: in-progress
priority: high
epic: 56
related: ["#57", "#120"]
labels: [expert, ui]
---

# 121. Expert dashboard pagination UI — queue rows beyond first page are invisible

_Source: `crates/server/src/http/routes/expert.rs` dashboard handler_

## Description

The backend `list_queue` supports `limit`, `offset`, `tenant_id`,
`since`, `reason`, and `include_reviewed` parameters. But the HTTP
dashboard handler always calls `ListQueueInput::default()` — limit=25,
no offset, no filters.

Once there are more than 25 attention-requiring runs, the remaining
rows are **permanently invisible** to the expert. The queue count at
the top of the page may say "42 items" but only 25 are shown and
there is no way to reach the other 17.

This affects the PoC immediately — any tenant with real agent activity
will exceed 25 runs quickly.

## Reproduction

1. Have more than 25 agent_runs with abnormal status
2. Open `/expert` dashboard
3. Only the first 25 are shown, no pagination controls

## Scope

- Add `Query` extractor for `limit`, `offset`, `tenant_id`, `reason`,
  `include_reviewed`
- Parse `reason` query param into `QueueReason`
- Render pagination controls (prev/next, page info)
- Render reason filter and tenant filter UI elements

## Quick Test

```
curl -s 'http://localhost:PORT/expert?offset=25&limit=25' | grep '<tr class="queue-row">'
```
