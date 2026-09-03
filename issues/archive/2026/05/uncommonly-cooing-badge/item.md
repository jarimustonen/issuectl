---
created: 2026-05-06
updated: 2026-05-31
type: feature
status: done
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [dependencies, schema]
closed: 2026-05-31
---

# Dependency tracking: canonical blocked_by + cycle detection + dependency-aware queries

_Source: src/schema.rs, src/query.rs, src/cli/doctor.rs, src/web/board.js (blocked indicator)_

## Description

Add canonical 'blocked_by: [slug]' frontmatter array. DO NOT also store a reverse 'blocks' field — derive it at runtime from scanning all blocked_by arrays (avoids drift). Doctor detects: missing referenced slugs, self-dependencies, cycles. Queries: 'blocked_by:any', 'blocks:<slug>', 'blocked_by:none'. Web board shows a blocked indicator on cards whose blocked_by is non-empty and any blocker is still open. Mutation: 'issuectl depend add/remove <slug> --blocked-by <other>'. Blocker summary auto-included in agent context bundles.
