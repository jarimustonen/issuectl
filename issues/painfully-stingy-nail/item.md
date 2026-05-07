---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, reporting, schema]
---

# Lightweight estimates (size or numeric) + workload reports

## Description

Optional 'size: S/M/L/XL' or 'estimate: 3' frontmatter. 'issuectl workload' aggregates open + in-progress by assignee, priority, cycle, epic. 'issuectl burndown --cycle <name>' shows ASCII burndown. Tiny data, frontmatter-native; valuable when paired with cycles.
