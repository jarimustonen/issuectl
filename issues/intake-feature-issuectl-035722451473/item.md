---
created: 2026-08-14
updated: 2026-08-14
type: feature
reporter: jari
status: open
priority: normal
labels:
- via:agent-homebase-wrapup
- needs-triage
---

# issuectl new: accept --lane (set scheduling lane at creation)

## Description

issuectl new: accept --lane (set scheduling lane at creation)

issuectl new has no --lane flag, so setting a lane at creation needs two calls:
`issuectl new … --slug X` then `issuectl update X --lane <lane>`. Observed repeatedly while
laning a backlog sweep (ADR-0010 cleanup) — every new issue that should start scheduled took
a follow-up update. Expected: `issuectl new --lane <lane>` (and ideally --lane-seq) so a new
issue can be born into the DAG in one call. `issuectl update` already has --lane; mirror it on new.
