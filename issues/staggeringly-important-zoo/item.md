---
created: 2026-05-10
updated: 2026-05-10
type: improvement
reporter: jari
status: in-progress
priority: high
epic: hugely-exciting-spiders
labels: [from-3dbear-0.5.1-feedback]
---

# doctor --fix is all-or-nothing: refuses to run while any schema violation exists

## Description

Layout migration is the safest mechanical operation, but is gated behind every other violation being clean. In a 240-issue migration this forces hours of manual cleanup against the pre-migration layout. Fix options: --fix-layout-only, staged --phase=layout|schema, or --fix --force that does what it can and reports the rest. See @intensely-ill-garden for full feedback context (3DBear monorepo 0.3.1 → 0.5.1 migration, 2026-05-10).
