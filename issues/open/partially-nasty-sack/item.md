---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, kanban, workflow]
---

# WIP-limit warnings per kanban column

## Description

Board config (per board, see @genuinely-cloistered-current) defines soft caps per status: wip_limits: {in-progress: 3, testing: 2}. CLI warns on transitions that exceed the cap; web board colors overloaded columns red but never blocks. No server-side enforcement needed for a local tool — visibility is enough.
