---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, visualization]
---

# Dependency graph visualization (Mermaid / SVG / web)

## Description

'issuectl graph [--format mermaid|dot|svg]' renders blockers/related/epic relationships from frontmatter. Web board: clicking a blocked card shows a mini dependency tree. Mermaid output is paste-ready for markdown docs. Builds on @uncommonly-cooing-badge (canonical blocked_by) and the existing related: field.
