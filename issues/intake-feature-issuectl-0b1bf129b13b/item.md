---
created: 2026-08-20
updated: 2026-08-20
type: feature
reporter: jari
status: open
priority: normal
labels:
- via:agent-homebase-wrapup
- needs-triage
lane: verb-surface
---

# issuectl apply cannot read a patch from stdin

## Description

issuectl apply cannot read a patch from stdin

## Observed

`issuectl apply` takes a patch file path only. Passing `-` (the usual convention for
stdin) is treated as a literal filename:

    $ issuectl apply --json - <<'PATCH'
    [ ... ]
    PATCH
    {"error":{"code":"command-failed","message":"cannot read patch file -: No such file or directory (os error 2)"},"schema_version":1}

`issuectl apply --help` confirms the contract is `Usage: issuectl apply [OPTIONS] <PATCH>`
with no stdin option documented.

## Expected

Either `-` reads the patch from stdin, or a `--stdin` flag does, so a patch can be
composed and applied in one step without a temp file.

## Why it is worth doing

Consistency inside the family, not novelty. `issuectl note` already has `--stdin`, and
`intakectl file` accepts `--body-file -`. `apply` is the one place where a multi-issue,
single-transaction edit is expressed, which is exactly the shape an agent generates
programmatically and pipes — so it is the command where the missing stdin path is felt
most.

## Impact

Low. The workaround (write a temp file, pass its path) is obvious and costs one line.
Filing it as a papercut, not a defect.

## Context

Hit 2026-08-20 while re-laning six issues in ossctl in one transaction during a stint
wrap. Fell back to six separate `issuectl update` calls.
