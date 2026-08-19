---
created: 2026-08-17
updated: 2026-08-19
type: feature
reporter: jari
status: in-progress
priority: normal
labels:
- via:agent-homebase-wrapup
lane: skills
lane_seq: 30
collision: [templates]
---

# issue skill: document update --lane/--lane-seq flags

## Description

issue skill: document update --lane/--lane-seq flags

Observed: the bundled `issue` skill (SKILL.md, installed by issuectl 0.6.3) documents `issuectl update` "Common flags" without mentioning `--lane <name>` / `--lane-seq <int>` / `--no-lane` / `--no-lane-seq`, and its `set` action docs do not say that `lane`/`lane_seq` are built-in fields rejected by `set`. An agent following the skill tried `issuectl set <slug> lane <value>` and got `validation: custom field "lane" is built-in: use update --lane <name> / --no-lane` (the error message is good and self-correcting, but the skill should teach the right call up front, since the execution-DAG workflow makes laning a very common operation).

Expected: the Update action's "Common flags" list includes `--lane` / `--lane-seq` (and their `--no-*` clearing forms), and ideally a one-line pointer that lane membership drives `issuectl dag`.

Exact failing command: `issuectl --json set cut-plan-module lane core --expected-version sha256:...` → exit 1, code command-failed.

## Comments

### 2026-08-17T17:16:21Z · @agent-stint

Triage: accepted. Skill-template docs gap — /issue skill's Update flags list must include --lane/--lane-seq/--no-lane/--no-lane-seq and note that set rejects built-in lane fields. Laned to skills (seq 30, after the two running skills fixes; collision: templates).
