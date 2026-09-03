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
principles in `/ai-first-cli-canon`:

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

The release path is the [Shipshape](https://github.com/jarimustonen/ossctl)
release engine (`/shipshape-release` → `shipshape release plan|cut`), which
reads the approved [`OSS-RELEASE.md`](OSS-RELEASE.md) contract. `shipshape
release cut` owns the version bump, CHANGELOG finalization, the crates.io
publish of both crates (adapter `cargo-publish`), and the `vX.Y.Z` git tag.

Binary distribution stays with [`cargo-dist`](https://opensource.axo.dev/cargo-dist/)
(installed locally as `dist`) via one GitHub Actions workflow:

- `.github/workflows/release.yml` — generated by `dist`. On the `vX.Y.Z`
  tag push (the tag Shipshape creates): builds binaries for macOS (x86_64 +
  aarch64) and Linux (x86_64), creates a GitHub Release with tarballs + a
  shell installer, pushes the Homebrew formula to
  `jarimustonen/homebrew-issuectl`. It does **not** publish to crates.io.

> `publish-crates.yml` was retired 2026-08-10 — its `release: [published]`
> trigger never fired (cargo-dist publishes the Release with `GITHUB_TOKEN`,
> which does not fire downstream workflows), so crates.io needed a manual
> trigger every release. The crates.io publish now lives in `shipshape release
> cut`, which closes that gap.

### One-time setup

1. **Shipshape release engine**: install the maintained CLI with
   `cargo install shipshape` and install its skills with `shipshape skill
   install --agent all`.
2. **Homebrew tap repo**: create `jarimustonen/homebrew-issuectl` on
   GitHub (empty). The release workflow commits `Formula/issuectl.rb`
   to it on each release.
3. **`HOMEBREW_TAP_TOKEN`** secret in this repo: a classic Personal
   Access Token with `repo` scope, used by the release workflow to
   push the formula to the tap repo.
4. **crates.io credential** where you run `shipshape release cut` (it runs
   `cargo publish` locally, not in CI): either `cargo login` once, or
   export `CARGO_REGISTRY_TOKEN` in that shell — an API token from
   <https://crates.io/settings/tokens> with `publish-update` scope,
   **crate-scoped to `issuectl` + `issuectl-core`** and with an expiry set.
   Because it lives on a workstation, keep it least-privilege and rotate it.
5. **Retire the old CI secret** (one-time cleanup, do this now): the
   repo-level `CARGO_REGISTRY_TOKEN` GitHub Actions secret that fed
   `publish-crates.yml` is no longer used by any workflow. **Revoke that
   token** at <https://crates.io/settings/tokens> and **delete the secret**
   from the repo's Settings → Secrets → Actions — a stale publish-scoped
   token is attack surface.

### Per-release steps

The bump/CHANGELOG/publish/tag steps are driven by Shipshape. You select the
SemVer bump, inspect the sealed plan, and the engine performs the release in a
clean checkout while refusing on drift.

1. **Preflight.** Ensure `main` is green (see the green gate in
   [AGENTS.md](AGENTS.md) "Operating policy"), that your working tree is
   clean and at `origin/main`, that `CHANGELOG.md` `## [Unreleased]` holds
   the items for this release, and that the crates.io credential from
   "One-time setup" is present in this shell.
   > **Caret boundary — do this BEFORE step 2.** When the bump crosses a
   > caret boundary (e.g. `0.6.x → 0.7.0`, or any major bump), edit the
   > internal `issuectl-core = { path = "../issuectl-core", version =
   > "X.Y.Z" }` requirement in `crates/issuectl/Cargo.toml` to the new
   > version and commit it — Shipshape rewrites exact (`=`) intra-workspace
   > pins but does not rewrite this caret requirement, and if it's stale
   > `cargo publish` of `issuectl` selects the old `issuectl-core`. Patch
   > bumps within the same minor don't need this.
2. **Seal the plan** (inspect what will happen; this persists the sealed plan
   but does not edit the project checkout):
   ```sh
   shipshape release plan --bump patch|minor|major
   ```
   The plan records the target version and the full release effect. Its phases
   include `bump → dry-run-all → build-all → publish-all → tag → dist → verify`.
   `publish-all` publishes both crates (`issuectl-core` before `issuectl`);
   `tag` creates `vX.Y.Z`, which fires cargo-dist for binaries.
3. **Cut the release** with the sealed plan id from step 2:
   ```sh
   shipshape release cut --plan <PLAN_ID>
   ```
   Shipshape applies the planned workspace version and lockfile edits,
   finalizes `CHANGELOG.md` (moves `[Unreleased]` → `[X.Y.Z]` with the date),
   and runs the contract's `scripts/release-bump-hook.sh`. The hook builds the
   bumped `issuectl` in a disposable target and regenerates all nine repo-local
   Claude, pi, and Codex skill copies under an isolated `HOME`. Shipshape then
   commits those mutations, publishes both crates to crates.io, pushes the
   `vX.Y.Z` tag, and verifies every declared destination.
4. Watch the workflows on [Actions](https://github.com/jarimustonen/issuectl/actions).
   The tag-triggered **Release** workflow (cargo-dist) creates the GitHub
   Release with binaries and updates the Homebrew tap. crates.io was
   already published in step 3 — there is no separate manual trigger.
5. Verify both crates landed at `X.Y.Z`:
   ```sh
   for c in issuectl issuectl-core; do cargo search "$c" --limit 1; done
   ```

#### If a cut is interrupted

`shipshape release cut` executes a content-addressed sealed plan and refuses
on repo drift. If execution is interrupted, use `shipshape release resume
<RUN_ID>` to reconcile and continue (a `cargo publish` that already landed is
a no-op — crates.io rejects duplicate `name@version`). `shipshape release
verify <RUN_ID>` is a read-only reconcile against the declared destinations.

Note the phase order: crates.io publish happens **before** the tag. If
`publish-all` succeeded but `tag` failed, the crates are permanent (yank
only) but the release is *not* lost — `resume` pushes the `vX.Y.Z` tag for
the same sealed source, which fires cargo-dist for binaries. Only if you
must abandon the version entirely do you `cargo yank` both crates and cut a
new patch — you can never re-publish the same `name@version`.

### Updating cargo-dist itself

If `dist` releases a new version and you want to upgrade:

```sh
cargo install cargo-dist --force
dist init --yes        # regenerates the workflow with new dist version
git diff               # review what changed
```
