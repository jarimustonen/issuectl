---
created: 2026-05-06
updated: 2026-08-04
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [kanban, v0.6.0-candidate, web-ui, deferred]
---

# Customizable kanban card fields + color coding by priority/label

## Description

Per-user / per-board config for which frontmatter keys appear on a card (priority badge, labels, assignee, due, estimate). Color coding: high-priority cards get a red left-border, labels render as colored chips driven by .issuectl/labels.yaml palette. Builds on @almost-homely-decision (per-user view) and @genuinely-cloistered-current (multi-board).
