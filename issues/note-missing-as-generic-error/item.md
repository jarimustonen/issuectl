---
created: 2026-08-11
updated: 2026-08-12
type: bug
status: fixed
priority: low
commits:
- hash: c1a371ff0e8cc76e02c4113e9f758edf5c5b1be6
  summary: 'test(note): lock specific missing --as diagnostic (human + --json)'
closed: 2026-08-12
closed_by: agent-note-fix
---

# issuectl note without --as prints generic help instead of clap missing-arg error

_Source: crates/issuectl/src/main.rs_

## Description

Follow-up 'minor adjacent note' from @warn-reserved-notes-section.

## Observed
`issuectl note <slug> "..."` with `--as <AUTHOR>` omitted prints only:
```
For more information, try '--help'.
```

## Expected
clap's standard specific message, e.g.:
```
error: the following required arguments were not provided:
  --as <AUTHOR>
```

## Task
Confirm the specific missing-argument message isn't being suppressed (custom error handling / usage-error remap in `fn main`), and restore clap's standard rendering for the missing `--as` case.

## Resolution

### 2026-08-12T05:46:38Z · @agent-note-fix

Root cause: behaviour was already correct on HEAD. Human mode reaches clap's e.exit() (the missing-arg case has no routing hint, so nothing suppresses clap's detail), and the --json branch wraps clap's full message in the usage-error envelope (stderr, exit 1). The reported generic 'For more information, try --help.' output does not reproduce. Closed by adding regression coverage in tests/cli_papercuts.rs: strengthened note_missing_as_names_the_flag to a durable error:+--as guard, and added note_missing_as_json_emits_usage_error_envelope. Both reject the generic-help fallback. No production code change needed.
