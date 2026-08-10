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
cargo run -- --root /path/to/some/repo list          # against an external repo
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
- Optional: enable the tracked pre-push hook that mirrors CI's lint job
  with `git config core.hooksPath .githooks` (one-off per clone).
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

The release path is the [`ossctl`](https://github.com/jarimustonen/ossctl)
release engine (`/oss-release` → `ossctl release plan|cut`), which reads
the approved [`OSS-RELEASE.md`](OSS-RELEASE.md) contract. `ossctl release
cut` owns the version bump, CHANGELOG finalize, the crates.io publish of
both crates (adapter `cargo-publish`), and the `vX.Y.Z` git tag.

Binary distribution stays with [`cargo-dist`](https://opensource.axo.dev/cargo-dist/)
(installed locally as `dist`) via one GitHub Actions workflow:

- `.github/workflows/release.yml` — generated by `dist`. On the `vX.Y.Z`
  tag push (the tag ossctl creates): builds binaries for macOS (x86_64 +
  aarch64) and Linux (x86_64), creates a GitHub Release with tarballs + a
  shell installer, pushes the Homebrew formula to
  `jarimustonen/homebrew-issuectl`. It does **not** publish to crates.io.

> `publish-crates.yml` was retired 2026-08-10 — its `release: [published]`
> trigger never fired (cargo-dist publishes the Release with `GITHUB_TOKEN`,
> which does not fire downstream workflows), so crates.io needed a manual
> trigger every release. The crates.io publish now lives in `ossctl release
> cut`, which closes that gap.

### One-time setup

1. **Homebrew tap repo**: create `jarimustonen/homebrew-issuectl` on
   GitHub (empty). The release workflow commits `Formula/issuectl.rb`
   to it on each release.
2. **`HOMEBREW_TAP_TOKEN`** secret in this repo: a classic Personal
   Access Token with `repo` scope, used by the release workflow to
   push the formula to the tap repo.
3. **`CARGO_REGISTRY_TOKEN`** secret in this repo: an API token from
   <https://crates.io/settings/tokens> with `publish-update` scope.

### Per-release steps

The bump/CHANGELOG/publish/tag steps are driven by `ossctl` — you supply
the approved version; the engine does the rest and refuses on repo drift.

1. Ensure `main` is green (see the green gate in
   [AGENTS.md](AGENTS.md) "Operating facts") and that `CHANGELOG.md`
   `## [Unreleased]` holds the items for this release.
2. **Seal the plan** (dry-run — inspect what will happen, no changes yet):
   ```sh
   ossctl release plan --version X.Y.Z
   ```
   Phases: `dry-run-all → build-all → publish-all → tag → dist`.
   `publish-all` publishes both crates (`issuectl-core` before `issuectl`);
   `tag` creates `vX.Y.Z`, which fires cargo-dist for binaries.
3. **Cut the release** with the sealed plan id from step 2:
   ```sh
   ossctl release cut --plan <PLAN_ID> --version X.Y.Z
   ```
   ossctl bumps `version` in `Cargo.toml`, finalizes `CHANGELOG.md`
   (moves `[Unreleased]` → `[X.Y.Z]` with the date), publishes both crates
   to crates.io, and pushes the `vX.Y.Z` tag.
   > **Caret note:** when the bump crosses a caret boundary (e.g.
   > `0.6.x → 0.7.0`, or any major bump), the internal
   > `issuectl-core = { path = "../issuectl-core", version = "X.Y.Z" }`
   > requirement in `crates/issuectl/Cargo.toml` must move to the new
   > version too, or `cargo build` can't select `issuectl-core`. Patch
   > bumps within the same minor don't need this.
4. Watch the workflows on [Actions](https://github.com/jarimustonen/issuectl/actions).
   The tag-triggered **Release** workflow (cargo-dist) creates the GitHub
   Release with binaries and updates the Homebrew tap. crates.io was
   already published in step 3 — there is no separate manual trigger.
5. Verify: `curl -s https://index.crates.io/is/su/issuectl | tail -1`.

If a cut is interrupted, `ossctl release resume` reconciles and continues;
`ossctl release verify` does a read-only reconcile against the registry.

### Updating cargo-dist itself

If `dist` releases a new version and you want to upgrade:

```sh
cargo install cargo-dist --force
dist init --yes        # regenerates the workflow with new dist version
git diff               # review what changed
```
