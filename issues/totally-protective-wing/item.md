---
created: 2026-05-10
updated: 2026-05-10
type: feature
reporter: jari
assignee: jari
status: in-progress
priority: normal
epic: hugely-exciting-spiders
commits:
- hash: e38559f
  summary: add issuectl init
---

# Add 'issuectl init' command that bootstraps schema, AGENTS.md, skills, hooks, and merge driver in one step

## Description

Currently first-time setup requires four to five separate commands (doctor --fix, agents init, skill install --agent all, hooks install, install-merge-driver --apply). Each is opt-in for a reason, but for the common 'I just adopted issuectl in a new repo' flow we want one command that runs the whole sequence with sensible defaults. Likely shape: 'issuectl init [--agent claude|codex|all] [--with-hooks] [--with-merge-driver]' with each step idempotent so re-running is safe. Should report what it created vs. what already existed (mirrors skill install's pattern). Open question: does init also bootstrap .schema.yaml directly, or rely on the existing doctor --fix path? Probably the latter — keeps one bootstrap source.
