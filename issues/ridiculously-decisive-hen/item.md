---
created: 2026-05-06
updated: 2026-05-06
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [maintenance, v0.6.0-candidate]
---

# Stale issue detector + auto-archive of old closed issues

## Description

Two related lifecycle commands. 'issuectl stale [--days 30]' lists issues with no recent updates (frontmatter 'updated' + git log on the file), highlights long-running 'in-progress', flags issues assigned to inactive users. 'issuectl archive [--older-than 90d] [--dry-run]' moves closed issues to issues/archive/YYYY/MM/<slug>/ to keep the active tree small. All commands must understand both active and archive roots. Prevents repo rot.
