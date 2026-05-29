---
created: 2026-05-06
updated: 2026-05-29
type: feature
status: done
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [interop, v0.6.0-candidate]
closed: 2026-05-29
---

# Import / export: GitHub Issues, JSON, CSV, markdown

## Description

Adoption pragmatism. 'issuectl import github --repo owner/name --state open' (uses gh CLI). 'issuectl import json <file>'. 'issuectl export json' / 'export markdown' / 'export csv'. Helps teams start from existing trackers, then continue locally. Risk: importer complexity balloons — start with JSON/CSV, add GitHub second, anything else only if requested.
