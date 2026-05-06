---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, interop]
---

# Import / export: GitHub Issues, JSON, CSV, markdown

## Description

Adoption pragmatism. 'issuectl import github --repo owner/name --state open' (uses gh CLI). 'issuectl import json <file>'. 'issuectl export json' / 'export markdown' / 'export csv'. Helps teams start from existing trackers, then continue locally. Risk: importer complexity balloons — start with JSON/CSV, add GitHub second, anything else only if requested.
