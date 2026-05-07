---
created: 2026-05-06
updated: 2026-05-06
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, workflow]
---

# Issue-local attachments and fixtures directories

## Description

First-class support for issues/<slug>/attachments/ (screenshots, logs) and issues/<slug>/fixtures/ (reproduction files for bugs — agents can run against them while fixing). Markdown body references via relative paths. Web detail view renders attachments inline. Doctor warns on huge binaries (suggest external storage or .gitignore). Existing convention is for AVIF only — extend to other formats with size warnings.
