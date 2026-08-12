---
created: 2026-08-11
updated: 2026-08-12
type: bug
status: in-progress
priority: low
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
