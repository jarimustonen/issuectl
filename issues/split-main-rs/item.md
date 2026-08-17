---
created: 2026-08-17
updated: 2026-08-17
type: chore
status: open
priority: high
lane: cli-fixes
lane_seq: 5
---

## Why this is a scheduling issue, not a cleanliness one

`crates/issuectl/src/main.rs` is **9278 lines** holding **58 `cmd_*` handlers** plus the single
`enum Command` with every clap struct. Because two worktrees editing it collide on rebase, it is
listed as a hot file in `AGENTS.md` — which means every issue touching it lands in **one serial
lane**. Measured cost, from the 2026-08-17 stint: 7 of 7 issues in that round touched `main.rs`,
so the entire session ran serially.

**Scope extended (2026-08-17, maintainer decision):** the same disease lives in two more files,
and this issue now covers all three:

| File | Lines | Impl / tests boundary |
|---|---|---|
| `crates/issuectl/src/main.rs` | 9278 | tests start ~7401 |
| `crates/issuectl-core/src/doctor.rs` | 7481 | tests start ~3902 |
| `crates/issuectl-core/src/mutate/mod.rs` | 7319 | tests start ~3209 |

`docs/design/lane-design.md` states the principle: a hot file that collects many issues is a
scheduling problem; splitting the file is the highest-leverage scheduling move available.

## Hard rules (read before touching anything)

- **PURE MOVE.** No behaviour change, no pub-item renames, no signature changes, no new
  dependencies, no new `#[allow]`s. If something beyond `use`-path fixes and
  visibility adjustments (`pub(crate)` where a helper crosses a new module boundary) seems
  required, stop and pick a different cut point instead of changing code.
- **External paths stay valid via re-exports.** After each phase, `issuectl_core::doctor::X`
  and `issuectl_core::mutate::X` must resolve exactly as before (`pub use` in the new
  `mod.rs`). Call sites outside the moved files must not need edits (updating `use` lines
  inside `crates/issuectl/src/` is fine).
- **No CLI surface change.** Before starting, build and capture
  `issuectl --help` plus every `issuectl <cmd> --help` to files; after each phase, diff —
  must be byte-identical. No `--json` shape changes. **Skill templates untouched** — if a
  template seems to need an update, the move was not behaviour-neutral; stop.
- **Green gate after every phase, one commit per phase:** `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
  `cargo build --workspace`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
  (moves break intra-doc `[`links`]` — the doc gate catches them).
- Use `git mv` for the file→`mod.rs` renames so history follows.

## Phase A — `main.rs` → `crates/issuectl/src/cmd/` family modules

The clap `enum Command` is ~1200 lines of arg definitions; **moving the arg structs/enums is
the point** — if every command's flags stay in one file, the hot file just moved.

1. Create `crates/issuectl/src/cmd/mod.rs` declaring one module per command family. Suggested
   assignment of the 58 handlers (adjust only when two commands share a private helper):
   - `cmd/read.rs` — list, show, search, stats, ready, open, pick, duplicates
   - `cmd/write.rs` — new/create, update, close, rename, note, set, assign, check, label,
     apply, bulk, body_set, depend, attach
   - `cmd/intake.rs` — intake_file, intake_migrate, intake_queue, intake_show + the intake
     transition handlers, triage
   - `cmd/skill.rs` — skill_list, skill_install, skill_print, skill_pi_status, skill_pi_prune,
     agents, init
   - `cmd/repo_admin.rs` — doctor, hooks, fmt, sync_commits, scan_todos, install_merge_driver,
     archive, stale
   - `cmd/views.rs` — dag, epic_tree, cycle_*, schedule_*, workload, burndown, metrics,
     activity, timeline, changelog, context, prompt
   - `cmd/transfer.rs` — export, import_json, import_github
   - `cmd/meta.rs` — version, config, completions, complete_values
2. Each family module holds **both** its clap types (`UpdateArgs`, `ConfigAction`,
   `CycleAction`, …) **and** its `cmd_*` handlers. `enum Command` stays in `main.rs` but each
   variant's payload references the family's types (one thin line per command), so editing one
   family's flags touches only that family's file.
3. Shared output/formatting helpers used by many handlers move to `cmd/mod.rs` (or stay in
   `main.rs` if only the dispatcher uses them).
4. Move each family's `#[cfg(test)]` tests from `main.rs` into its module; tests that exercise
   cross-family dispatch stay in `main.rs`.
5. Target: `main.rs` well under 1000 lines (Cli struct, thin `enum Command`, dispatch match,
   `find_root`, `fn main`). Green gate, commit
   `refactor(cli): split main.rs into cmd/ family modules (pure move)`.

## Phase B — `doctor.rs` → `crates/issuectl-core/src/doctor/`

1. `git mv src/doctor.rs src/doctor/mod.rs`, then carve:
   - `doctor/checks.rs` — the read-only finding producers
   - `doctor/apply.rs` — the `--fix` pipeline (`DoctorActions`, apply outcome, preflight /
     post-apply phases)
   - a further coherent chunk (e.g. the `.issuectl/AGENTS.md` regen) gets its own file if it
     cleanly separates; do not force it
   - `doctor/mod.rs` keeps the public types, orchestration entry points, and `pub use`
     re-exports so every existing `doctor::X` path still resolves
2. Move each test beside the impl it tests; a residual `doctor/tests.rs` is acceptable for
   cross-cutting ones. Target: no file over ~2500 lines. Green gate, commit.

## Phase C — `mutate/mod.rs` → sibling verb files

`mutate/` already has the pattern (`new_issue.rs`, `intake.rs`, `archive.rs`, …); `mod.rs`
just still holds too much.

1. Move verb families into siblings, e.g. `mutate/update.rs` (`update_issue*`,
   `bulk_update`), `mutate/close.rs` (`close_issue*`), `mutate/body.rs` (`update_body*`,
   `note_issue*`, `toggle_checkbox*`), and `mutate/custom_fields.rs` for the key/value
   validation helpers if it cuts cleanly.
2. `mod.rs` keeps: `MutateError`, the repo flock (`acquire` / `canonical_root`),
   `write_item_atomic`, shared validation plumbing, and `pub use` re-exports preserving every
   existing `mutate::X` path.
3. Distribute the `#[cfg(test)]` block beside the moved verbs. Green gate, commit.

## Phase D — bookkeeping

1. Narrow `AGENTS.md`'s hot-file list: `main.rs` becomes "two worktrees editing the same
   `cmd/<family>.rs` collide; different families are parallel-safe"; the `mutate/` and
   `doctor.rs` entries narrow the same way (`mod.rs` + the specific file touched).
2. Update the `collision:` tokens on the open laned issues that currently name `main.rs` to
   the new family file that actually covers them.
3. Add a `### Internal` note under `CHANGELOG.md` `[Unreleased]`.
4. Green gate, commit, merge back per `/worktree-merge`.

## Acceptance

- All three files split as above; no file in the touched set over ~2500 lines and `main.rs`
  under ~1000.
- Captured `--help` output byte-identical pre/post. Full green gate passes after every phase.
- `AGENTS.md` hot-file list narrowed in the same change.
- No skill-template change was needed (if one was, the move was not behaviour-neutral).

## Risk

This is the maximally-colliding change in the repo: it runs **alone** — no other worktree may
touch `main.rs`, `doctor.rs`, or `mutate/` while it is live, and it invalidates any in-flight
branch touching them. The `skills` lane (only `skill.rs` + templates) is the one lane safe to
run in parallel.
