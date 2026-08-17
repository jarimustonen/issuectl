---
created: 2026-08-17
updated: 2026-08-17
type: bug
reporter: jari
status: open
priority: normal
labels:
- via:agent-homebase-wrapup
- needs-triage
---

# dag lane ordering: priority silently outranks lane_seq within a lane

## Description

dag lane ordering: priority silently outranks lane_seq within a lane

Observed (ossctl repo, 2026-08-17): in lane contract-engine, issue A has lane_seq 20 / priority normal and issue B has lane_seq 30 / priority high. Both `issuectl dag` and `issuectl dag --json` render B BEFORE A. Expected: lane_seq is the documented intra-lane ordering mechanism, so seq 20 should precede seq 30 regardless of priority — or, if priority-first is intended, the dag output/docs should say so (the surprise is the silent precedence, not the policy). Repro: two issues in one lane, lower seq + normal prio vs higher seq + high prio; compare `issuectl dag --json` lane order to the lane_seq values.
