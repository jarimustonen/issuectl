---
created: 2026-08-17
updated: 2026-08-20
type: bug
reporter: jari
status: fixed
priority: normal
closed: 2026-08-17
lane: help-docs
lane_seq: 20
collision: [crates/issuectl/src/cmd/views.rs]
commits:
- hash: 3cfbc3b
  summary: surface deliberate DAG intra-lane ordering
provenance: agent-homebase-wrapup
---

# dag lane ordering: priority silently outranks lane_seq within a lane

## Description

dag lane ordering: priority silently outranks lane_seq within a lane

Observed (ossctl repo, 2026-08-17): in lane contract-engine, issue A has lane_seq 20 / priority normal and issue B has lane_seq 30 / priority high. Both `issuectl dag` and `issuectl dag --json` render B BEFORE A. Expected: lane_seq is the documented intra-lane ordering mechanism, so seq 20 should precede seq 30 regardless of priority — or, if priority-first is intended, the dag output/docs should say so (the surprise is the silent precedence, not the policy). Repro: two issues in one lane, lower seq + normal prio vs higher seq + high prio; compare `issuectl dag --json` lane order to the lane_seq values.

## Comments

### 2026-08-17T17:13:13Z · @agent-stint

Triage: the precedence IS deliberate and documented — dag.rs states intra-lane order is topological on blocked_by, then priority, then lane_seq, and update/create --lane-seq help says 'consulted after blocked_by and priority'. So this is accepted as a docs/UX gap, not a reordering bug: surface the ordering policy in dag --help and the dag output/lane-design doc so the precedence is not silent. Do NOT change the ordering.
