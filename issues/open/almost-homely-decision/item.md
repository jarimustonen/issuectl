---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
labels: [kanban, web-ui]
related: ['@truly-somber-payment']
epic: hugely-exciting-spiders
---

# Per-user kanban view config that remembers last state

_Source: src/web/ (board.js + new persistence layer)_

## Description

Default kanban view should be configurable per user and reopen in the state where the user left it (filters, sort order, scroll/expansion, theme, etc.). Open question: where to persist — browser localStorage only vs. a server-side per-user file under issues/ or .issuectl/.
