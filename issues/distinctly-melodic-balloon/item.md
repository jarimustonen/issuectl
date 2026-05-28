---
created: 2026-05-06
updated: 2026-05-28
type: feature
status: done
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [maintenance, v0.6.0-candidate]
closed: 2026-05-28
---

# Duplicate detection (heuristic, local-only)

## Description

'issuectl duplicates' uses local heuristics — normalized title overlap, shared labels, similar body tokens — to flag potential duplicates. No embeddings, no remote AI. On 'issuectl new', optionally pre-check and prompt if a strong match exists. Important because random slugs make it hard for humans to spot duplicates.
