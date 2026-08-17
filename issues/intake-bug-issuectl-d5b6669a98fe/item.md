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

# update response envelope echoes null for fields it just set (lane_seq)

## Description

update response envelope echoes null for fields it just set (lane_seq)

Observed (issuectl current as of 2026-08-17, orchestratectl repo): `issuectl --json update <slug> --lane-seq 1` succeeded but the response envelope reported `"lane_seq": null`. A follow-up `issuectl --json show <slug>` returned the correct persisted value (1). Reproduced twice in a row on two different issues (`stint-skills-drop-intake-specifics`, `stint-skills-issuectl-dag`).

Expected: the update response's `.data` reflects the post-update issue state for the fields the call just set, so a caller can verify the write from the response alone instead of issuing a second `show`.

Impact: low (the write itself is correct), but the misleading envelope sent this agent on a false-alarm verification loop.
