---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [reporting, git-native]
---

# Git-derived activity / timeline / changelog + lightweight metrics

_Source: src/cli/{activity,timeline,changelog,metrics}.rs (new)_

## Description

Several reporting commands, one issue. 'issuectl activity [--since 7d]' lists recent issue file changes from git log. 'issuectl timeline <slug>' shows status transitions reconstructed from git history. 'issuectl changelog <ref>..<ref>' generates markdown release notes by cross-referencing closed issues with linked commits in the range, grouped by type/label. 'issuectl metrics [--since 30d]' computes cycle time, throughput, workload by assignee. All offline, no event database — git is the event log. Caveat: rebases/squashes can rewrite history; document that frontmatter timestamps are authoritative when available.
