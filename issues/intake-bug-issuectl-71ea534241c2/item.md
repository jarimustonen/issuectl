---
created: 2026-08-22
updated: 2026-08-22
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:homebase-wrapup-issuectl-create-path-field-20260821
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
