---
created: 2026-10-12
updated: 2026-10-16
type: bug
reporter: claude
status: fixed
priority: normal
provenance: agent
provenance_detail: native-agent-host wrap-up 2026-10-12
lane: mutation
closed: 2026-10-16
closed_by: pi
---

# update cannot set the built-in reporter field

## Description

`issuectl update <slug> --field reporter=jari` fails with `custom field "reporter" is built-in: --reporter`, but `issuectl update <slug> --reporter jari` fails with `unexpected argument '--reporter'`; only `--no-reporter` exists. `create` has no reporter flag either, so there is no CLI path to set the reporter after creation outside `intake file`.

Expected: `update --reporter <name>` and `create --reporter <name>`.

## Verification

The update flag was missing as reported. The report's claim about creation
was incorrect: `create --reporter` already worked and is covered by regression
tests. Core reporter patches also already existed. The fix wires the canonical
flag into that request and rejects explicit reporter assignments to existing
epics, which use owner instead; conversion to epic still migrates a lone reporter.

## Acceptance Criteria

- [x] `update --reporter NAME` sets reporter through core validation; existing `create --reporter` remains supported.
- [x] Setting and clearing conflict; empty names and built-in custom-field bypasses are rejected.
- [x] Epic owner rules and reporter-to-owner type conversion are preserved across single and patch updates.
- [x] Optimistic version checks, JSON success/error envelopes, query dry runs and bulk writes are tested.
- [x] Reporter set/clear edits cannot use `--preserve-updated`, including mixed custom-field requests and patches.
- [x] Consumer skill templates, Codex pair and dogfooded copies describe the flag; Unreleased notes record the fix.
