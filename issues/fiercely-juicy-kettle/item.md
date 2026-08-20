---
created: 2026-05-06
updated: 2026-08-20
type: feature
status: obsolete
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
related: ['@excessively-beneficial-owner']
labels: [kanban, web-ui]
closed: 2026-08-10
---

# Copy-to-clipboard buttons on kanban cards (issue slug + prompt text)

_Source: src/web/ (board.js, board.css)_

## Description

Add small copy buttons on each card to copy useful values to the clipboard: at minimum the issue slug (or @slug reference) and ideally a ready-made prompt string composed from the issue (title + description, suitable for pasting into Claude Code). Uses the browser Clipboard API; falls back gracefully on non-secure contexts. Related to the 'start implementation' investigation — same prompt-shaping logic could power both.

## Resolution

### 2026-08-10T10:03:40Z · @issuectl

Web/browser UI is being removed from issuectl (product decision 2026-08-10). This is a web-board enhancement, so it is obsolete. See @remove-web-ui.
