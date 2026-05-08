---
created: 2026-05-08
updated: 2026-05-08
type: task
reporter: jari
status: in-progress
priority: normal
epic: exorbitantly-ill-apples
commits:
- hash: 4f62aef
  summary: custom_fields support on UpdateIssueRequest + CLI --field/--clear-field
- hash: 149393c
  summary: centralize reserved-key list, tighten whitespace, address /llm-review fixes
---

# Add custom_fields to UpdateIssueRequest

## Description

`UpdateIssueRequest` accepts no arbitrary user-defined frontmatter
keys; `NewIssueRequest` does (`--field key=value`). With
`#[serde(deny_unknown_fields)]`, an API client cannot add, modify,
or remove a custom key on an existing issue. If the repo schema
adds a required custom field (e.g. `triage`, `reviewer`), older
issues become unrepairable through the API: every PATCH 422s with
`MutateError::SchemaViolation`, and the client has no way to set
the missing key.

Wire surface needs:
- `custom_fields: BTreeMap<String, Patch<String>>` (set / clear /
  unspecified) on `UpdateIssueRequest`.
- `update_issue_under_lock` plumbing to `set_string` /
  `remove_key` against the raw `Mapping`.
- Schema validation runs against the post-mutation frontmatter
  (already does — verify).
- CLI parity: `--field key=value` / `--unset-field key` on
  `issuectl update`.

Out of scope here: nested-map custom field updates (the wire
surface is flat string-valued for now).

Spun off from @painfully-endurable-steel review — required to land
in v0.5.0 because the schema work in @slightly-finicky-heart needs
this to repair pre-existing issues that fall out of compliance.
