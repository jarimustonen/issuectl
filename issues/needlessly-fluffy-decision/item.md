---
created: 2026-05-06
updated: 2026-05-06
type: feature
status: open
priority: high
reporter: jari
assignee: jari
epic: exorbitantly-ill-apples
labels: [breaking, kanban, web-ui]
---

# Drag-and-drop issues between kanban columns with write-back to item.md

_Source: src/web/ (board.js + new POST/PATCH endpoints), src/cli/update.rs_

## Description

Today the board is read-only (GET routes only). Add drag-and-drop so users can move issues between columns, and have the change written back to the issue's frontmatter (status field, possibly moving to closed/). This is a significant scope change — read-only is currently a stated security/scope guarantee in the docs (issuectl docs kanban). Needs a writes-enabled mode (opt-in flag), CSRF-style guards, and care around concurrent edits via the CLI.
