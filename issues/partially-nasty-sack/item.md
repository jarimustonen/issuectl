---
created: 2026-05-06
updated: 2026-05-06
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [kanban, v0.6.0-candidate, workflow]
---

# WIP-limit warnings per kanban column

## Description

Board config (per board, see @genuinely-cloistered-current) defines soft caps per status: wip_limits: {in-progress: 3, testing: 2}. CLI warns on transitions that exceed the cap; web board colors overloaded columns red but never blocks. No server-side enforcement needed for a local tool — visibility is enough.
