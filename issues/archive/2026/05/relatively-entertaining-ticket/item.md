---
created: 2026-05-08
updated: 2026-05-09
type: chore
reporter: jari
status: done
priority: normal
epic: exorbitantly-ill-apples
labels: [release-v0.5.0]
closed: 2026-05-09
---

# CLI golden-test harness for cmd_new error output

## Description

Spin-off from @astoundingly-harsh-nest /llm-review (round 1, finding M10).

## Description

The typed-error refactor (commit 52cd755) promises that `cmd_new`'s
human-readable error text stays byte-identical to pre-refactor output.
Today that promise is anchored by:

- A manual smoke test recorded in commit 52cd755's message (4 paths).
- A unit test in `mutate.rs` that locks `From<DoNewError> for
  anyhow::Error` text per variant (commit 397890b).

The unit test catches the typical drift mode (someone edits a
variant without touching the conversion). It does NOT catch:

- `cmd_new`'s own `println!` formatting (success / JSON branches).
- Anyhow's `Debug` rendering used by `main()`'s `Result<()>`
  (multi-line "Caused by:" chains).
- Drift introduced anywhere upstream of `cmd_new` that still
  changes the final terminal output.

Skripts and AI-agents (this project's own `/issue` skill included)
parse `issuectl` output. The wording is a de-facto interface; we
should not rely on smoke tests to defend it.

## Out of scope here

- Per-variant DoNewError-to-anyhow text test already exists; no
  change needed there.
- Other commands (`update`, `close`, `note`, `check`, `apply`,
  `ls`, `show`, `search`) — this issue is about establishing the
  harness. Once the pattern exists, expanding coverage is cheap
  follow-up.

## Fix sketch

1. Add `assert_cmd` (or equivalent) as a dev-dependency.
2. Decide on harness shape: integration tests under `tests/`
   (Rust convention) vs. inline `#[cfg(test)]` (project convention
   today). Project convention says inline; integration tests
   would deviate. Pick deliberately and document in AGENTS.md.
3. Snapshot vs. exact-match: prefer `assert_eq!` against a literal
   for the small set of failure messages here; reserve
   `insta`-style snapshots for if/when the surface grows.
4. Fixture strategy: `tempfile`-based repos seeded by the helpers
   already in `mutate::tests`.
5. Cover at minimum: validation (--owner on non-epic), conflict
   (slug taken), schema-violation (missing required field),
   schema-config (malformed YAML), IO (chmod + uid-0 probe-skip
   like the existing test).
6. Run via `cargo test --release` like the rest of the suite.

## Definition of done

- Harness exists and runs in CI.
- `cmd_new`'s 5 error paths above are locked to exact stderr.
- AGENTS.md updated with the convention decision (integration
  tests vs. inline) and a one-line rule for when each applies.
- Existing 407 tests still pass.
