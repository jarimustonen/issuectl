---
created: 2026-08-17
updated: 2026-08-19
type: bug
reporter: jari
status: in-progress
priority: normal
labels:
- via:agent-homebase-wrapup
lane: verb-surface
lane_seq: 3
collision: [crates/issuectl/src/cmd/write.rs]
commits:
- hash: b2c4878
  summary: mark update JSON echo fix in progress
- hash: 782e1cc
  summary: echo updated scheduling fields in JSON
---

# update response envelope echoes null for fields it just set (lane_seq)

## Description

update response envelope echoes null for fields it just set (lane_seq)

Observed (issuectl current as of 2026-08-17, orchestratectl repo): `issuectl --json update <slug> --lane-seq 1` succeeded but the response envelope reported `"lane_seq": null`. A follow-up `issuectl --json show <slug>` returned the correct persisted value (1). Reproduced twice in a row on two different issues (`stint-skills-drop-intake-specifics`, `stint-skills-issuectl-dag`).

Expected: the update response's `.data` reflects the post-update issue state for the fields the call just set, so a caller can verify the write from the response alone instead of issuing a second `show`.

Impact: low (the write itself is correct), but the misleading envelope sent this agent on a false-alarm verification loop.

## Comments

### 2026-08-17T17:25:54Z · @agent-stint

Triage: confirmed in code — cmd_update's --json echo only reports status/priority/labels via echo_mutated_fields; lane/lane_seq/collision set by the same call are not echoed from UpdateOutcome (create's lane echo already does this correctly under the write lock — follow that pattern). Laned verb-surface seq 3, ahead of update-canonical-forms which reworks the same echo surface (collision: cmd/write.rs).
