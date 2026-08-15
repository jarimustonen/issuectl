---
created: 2026-08-14
updated: 2026-08-15
type: feature
reporter: jari
status: in-progress
priority: normal
labels:
- via:agent-homebase-wrapup
lane: cli-fixes
lane_seq: 30
collision: [crates/issuectl/src/main.rs]
---

# issuectl update: add --blocked-by / --add-blocked-by (edit blocked_by v…

## Description

issuectl update: add --blocked-by / --add-blocked-by (edit blocked_by via CLI)

issuectl update exposes --lane, --add-related, --add-collision, --add-label, but NOTHING for
`blocked_by`. To gate a lane head behind another issue I had to hand-edit the item.md
frontmatter `blocked_by: ['@slug']` directly — error-prone and bypasses validation. Expected:
`issuectl update <slug> --add-blocked-by @<slug>` / `--remove-blocked-by` (repeatable), matching
the existing --add-related shape, so DAG dependency edges can be set via the CLI like every
other schedulable field.
