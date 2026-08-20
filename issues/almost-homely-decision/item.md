---
created: 2026-05-06
updated: 2026-08-20
type: feature
status: obsolete
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
related: ['@truly-somber-payment']
labels: [kanban, web-ui]
closed: 2026-08-10
---

# Per-user kanban view config that remembers last state

_Source: src/web/ (board.js + new persistence layer)_

## Description

Default kanban view should be configurable per user and reopen in the state where the user left it (filters, sort order, scroll/expansion, theme, etc.). Open question: where to persist — browser localStorage only vs. a server-side per-user file under issues/ or .issuectl/.

## Resolution

### 2026-08-10T10:03:40Z · @issuectl

Web/browser UI is being removed from issuectl (product decision 2026-08-10). This is a web-board enhancement, so it is obsolete. See @remove-web-ui.
