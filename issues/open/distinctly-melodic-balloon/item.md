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

# Duplicate detection (heuristic, local-only)

## Description

'issuectl duplicates' uses local heuristics — normalized title overlap, shared labels, similar body tokens — to flag potential duplicates. No embeddings, no remote AI. On 'issuectl new', optionally pre-check and prompt if a strong match exists. Important because random slugs make it hard for humans to spot duplicates.
