---
created: 2026-05-10
updated: 2026-05-10
type: feature
reporter: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [from-3dbear-0.5.1-feedback]
---

# doctor: status_aliases and type_aliases for auto-coercing legacy values during migration

## Description

Common legacy values rejected by 0.5.1 schema: status closed/resolved/in_progress/paused/blocked, type enhancement/refactor. User mapped them by hand-script. Fix: add status_aliases / type_aliases maps to schema (built-in defaults plus user overrides), and let --fix auto-rewrite. Would have saved ~half of one team's migration work. See @intensely-ill-garden for full feedback context (3DBear monorepo 0.3.1 → 0.5.1 migration, 2026-05-10).
