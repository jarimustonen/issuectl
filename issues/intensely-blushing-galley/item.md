---
created: 2026-08-05
updated: 2026-08-06
type: improvement
status: in-progress
priority: low
commits:
- hash: 26e75b0535ea614d7a92c1258e168fddb06d1feb
  summary: promote closed_by to typed field, doctor heal, show/close output
---

# Promote closed_by to a typed Issue field + doctor heal

## Description

# Promote `closed_by` to a typed `Issue` field + doctor heal

_Follow-up from the llm-review on close-as-flag-asymmetry (the `close --as` feature)._

`close --as <author>` now records the closer as a `closed_by:` frontmatter
field, managed in lockstep with `closed:` (set on close, scrubbed on reopen,
reserved so it can't be planted via `set`/`update --field`). It surfaces in
`show --json` via `Issue::extra` and is version-hashed. That closed all the
correctness gaps the review found.

Deferred, non-blocking enhancements the reviewers raised:

- **Typed field.** `closed_by` lives in `Issue::extra` (stringly-typed) rather
  than a first-class `Issue.closed_by: Option<String>`. A typed field would let
  `show`'s human output display the closer and give typed consumers / SSE
  summaries direct access instead of an `extra` lookup.
- **`doctor` heal.** `doctor` heals `closed:` on closing-status issues but knows
  nothing about `closed_by`. It could warn on `closed_by` present when the
  status is non-closing (self-inconsistent legacy/hand-edited state).
- **Human `close` output.** `close` prints only "Closed <slug>"; it could echo
  "(by <closer>)" when `--as` was given.

## Severity
Low — ergonomic/structural polish. The feature is correct and complete without
these.
