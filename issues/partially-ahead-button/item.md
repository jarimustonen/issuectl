---
created: 2026-05-08
updated: 2026-05-09
type: chore
reporter: jari
status: in-progress
priority: normal
epic: exorbitantly-ill-apples
labels: [release-v0.5.0]
commits:
- hash: 5aa3f3d
  summary: extract do_new_locked into src/write/new_issue.rs; rewire mutate
- hash: a1debda
  summary: round-2 layering fixes (refs.rs; mutate/new_issue; API dup-key rejection)
---

# Extract do_new_locked + NewArgs out of main.rs into domain module

## Description

Spin-off from @astoundingly-harsh-nest /llm-review (round 1, finding M8).

## Description

`do_new_locked`, `NewArgs`, `NewOutcome`, `claim_random_slug`, and
the new `DoNewError` enum all live in `src/main.rs` because the
original implementation grew up there as part of `cmd_new`. The
server-side `mutate::new_issue` now reaches into `crate::do_new_locked`
and `crate::DoNewError` from the binary entry module — API
correctness depends on a type owned by the CLI binary's root module.
Layering is backwards.

This is invisible to users today. It becomes a problem when the
next domain mutation gets added (a future `do_set_locked`,
`do_apply_locked`, …) and the natural-but-wrong path is to copy the
`main.rs` pattern instead of putting domain logic in `mutate.rs`.
Also makes the typed-error tests in `mutate.rs` reach across the
crate root via `crate::DoNewError`, which is jarring.

## Out of scope (already covered in v0.5.0 elsewhere)

- The verbs `set` / `note` / `check` / `label` / `apply` covered
  by @peculiarly-political-interest are already in `mutate.rs`,
  not `main.rs`. They do not need this refactor — they only need
  `MutateError` typed properly.

## Fix sketch

Move `do_new_locked` (~130 lines) + `claim_random_slug` (~25
lines) + `NewArgs` + `NewOutcome` + `DoNewError` into a domain
module — likely `src/write/new_issue.rs` or `src/domain/new_issue.rs`.
Decide whether `NewArgs` (CLI input) and `NewIssueRequest` (API
input from `mutate.rs`) collapse into a single canonical input
shape, or stay separate with a converter. Update both `cmd_new`
and `mutate::new_issue` to call the new module.

The diff is ~300 lines of relocation; deserves its own focused
review and own commit so the moves are not bundled with behavioral
changes.

## Definition of done

- `do_new_locked` and friends live outside `main.rs`.
- `mutate::new_issue` does not reference symbols at the crate root
  for new-issue creation.
- All existing tests still pass; tests-next-to-code convention
  preserved (the moved code carries its tests).
- AGENTS.md note added: "new domain mutations live in the domain
  module, not in `main.rs`" so the antipattern does not regrow.
