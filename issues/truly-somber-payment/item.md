---
created: 2026-05-06
updated: 2026-08-20
type: feature
status: obsolete
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
related: ['@almost-homely-decision']
labels: [kanban, web-ui]
closed: 2026-08-10
---

# Sort kanban columns by priority (default: high + assigned to me)

_Source: src/web/ (board.js, board.css, /api/issues)_

## Description

Allow sorting issues within each kanban column. Default ordering should put high-priority items first, with the current user's own assignments preferred at the top. Per-user preference is persisted (see related per-user view config feature).

## Comments

### 2026-05-10T17:56:05Z · @jari

Refocus: not just 'sort by priority by default'. Generalize to a sort option that can be (a) declared per-board in .issuectl/boards/<name>.yaml (e.g. sort_by: [priority, -updated]), AND (b) overridden manually in the web UI via a sort dropdown. Same syntax in both places. Reuse the query-engine field set so any sortable field works (priority, updated, created, slug, etc.). Manual UI selection persists in localStorage per board so it survives reloads (overlaps with @almost-homely-decision — coordinate).

## Resolution

### 2026-08-10T10:03:40Z · @issuectl

Web/browser UI is being removed from issuectl (product decision 2026-08-10). This is a web-board enhancement, so it is obsolete. See @remove-web-ui.
