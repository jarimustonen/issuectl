---
created: 2026-05-06
updated: 2026-05-31
type: feature
status: done
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [reporting, schema, v0.6.0-candidate]
closed: 2026-05-31
---

# Lightweight estimates (size or numeric) + workload reports

## Description

Optional 'size: S/M/L/XL' or 'estimate: 3' frontmatter. 'issuectl workload' aggregates open + in-progress by assignee, priority, cycle, epic. 'issuectl burndown --cycle <name>' shows ASCII burndown. Tiny data, frontmatter-native; valuable when paired with cycles.
