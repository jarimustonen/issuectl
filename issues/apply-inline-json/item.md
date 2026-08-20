---
created: 2026-08-20
updated: 2026-08-20
type: feature
status: duplicate
priority: low
closed: 2026-08-20
closed_by: agent-triage
---

# apply: accept an inline JSON patch, not only a file path

## Description

`issuectl apply` takes `<PATCH>` as a **file path only**. Passing the patch inline — the
obvious first attempt for an agent building a one-shot mutation — fails with a message that
reads as a missing file rather than as "inline JSON is not supported":

```console
$ issuectl --json apply '{"slug":"artifact-host-core-extract","add_blocked_by":["cli-module-split"]}'
cannot read patch file {"slug":"artifact-host-core-extract","add_blocked_by":["cli-module-split"]}: No such file or directory (os error 2)
```

**Observed:** the JSON string is interpreted as a filename.
**Expected:** either the patch is applied, or the error says inline JSON is unsupported and
names the file-path form.

## Suggested fix

When the argument starts with `{` (after trimming), parse it as an inline JSON patch instead
of a path. That is unambiguous — no real filename starts with `{` — and it removes a temp-file
round trip from every scripted single-field mutation. Reading from stdin via `-` would serve
the same need if that fits the CLI canon better.

## Why it matters

An agent adding one dependency currently has to: write a temp file, discover that `--json`
additionally requires `expected_version`, fetch the version with `show --json`, rewrite the
temp file, and only then apply. The `expected_version` requirement is **fine** — its error
message states exactly what to do and the optimistic-concurrency design is deliberate. The
temp file is the only avoidable step.

Low priority: the workaround is one line of shell, and the current error message does at least
name the file it tried to read.

## Provenance

Hit 2026-08-20 while wiring a `blocked_by` edge between two issues in the `glasspad` repo
during a stint handoff. Filed from that session.

## Resolution

### 2026-08-20T12:15:51Z · @agent-triage

Duplicate of @apply-patch-from-stdin, which asks for the same one-step patch input (stdin instead of a temp file) and is already laned in verb-surface. The inline-`{` suggestion and the misleading "cannot read patch file" error message have been folded into that issue as part of its scope. Reopen only if stdin support lands while an inline argument still fails with the file-not-found wording.
