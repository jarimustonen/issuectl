---
created: 2026-05-10
updated: 2026-05-10
type: improvement
reporter: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [from-3dbear-0.5.1-feedback]
---

# agents init: log which schema source was used (project vs. built-in defaults)

## Description

Running 'agents init' before .schema.yaml exists writes a managed block based on built-in defaults silently. Log 'Using built-in default schema (issues/.schema.yaml not found)' or 'Using project schema at issues/.schema.yaml' so user knows which inputs went in. See @intensely-ill-garden for full feedback context (3DBear monorepo 0.3.1 → 0.5.1 migration, 2026-05-10).
