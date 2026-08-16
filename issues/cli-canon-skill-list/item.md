---
created: 2026-08-16
updated: 2026-08-16
type: improvement
status: open
priority: normal
labels: [cli-canon, tooling]
lane: cli-canon
lane_seq: 40
---

# cli-canon: §15 skill list subcommand

## Description


Filed by the `stack-cli-alignment` CLI-surface normalisation (homebase epic), phase 1.
Source: homebase `issues/cli-alignment-audit/analysis.md` (2026-08-10 audit) + live
re-verification 2026-08-16. Canon: `AGENTS-AI-FIRST-CLI.md`. This is a **fix** issue
(the audit + review only recommend); laned in `cli-canon` for a future `/stint-start`.

**Gap (§15) — `skill install`/`print` exist but no `skill list`.**

**Do:** add `skill list` (enumerate the companion skills the CLI can install), completing the
`list/install/print` triad to match ossctl/orchestratectl.

**Current state (evidence):** `skill install`/`print` exist, no `skill list`.
