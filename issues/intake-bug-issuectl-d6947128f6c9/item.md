---
created: 2026-08-15
updated: 2026-08-15
type: bug
reporter: jari
status: open
priority: normal
labels:
- via:agent-homebase-wrapup
lane: cli-fixes
lane_seq: 10
collision: [crates/issuectl/src/main.rs]
---

# label: flag-style --remove silently no-ops with --json instead of error…

## Description

label: flag-style --remove silently no-ops with --json instead of erroring; OP is positional-only

## Observed
`issuectl label` takes the operation as a **positional** arg — `issuectl label <SLUG> <OP> <LABEL>` where OP ∈ {add, remove}. Two rough edges hit while removing a `needs-triage` label:

1. **Flag-style form fails unclearly.** `issuectl label <slug> --remove needs-triage` (mirroring the `--add/--remove` flag style most other issuectl subcommands use, e.g. `update`) exits with a bare `Usage:` error — it does not hint that OP is positional.

2. **`--json` swallows the arg error into a silent no-op.** The first attempt `issuectl label <slug> --remove needs-triage --json` produced **empty output** (no error envelope) and did **not** apply the change — it looked like a successful no-op. A subsequent `issuectl --json show <slug>` confirmed the label was still present. A malformed `label` invocation with `--json` should emit a JSON `error` envelope + non-zero exit, not empty stdout with the mutation silently skipped.

## Expected
- Either accept `--add/--remove <label>` flag aliases on `label` (consistent with `update`), or make the positional-only requirement explicit in a helpful error.
- With `--json`, an invalid-arguments failure must print a JSON error envelope and exit non-zero — never empty output while skipping the mutation.

## Repro
    issuectl label <slug> --remove needs-triage --json   # empty stdout, label NOT removed
    issuectl label <slug> --remove needs-triage           # bare "Usage:" error
    issuectl label <slug> remove needs-triage              # works (positional OP)

## Env
issuectl invoked from the glasspad repo, macOS (darwin 25.5.0), 2026-08-15.
