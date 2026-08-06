---
created: 2026-08-06
updated: 2026-08-06
type: task
status: open
priority: normal
---

# Verify intake queue surfaces legacy label-encoded items (transitional split queue)

## Description

docs/design/intake-flow.md §6 'transitional split queue' specifies that `issuectl intake queue` should also surface recognised legacy forms (open + needs-triage/deferred labels, via:telegram) with a `legacy: true` flag + a 'run intake migration' nudge, until a repo reports migration-complete — so the queue never silently abandons un-migrated items. Verify this shipped in 0.7.0; if not, implement it. It is the safety net for the homebase/deutschpad `issuectl intake migrate` runs.
