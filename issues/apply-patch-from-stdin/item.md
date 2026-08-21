---
created: 2026-08-20
updated: 2026-08-21
type: feature
reporter: jari
status: done
priority: normal
lane: verb-surface
commits:
- hash: 97ae6ea
  summary: mark work in progress and clear intake labels
- hash: 485b001
  summary: add stdin transactional patch input and diagnostics
- hash: 9e75f69
  summary: record inline JSON decision and implementation commits
- hash: eace1dc
  summary: apply assessed review fixes and persist review artifacts
closed: 2026-08-21
---

# apply: accept a patch from stdin or inline JSON, not only a file path

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

## Comments

### 2026-08-20T12:15:47Z · @agent-triage

Folded in from the duplicate `apply-inline-json` (closed 2026-08-20), which reported the same
temp-file round trip from the inline-argument angle. Two extra asks come with it:

1. **Inline JSON.** An argument that starts with `{` after trimming is unambiguous — no real
   filename does — so it could be parsed as an inline patch directly. Optional: stdin alone
   already removes the temp file. Decide inline support as part of this issue rather than
   leaving it as a separate ticket.
2. **The error message is the real papercut.** Both reports hit
   `cannot read patch file <thing>: No such file or directory`, which reads as a missing file
   rather than "this form is not supported". Whatever input forms land, an unsupported
   argument must name the accepted forms (path, `-`, `--stdin`, and inline if adopted).

Original inline-JSON provenance: hit 2026-08-20 while wiring a `blocked_by` edge between two
issues in the `glasspad` repo during a stint handoff.

## Decisions

### 2026-08-21T08:15:13Z · @agent

Inline JSON argv was declined: stdin already provides temp-file-free composition while avoiding a second, shell-quoting-sensitive positional grammar. JSON remains accepted from both supported sources (a path or stdin). Bare unsupported tokens now name those accepted forms; path-shaped missing files retain the useful file diagnostic.
