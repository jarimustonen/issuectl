---
created: 2026-10-01
updated: 2026-10-01
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:nah-2026-10-01-unlaned-unscheduled
---

# lane: unlaned lands in unscheduled despite dag --help

## Description

lane: unlaned lands in unscheduled despite dag --help

issuectl 0.18.7 in native-agent-host (2026-10-01). `issuectl dag --help` says: "`lane: unlaned` means confirmed parallel-safe work: every member is independently headed and spawnable. It differs from an absent lane, which means unclassified work."

Observed: `issuectl update design-package-cross-device --lane unlaned` (status open, no collision; also tried --lane-seq 1) → `issuectl dag --json --reservations '[]'` lists the slug in `.data.unscheduled` and there is no `unlaned` entry in `.data.lanes[]`. The wrap-up helper `list-unlaned-issues.sh` therefore still reports it as outside the DAG.

Expected: an `unlaned` lane row in `.data.lanes[]` with the issue independently headed and spawnable, per the help text; or, if `unlaned` was retired, `update --lane unlaned` should be rejected and the help corrected.
