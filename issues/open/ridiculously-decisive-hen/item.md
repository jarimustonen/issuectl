---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, maintenance]
---

# Stale issue detector + auto-archive of old closed issues

## Description

Two related lifecycle commands. 'issuectl stale [--days 30]' lists issues with no recent updates (frontmatter 'updated' + git log on the file), highlights long-running 'in-progress', flags issues assigned to inactive users. 'issuectl archive [--older-than 90d] [--dry-run]' moves closed issues to issues/archive/YYYY/MM/<slug>/ to keep the active tree small. All commands must understand both active and archive roots. Prevents repo rot.
