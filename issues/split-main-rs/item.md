---
created: 2026-08-17
updated: 2026-08-17
type: chore
status: open
priority: high
lane: cli-fixes
lane_seq: 5
---

# Split main.rs: it is the scheduling bottleneck, not just a big file

## Description

## Why this is a scheduling issue, not a cleanliness one

`crates/issuectl/src/main.rs` is **9278 lines** holding **58 `cmd_*` handlers** plus the single
`enum Command` with every clap struct. Because two worktrees editing it collide on rebase, it is
listed as a hot file in `AGENTS.md` — which means every issue touching it lands in **one serial
lane**.

Measured cost, from the 2026-08-17 stint: **7 of the 7 issues in that round touched `main.rs`**,
so the entire session ran serially. The first wave managed three lanes in parallel only because
those three units happened to avoid the file; everything after it was a queue. Twice we batched
two small text-only units into one worker to save a serial slot — that helped, but it is a
workaround, not a fix.

`docs/design/lane-design.md` (landed in the same round) states the general principle this repo is
now the worked example of:

> A hot file that collects many issues is a scheduling problem. Splitting the file is the
> highest-leverage scheduling move available, not a cosmetic refactor.

`issuectl dag` now reports per-lane depth and the spawnable-head count, so the cost is directly
observable: while `cli-fixes` is N deep, this repo's parallelism budget is N-times worse than the
issue set actually requires.

## Ordered at the head of the lane deliberately

This sits at `lane_seq: 5`, ahead of the other `cli-fixes` work, because every later unit in the
lane benefits: once the handlers live in separate modules, issues touching different commands stop
colliding and can run in parallel lanes instead of queueing behind each other. Doing it after the
queue drains wastes the benefit on an empty lane.

## Shape of the split (proposal, not a mandate)

`AGENTS.md` already constrains the design, and this must not violate it:

- The binary crate owns **only** clap structs, `find_root`, the `cmd_*` handlers, and `fn main`.
  Domain logic stays in `issuectl-core`; do **not** move behaviour across the crate boundary as
  part of this.
- `cmd_*` handlers stay thin (argument parsing + JSON/human formatting, ~30 lines target). If a
  handler is fat, the fix is to move logic into a core domain module, not into a new bin module.
- Every write path still routes through `issuectl-core/src/mutate/`.

Suggested grouping — one module per command family under `crates/issuectl/src/cmd/`, e.g.
`cmd/issue.rs` (create/update/show/close/note), `cmd/query.rs` (ls/search/dag/report),
`cmd/skill.rs`, `cmd/doctor.rs`, `cmd/intake.rs`, `cmd/config.rs`, with the clap `Command` enum
either staying in `main.rs` or split into per-family subcommand enums. **The clap enum is the part
that decides whether the collision actually goes away** — if every command's flags still live in
one file, the hot file just moved. Decide that explicitly rather than only moving function bodies.

## Acceptance

- `main.rs` no longer holds the bulk of the `cmd_*` handlers.
- Two issues touching *different* command families can be worked in parallel without a rebase
  collision — i.e. `AGENTS.md`'s hot-file list can be narrowed, and it is updated in the same
  change to say what is now safe to parallelise.
- No behaviour change: this is a pure move. The full green gate passes
  (`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `cargo build --workspace`,
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`).
- No `--json` output or CLI surface change, so no skill-template update should be needed. If one
  turns out to be needed, that is a signal the move was not behaviour-neutral.

## Risk

This is the maximally-colliding change in the repo: it must run **alone**, with no other
`main.rs` worktree live, and it will invalidate any in-flight branch that touches the file. Land
it when the lane is otherwise idle.
