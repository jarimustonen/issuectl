---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, cli]
---

# Bulk operations: issuectl bulk '<query>' --add-label / --set / ...

## Description

'issuectl bulk <query> --add-label X --remove-label Y --set status=done' applies a mutation to every issue matching the query. Builds on the shared query engine (@unusually-elegant-rule). Always supports --dry-run that prints affected slugs + diff. File-based fit: bulk changes are just one git commit touching many markdown files.
