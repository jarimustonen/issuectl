---
created: 2026-08-16
updated: 2026-08-16
type: chore
reporter: audit-bot
status: open
priority: high
lane: cli-fixes
---

# Replace maintainer-specific author example in main

## Description

The public-repository audit found a maintainer-specific author example in crates/issuectl/src/main.rs:4995. This worktree is intentionally not allowed to edit main.rs. Replace the example with an obviously fictional username, preserving the normalization behavior.
