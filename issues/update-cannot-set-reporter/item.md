---
created: 2026-10-12
updated: 2026-10-12
type: bug
reporter: claude
status: open
priority: normal
provenance: agent
provenance_detail: native-agent-host wrap-up 2026-10-12
lane: mutation
---

# update cannot set the built-in reporter field

## Description

`issuectl update <slug> --field reporter=jari` fails with `custom field "reporter" is built-in: --reporter`, but `issuectl update <slug> --reporter jari` fails with `unexpected argument '--reporter'`; only `--no-reporter` exists. `create` has no reporter flag either, so there is no CLI path to set the reporter after creation outside `intake file`.

Expected: `update --reporter <name>` and `create --reporter <name>`.
