# Implement: {{title}}

You are implementing issue `@{{slug}}` ({{type}}, status: {{status}},
priority: {{priority}}).

## Issue body

{{body}}

## Acceptance criteria

{{acceptance_criteria}}

## Parent epic

- slug: @{{epic_slug}}
- title: {{epic_title}}
- goal: {{epic_goal}}

## Related

{{related}}

## Blocked by

{{blocked_by}}

## Recorded commits

{{commits}}

---

When done, record commits with `issuectl --json update {{slug}}
--add-commit "HASH:summary" --expected-version <ver>` and close with
`issuectl --json close {{slug}} --status done --expected-version <ver>`.
