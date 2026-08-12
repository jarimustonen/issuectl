---
created: 2026-05-06
updated: 2026-08-12
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, visualization]
---

# Epic tree view in CLI

> **Rescoped 2026-08-10 (CLI-only).** The browser/web UI is being removed from
> issuectl (see `@remove-web-ui`), so the web-board tree view is dropped. Only
> the CLI command remains in scope.

## Description

Render epics with their child issues as a collapsible/indented tree in the CLI:
`issuectl epic tree <slug>` (and, without a slug, all epics). Today epics exist
but flat `list` output hides the hierarchy. Pairs with epic progress rollups.
