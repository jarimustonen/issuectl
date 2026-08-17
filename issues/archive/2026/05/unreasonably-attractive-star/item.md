---
created: 2026-05-10
updated: 2026-05-10
type: bug
reporter: jari
status: fixed
priority: high
epic: hugely-exciting-spiders
labels: [from-3dbear-0.5.1-feedback]
commits:
- hash: a8dbdd4
  summary: schema bootstrap moved before preflight refusal in apply()
- hash: eedf7cb
  summary: JSON envelope contract change documented in CHANGELOG; (preflight, fix_applied=true) state codified in stop_with_blockers invariant
closed: 2026-05-10
---

# doctor --fix does not create .schema.yaml when other violations block migration (broken promise)

## Description

Doctor prints 'Schema file missing at issues/.schema.yaml (will be auto-created on first --fix or write).' but the all-or-nothing fix policy aborts before reaching schema bootstrap. User has no way to ask doctor to 'just create the schema file'. Fix: bootstrap .schema.yaml unconditionally on --fix, regardless of other blockers. Or expose 'issuectl schema init' as explicit subcommand. See @intensely-ill-garden for full feedback context (3DBear monorepo 0.3.1 → 0.5.1 migration, 2026-05-10).
