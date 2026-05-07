---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, agent-friendly, schema]
---

# Schema-driven agent instructions (custom fields → context bundle constraints)

## Description

Builds on the schema file (@singularly-hulking-crown) and context bundle (@profoundly-domineering-wound). When the user defines custom fields or constraints in the schema (e.g. 'estimate must be an integer S/M/L/XL', 'labels must be from this enum'), 'issuectl context <slug>' auto-injects those constraints as system instructions in the rendered prompt. Agents then know the rules without the user re-stating them in every prompt.
