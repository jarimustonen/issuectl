---
created: 2026-10-21
updated: 2026-10-21
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:oh-wrapup-20261021-ictl-laneorder
---

# dag: priority overrides lane_seq within a lane, so a parked high-priori…

## Description

dag: priority overrides lane_seq within a lane, so a parked high-priority head cannot be moved back

Observed 2026-10-21 in open-habitat. Lane `web` had `reconnect-live-cause` (priority high, waiting only for a new live occurrence) at its head, blocking five normal-priority issues. `issuectl update reconnect-live-cause --lane-seq 99 --json` succeeded, but `issuectl dag --json --reservations '[]'` still listed it at position 0 / head_of_line, apparently because ordering sorts by priority before lane_seq. Workaround: move it to another lane (`--lane unlaned`).

Expected: within a lane, explicit `lane_seq` defines order (priority at most a tie-breaker), or the docs/`dag` output state the ordering rule and `update --lane-seq` warns when it has no effect on position. Also minor: `update` has no `--preparation` flag (needs `--field preparation=ready`) while dag reports `preparation_reason: preparation_missing`; a first-class flag or a hint in the reason would help.
