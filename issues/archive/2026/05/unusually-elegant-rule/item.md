---
created: 2026-05-06
updated: 2026-05-08
type: feature
status: done
priority: high
reporter: jari
assignee: jari
epic: exorbitantly-ill-apples
labels: [foundation, query]
commits:
- hash: 7be33a4
  summary: shared query engine for ls/search/api
- hash: c731954
  summary: apply review fixes (backwards-compat, semantics, perf, DoS caps)
- hash: 379005f
  summary: apply round-2 review fixes (panic guard, quoted escape, date prefix)
closed: 2026-05-08
---

# Shared query engine (CLI + web + automation) with --json and full-text search

_Source: src/query.rs (new), src/cli/ls.rs, src/cli/search.rs, src/web/api.rs_

## Description

One query syntax shared by CLI, web filters, saved queries, bulk edits, reports. v1 keep-it-simple: 'field:value', '-field:value', 'text:"phrase"', 'status:any', 'assignee:none', relative dates ('updated:<-14d'). Powers 'issuectl list' / 'issuectl search'. Substrate for several other features in this epic (saved queries, dependency-aware queries, stale reports, agent discovery).
