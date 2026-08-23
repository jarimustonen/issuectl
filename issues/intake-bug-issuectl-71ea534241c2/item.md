---
created: 2026-08-22
updated: 2026-08-23
type: bug
reporter: jari
status: fixed
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:homebase-wrapup-issuectl-create-path-field-20260821
lane: skills
related: ['@intake-bug-issuectl-704cd8eb0a0e']
commits:
- hash: 95740050093c0affe4b5b6fdbedc795aaa013214
  summary: fix shipped create path and echo guidance
- hash: dddba8c8f37c90b9d898f3130910986c4dffbf4e
  summary: harden reviewed skill contract coverage
- hash: 4535d77
  summary: assess multi-model skill review
closed: 2026-08-23
---

# Issue skill reads the wrong create result path field

## Description

Issue skill reads the wrong create result path field

## Observed

The installed `/issue` skill documents the `issuectl --json create` success payload with an `item_path` field and instructs agents to read `.data.item_path` to locate the created issue file.

During a Homebase stint, this command succeeded:

```sh
issuectl --json create --type bug --title "Rust trial reports drift for an all-no-op plan" --slug rust-trial-false-drift --body-file "$body"
```

The actual success envelope returned `.data.path` (plus `.data.dir`), not `.data.item_path`. Following the bundled skill literally produced:

```text
fatal: pathspec 'null' did not match any files
```

The issue itself had already been created successfully, so this also creates a retry/duplicate hazard for agents that mistake the follow-up failure for a failed create.

## Expected

The bundled skill and the CLI's actual schema should use the same canonical field. Prefer updating all examples and extraction instructions to `.data.path` if that is the supported shared vocabulary, or restore `item_path` consistently if it is the intended contract. Add a skill/example contract test so binary and bundled prose cannot drift.

## Environment

- issuectl 0.16.0
- installed `/issue` skill also declares issuectl 0.16.0
- observed 2026-08-21 in Homebase

## Decisions

### 2026-08-22T18:50:04Z · @agent-triage

Accepted scope also includes clarifying the update echo wording: only lane, lane_seq, and collision are echoed conditionally; blocked_by callers read the canonical value with show or dag. This folds the useful documentation correction from @intake-bug-issuectl-704cd8eb0a0e into this single skill-contract unit without adding a second implementation issue.

## Resolution

### 2026-08-23T19:03:47Z · @issuectl

Shipped and dogfooded guidance now uses .data.path, documents conditional scheduling echoes and blocked_by reads, and is pinned by contract tests. Full workspace green gate passed.
