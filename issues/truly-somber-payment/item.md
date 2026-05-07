---
created: 2026-05-06
updated: 2026-05-06
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
related: ['@almost-homely-decision']
labels: [kanban, web-ui]
---

# Sort kanban columns by priority (default: high + assigned to me)

_Source: src/web/ (board.js, board.css, /api/issues)_

## Description

Allow sorting issues within each kanban column. Default ordering should put high-priority items first, with the current user's own assignments preferred at the top. Per-user preference is persisted (see related per-user view config feature).
