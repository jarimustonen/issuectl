---
created: 2026-05-10
updated: 2026-05-10
type: chore
status: done
priority: normal
epic: exorbitantly-ill-apples
commits:
- hash: ace53b2
  summary: 'feat(doctor): add stop_phase discriminator to apply_outcome envelope'
- hash: ee7375f
  summary: 'fix(doctor): apply review fixes to stop_phase envelope'
closed: 2026-05-10
---

# Doctor: split ApplyOutcome.blockers into preflight + post-apply, update JSON envelope and skill templates

## Description

Spin-off from @slightly-hellish-airport round-1 /llm-review (OpenAI, Anthropic).

## Problem

After @slightly-hellish-airport, `ApplyOutcome.blockers` is populated
in two distinct phases — preflight (no writes attempted) AND
post-flat-layout safety re-check (writes already landed). The doc
comment was updated to flag this, but the JSON envelope still
exposes a single `blockers` field, the field NAME still implies
preflight semantics, and `templates/issue-skill.md` /
`templates/issue-prompt.md` were not updated.

`fix_applied: true && blockers != []` is now a valid combination,
which contradicts the previous documented contract that blockers ⇒
no writes.

## Why this needs its own design

Multiple shapes are defensible (split fields, `stop_phase` discriminator,
phased-blockers struct). All change the JSON contract and require a
matching skill-template update in the same commit per AGENTS.md.
This is an output-contract redesign — separate from the safety-check
fix that surfaced the issue.

## Acceptance criteria

- `apply_outcome` in `--json --fix` clearly distinguishes preflight
  blockers from post-apply blockers OR carries an explicit
  `stop_phase` field.
- `templates/issue-skill.md` and `templates/issue-prompt.md`
  document the new shape and the partial-progress combination.
- AGENTS.md note on doctor `--fix` forward-progress is updated to
  match.
- Tests cover the preflight-only, post-apply-only, and clean-success
  envelopes.

## Context

- Round-1 /llm-review on @slightly-hellish-airport (OpenAI, Anthropic)
- @greatly-flat-sleet (apply pipeline refactor)
- @slightly-hellish-airport (post-migration blocker re-check)
