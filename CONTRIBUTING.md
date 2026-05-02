# Contributing to issuectl

Thank you for considering a contribution. This document covers how to set
up the project, the conventions we follow, and the PR process.

## Development setup

Requires Rust (2021 edition). Stable toolchain is fine.

```sh
git clone https://github.com/jarimustonen/issuectl
cd issuectl
cargo build
cargo test
```

Useful one-liners while iterating:

```sh
cargo run -- list                                # against this repo's issues/
cargo run -- --root tests/fixtures/grooveserve list   # against the fixture
cargo test                                       # unit + integration tests
cargo clippy --all-targets                       # linter
cargo fmt --all                                  # format
```

## Repository layout

```
src/
├── main.rs        # CLI entry point + command implementations + tests
├── models.rs      # Issue / Commit data types
├── parser.rs      # item.md frontmatter + body parsing
├── repo.rs        # Filesystem walking + load_issues + find_highest_number
├── skill.rs       # `issuectl skill install` (embeds templates/)
└── write.rs       # Frontmatter mutation, render_new_item, round-trip
templates/         # Embedded via include_str! into the binary
tests/fixtures/    # End-to-end manual-verification fixtures
issues/            # The project's own issue tracker (eats its own dog food)
```

## Design principles

This is an **AI-first CLI**. Every change should be consistent with the
principles in [AGENTS-AI-FIRST-CLI.md](AGENTS-AI-FIRST-CLI.md):

- Strict input validation; reject unknown values with the list of valid
  alternatives. No silent fixups, no coercion.
- Structured `--json` output where applicable.
- No interactive prompts. All input via flags, output to stdout/stderr,
  meaningful exit codes (0 = success, ≠0 = failure).
- Errors include the offending value and the expected format.

## Testing

Tests live alongside the code in `#[cfg(test)] mod` blocks:

- `src/write.rs` — pure helpers, frontmatter round-trip, slugify
- `src/repo.rs` — `find_highest_number`, `load_issues`
- `src/main.rs` — CLI helpers (`parse_commit_spec`, `normalize_related_refs`,
  `is_closing_status`) and integration tests for `do_new` / `do_update` /
  `do_close` via tempfile-backed test repos.

When adding a feature, add a test. When fixing a bug, add a regression
test that fails before the fix.

```sh
cargo test                       # all tests
cargo test write::               # one module
cargo test -- --nocapture        # see println! output
```

## Coding conventions

- Run `cargo fmt --all` before committing.
- Run `cargo clippy --all-targets` and resolve any new warnings in code
  you touched (pre-existing warnings can be addressed in a separate PR).
- Prefer dedicated tools over shelling out (use `fs::rename` rather than
  `git mv`; the user's commit captures renames anyway).
- Don't add backwards-compat shims unless the change is a breaking one
  shipped after a 1.0 release.

## Commit messages

Follow the [Conventional Commits](https://www.conventionalcommits.org/)
style:

```
feat: add --root flag for external repo targeting
fix(renumber): preserve blank line between frontmatter and body
perf(renumber): hoist regex compilation out of per-line loop
docs(readme): document the JSON schema
test(write): cover round-trip with unknown frontmatter fields
```

Common types: `feat`, `fix`, `perf`, `docs`, `test`, `refactor`, `chore`.

## Pull requests

1. Open an issue first for larger changes so we can align on scope.
2. Fork and create a feature branch.
3. Write tests for new behavior.
4. Make sure `cargo test` and `cargo clippy --all-targets` pass.
5. Update [CHANGELOG.md](CHANGELOG.md) under `## [Unreleased]`.
6. Open the PR. CI runs build, test, clippy, and rustfmt checks on
   stable Rust on Linux and macOS.

For trivial fixes (typos, doc updates) feel free to open the PR directly.

## Reporting bugs

Use [GitHub Issues](https://github.com/jarimustonen/issuectl/issues) and
the bug report template. Include:

- `issuectl --version`
- `rustc --version`
- A minimal reproduction (an `item.md` snippet plus the command you ran)
- Expected vs. actual behavior

For security issues, see [SECURITY.md](SECURITY.md) — please do **not**
file public GitHub Issues for security bugs.

## Releasing (maintainers)

1. Update `CHANGELOG.md`: move items from `## [Unreleased]` to a new
   `## [x.y.z]` section, with the date.
2. Update `Cargo.toml` `version`.
3. Tag: `git tag -a vX.Y.Z -m "Release X.Y.Z"`.
4. Push: `git push --follow-tags`.
5. Publish: `cargo publish` (once we're on crates.io).
