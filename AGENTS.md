# AGENTS.md

Guidance for agents and humans working inside this repository. This file holds
what you could not learn from the tree itself: earlier decisions and their
reasons, traps, and the maintainer's preferences. Where the reasoning already
lives in `docs/decisions/` (ADRs) or `docs/design/`, this file points there
instead of repeating it.

## What this project is

`issuectl` is an AI-first CLI for managing markdown issues with YAML
frontmatter, stored under `issues/`. [README.md](README.md) is the user-facing
overview. The repo tracks its own work with `issuectl`; see
[issues/AGENTS.md](issues/AGENTS.md) and the `/issue` skill for that.

`issuectl --help` and `issuectl <subcommand> --help` are the source of truth for
accepted flags. This file does not restate them.

## CLI design

The binding reference for any CLI-surface work is the `/ai-first-cli-canon`
skill: strict input validation, `--json` output, no interactive prompts,
informative errors, composable commands. The copy under
`.claude/skills/ai-first-cli-canon/` is installed by `project-canon skill
install` and is overwritten on reinstall, so a change to the canon belongs in
`project-canon`, followed by a reinstall here. ADR 0004
([docs/decisions/0004-cli-verb-surface.md](docs/decisions/0004-cli-verb-surface.md))
is the verb-surface policy: `update` is the sole selective mutation verb, new
top-level verbs need an ADR amendment, and permanent aliases are not wanted.

**The `--json` envelope is the agent-facing contract**, and other programs
parse it byte for byte. Every success, including partial success (`import`,
exit 2), is `{"schema_version":1,"data":…,"warnings":[]}` on stdout; domain
fields live only in `data`, non-fatal warnings only in top-level `warnings`.
Every no-work error is
`{"schema_version":1,"error":{"code":"<kebab>","message":"…",…}}` on stderr
with empty stdout, including not-found, `fail()`, clap usage errors, and
doctor `--fix` errors (whose stable `details` sit inside `error`). Read-only
`doctor` is the one exception: it keeps its full result on stdout whatever its
exit status, so `issuectl --json doctor | jq` works on an unhealthy repo.
`schema_version` here is the CLI output API version, separate from
`SUPPORTED_SCHEMA_VERSION` for `.schema.yaml`; bump it only for a breaking
output change, never for an added field.

When scripting `issuectl` from another tool or agent, use `--json`. The
human-readable mode exists for people at a terminal and its text is not stable.

## The skill templates are the consumer contract

The `/issue`, `/issue-new`, and `/issue-intake` skills ship inside the binary
(`include_str!` in `crates/issuectl-core/src/skill.rs`) and are installed into
consumer repos by `issuectl skill install`. For an agent in another repository
they are the *only* description of this CLI it will ever read. A CLI change
that an agent would notice (a new or renamed flag, changed accepted values,
changed `--json` shape or defaults, changed install paths, changed exit-code
semantics it handles) is therefore incomplete until the templates say the same
thing, in the same commit.

The templates live in `crates/issuectl-core/templates/`. Each skill has a
Claude/pi variant (`*-skill.md`, with YAML frontmatter) and a Codex variant
(`*-prompt.md`), which is the same body with the frontmatter stripped. The
seventh file there, `issues-agents.md`, is the scaffold `init` writes to
`issues/AGENTS.md`, not a skill.

The skills are dogfooded into this repo, and the test
`skill::tests::dogfooded_copies_match_templates` fails if any of the nine
copies drifts from its template:

| Template | Copies |
|---|---|
| `issue-skill.md` | `.claude/skills/issue/SKILL.md`, `.pi/agent/skills/issue/SKILL.md` |
| `issue-prompt.md` | `.codex/prompts/issue.md` |
| `issue-new-skill.md` | `.claude/skills/issue-new/SKILL.md`, `.pi/agent/skills/issue-new/SKILL.md` |
| `issue-new-prompt.md` | `.codex/prompts/issue-new.md` |
| `issue-intake-skill.md` | `.claude/skills/issue-intake/SKILL.md`, `.pi/agent/skills/issue-intake/SKILL.md` |
| `issue-intake-prompt.md` | `.codex/prompts/issue-intake.md` |

After editing a template, `issuectl skill install --agent all --force`
refreshes the copies. If a Claude/Codex pair has drifted, regenerate the Codex
file from the Claude one; `/issue`'s frontmatter is four lines and the intake
skills carry an extra `argument-hint` line, hence the different offsets:

```sh
tail -n +5 templates/issue-skill.md         > templates/issue-prompt.md
tail -n +6 templates/issue-new-skill.md     > templates/issue-new-prompt.md
tail -n +6 templates/issue-intake-skill.md  > templates/issue-intake-prompt.md
```

`standalone_intake_skills_are_wellformed` additionally pins the split between
`/issue-new` (filing) and `/issue-intake` (processing). `/triage-bugs` under
`.claude/skills/` is a repo-local deprecation alias only; it is not shipped in
the binary. `skill install --agent pi` writes target-local
`.pi/agent/skills/`; the older global `~/.pi/agent/skills` corpus is only
inspected and pruned by `skill pi-status` / `pi-prune`, never written to
implicitly ([docs/design/pi-skill-mirror.md](docs/design/pi-skill-mirror.md)).

## Architecture facts and the reasons behind them

**Domain code lives in `issuectl-core`; the `issuectl` binary owns only clap
structs, `find_root`, the `cmd_*` handlers, and `main`.** The two are separate
crates, so core cannot call into the binary; if a mutation site needs a helper,
the helper belongs in a core module (`issue_fields.rs`, `refs.rs`, …). A
`_pub` re-export wrapper is the tell that something has leaked the wrong way.
`issuectl-core` is published but explicitly unstable (see its `lib.rs` doc
comment): the semver contract is the binary's CLI surface. The CLI canon's
§22 asks for a no-I/O core and a `*-cli` binary name; both were considered and
rejected, with reasons in
[ADR 0002](docs/decisions/0002-io-stays-in-core.md). Do not treat them as gaps.

**Every write goes through a function in `issuectl-core/src/mutate/`.** That
is the one place where the repo-wide `flock` is taken, the canonical version
token is emitted, and schema validation runs. Calling `write::*` from the
binary would skip all three. A `cmd_*` handler is argument parsing plus
JSON/human formatting, ideally under thirty lines.

**`blocked_by` stays in the `extra` map.** Its top-level JSON appearance is a
canonical projection, not a typed field. Typing it would change every existing
issue's version token; that trade was weighed and refused. The scheduling
fields `lane` / `collision` / `lane_seq` are the contrasting typed case, hashed
only when set so untouched issues hash as before.
[ADR 0003](docs/decisions/0003-frontmatter-field-typing.md) has the full
reasoning and the DAG semantics;
[docs/design/lane-design.md](docs/design/lane-design.md) explains how lanes
are meant to be drawn.

**Slugs.** `create` derives a kebab slug from the title; `--slug-random` opts
into the random form, which is also the fallback for unusable titles; an
explicit `--slug` is authoritative. The three paths deliberately handle
collisions differently: an explicit slug errors, the derived one takes a
`-2`/`-3` suffix (`claim_derived_slug`), the random one retries internally
(`claim_random_slug`). `intake file` and recurring occurrences force the random
form because their titles are untrusted or repeat; `import` keeps the derived
default.

**Wall-clock time goes through `Clock`.** `SystemClock` in production,
`FixedClock` in tests; the only `Utc::now()` in `issuectl-core` is inside
`SystemClock`. `SystemClock::today()` uses the local calendar while
`FixedClock::today()` reads its UTC instant, so pin test instants mid-day UTC
unless the test is about a date boundary.

**Doctor `--fix` moves forward only.** It never rolls back partial progress;
scripted callers branch on `stop_phase`, the `--fix --json` error codes are
stable, and schema `required_when` plus status/type aliases drive coercion.
[docs/design/doctor-fix.md](docs/design/doctor-fix.md) is the reference.

**Archived issues live at `issues/archive/YYYY/MM/<slug>/` inside the repo**,
bucketed by `closed:` (falling back to `updated:`). Discovery is archive-aware,
so `show`, `list`, and queries find them; the same slug active and archived is
`Ambiguous`. Moving an issue out of a closing status unarchives it under the
write lock, and empty buckets are pruned. This repo uses `issuectl archive`
(default `--older-than 90d`); run it when closed issues pile up in the active
tree.

**Attachments and body refs.** Per-issue `attachments/` and `fixtures/` are
created on demand by `ensure_issue_subdir`, because git drops empty
directories. Relative body refs resolve against the issue directory and the
extractor rejects `../` and backslash traversal. Extraction uses
pulldown-cmark rather than a regex because the CommonMark parser skips code
spans and blocks for free; a regex would reintroduce false refs. In `doctor`,
`BodyRef.has_line_anchor` (a GitHub-style `#L<n>` fragment) is the only thing
that lets a ref be skipped as a code permalink when a same-named file exists at
the repo root. Skipping unconditionally would hide a missing attachment whose
name collides with a repo-root file; the test
`broken_refs_still_flags_when_filename_collides_with_repo_root` pins this.
`doctor` also warns on binaries over 1 MiB and non-AVIF raster images, since
the tracker is git-history.

**The planning-doc-type list is owned by the upstream `init-project` skill.**
issuectl neither enumerates nor enforces it, so there is one source.

**Tests sit next to the code** in `#[cfg(test)]` modules. The integration
tests in `crates/issuectl/tests/` exist only for what an inline test cannot
observe: process exit codes, byte-level stdout/stderr, argument parsing by the
built binary, and `main()`'s error rendering. New features come with tests;
bug fixes come with a regression test that fails before the fix.

[CONTRIBUTING.md](CONTRIBUTING.md) covers dev setup, commit-message
conventions (Conventional Commits), and the PR process.

## Trust findings less than the tree

A scan, audit, or pre-check is a hint, not evidence. Before laning work off a
finding, scoping work down because of one, or reporting its conclusion onward,
confirm it against the tree yourself. This has failed in both directions: a
canon audit reported "no core/cli split" when the split had existed for months
(`@cli-canon-s22`), and a triage pre-scan cleared the public package of
user-specific leaks that a worker redoing the sweep then found
(`@audit-no-user-specifics`). When briefing a worker on a finding, say that the
earlier scan is a hint and ask it to redo the check.

## Operating policy (for `/stint`)

`/stint` reads this section for how to run a work session here. The work queue
and the session handoff live in [TODO.md](TODO.md); this section is the
standing policy. The maintainer's preferences below are decisions, not
suggestions: a stint follows them without asking again.

### What is precious and what is not

Two things here cannot be undone: a crates.io publish (yank only, and the
`name@version` can never be reused) and a pushed `vX.Y.Z` tag. `main` is
shared and must stay green, since Shipshape cuts releases from it and refuses
on drift. Everything else, including worktrees, sealed plans, and the Homebrew
tap formula, can be redone. Weigh interruptions accordingly: the maintainer's
attention is the scarce resource, and a question is worth asking only when the
outcomes differ in a way they would care about and you cannot tell which they
would pick.

### Deploy means release

This is a Rust CLI, not a service. Releasing is the Shipshape engine
(`/shipshape-release`, `shipshape release plan|cut`) reading the approved
[OSS-RELEASE.md](OSS-RELEASE.md) contract. Preconditions: `[Unreleased]` in
`CHANGELOG.md` is complete, `main` is clean, pushed, and green. Before a minor
or major bump, update and commit the internal `issuectl-core` requirement in
`crates/issuectl/Cargo.toml`; Shipshape rewrites exact pins but not this caret
requirement, and a stale one makes `cargo publish` of `issuectl` select the old
core. Then:

```sh
shipshape release plan --bump patch|minor|major   # inspect the sealed plan
shipshape release cut --plan <id>
```

The cut owns the workspace version bump, `Cargo.lock` refresh, CHANGELOG
finalization, the `scripts/release-bump-hook.sh` refresh of the dogfooded
skill copies, the release commit, the crates.io publishes (`issuectl-core`
before `issuectl`), and the tag. The tag fires cargo-dist
(`.github/workflows/release.yml`) for GitHub Release binaries, the shell
installer, and the Homebrew tap; Shipshape's verify barrier waits for those
too. An interrupted cut is continued with `shipshape release resume <run_id>`;
`shipshape release verify <run_id>` is a read-only reconcile. The tag also
fires `publish-crates.yml`, a CI backstop that treats "already published" as
success, so its green run proves nothing on its own. Full steps:
[CONTRIBUTING.md](CONTRIBUTING.md) "Per-release steps".

**Releases are cut autonomously, without a go/no-go question** (maintainer
decisions, 2026-08-05 and 2026-08-06). When `main` carries unreleased
user-facing changes and the green gate passes, run the recipe end to end and
report as you go. The safety is structural: content-addressed sealed plan,
`dry-run-all` before any publish, dependency-ordered publish, resume/abandon
recovery, and the verify barrier. What must never happen is a publish from a
red tree.

**Check the channels yourself after every cut.** The engine's verdict has been
wrong both ways on real cuts: 0.15.0 reported failure with everything
delivered, and 0.16.0 correctly reported `gh-releases` missing but with no
cause, which looked the same as "still building". Before the tap was watched
it once sat stale through three releases (`@homebrew-tap-stale`). So confirm
directly that the GitHub Release has assets and the tap formula advanced:

```sh
gh release view vX.Y.Z --json assets --jq '.assets|length'   # compare with the previous tag
curl -s -H 'User-Agent: issuectl-check' https://crates.io/api/v1/crates/issuectl | jq -r '.crate.max_version'
```

The `User-Agent` header matters: crates.io returns `null` for every field
without one, which makes a successful publish look failed. A zero asset count
is ambiguous between "still building", "died", and "never ran"; resolve it at
the delegated run (`gh run list --workflow=release.yml --limit 1`, then `gh
run view <id> --json jobs`), not at the destination. A cancelled build job
skips `host` and `publish-homebrew-formula`, so there is no Release *and* a
stale tap. Recovery is `gh run rerun <id> --failed`; the publish and tag are
already permanent, so re-cutting or re-tagging is never the answer.

**Binary distribution belongs to cargo-dist**, configured in
`dist-workspace.toml`, whose `[dist.github-custom-runners]` table sends the
macOS ARM64 build to a self-hosted runner (fast, versus 45+ minutes waiting
for a hosted allocation). `shipshape dist generate` and a bare `dist generate`
both drop that override and the runner-isolation guard, and `/shipshape-dist`
refuses to emit a runner override at all. `release.yml` is regenerated only
through `scripts/release_workflow.py` (see CONTRIBUTING "Updating cargo-dist
itself").

### Git

`pull --rebase` then `push` on a clean, green `main` needs no confirmation
(maintainer decision, 2026-08-05). Force-pushing a shared branch or pushing a
red tree would break the release path and other sessions; neither is ever
wanted.

### Live-version check

Shipped: `git tag --sort=-creatordate | head -1` and `grep '^version'
Cargo.toml`. Published: the crates.io query above and the Homebrew tap. Compare
against `main` before proposing a release.

### Green gate

A unit counts as landed when all of these pass:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

CI (`ci.yml`) is looser than this gate: it runs clippy without `-D warnings`
and does not run `cargo doc`, so a green CI is not evidence the gate passes.
The doc step is the one most often skipped locally; broken intra-doc links
surface only there. A release build is not required per unit.

### Hot files

Two worktrees editing the same file collide; different files in the same
family are parallel-safe. The families to sequence on:

- `crates/issuectl/src/cmd/<family>.rs` (one file per command family)
- `crates/issuectl-core/src/mutate/mod.rs` plus the specific mutation verb file
- `crates/issuectl-core/src/doctor/mod.rs` plus the specific doctor module
- `crates/issuectl-core/src/schema.rs`
- `crates/issuectl-core/templates/` (the six skill templates move together with
  their dogfooded copies)

The maintainer favors maximal parallelism: when lanes touch no shared hot
file, spawn them all at once rather than one at a time. Sequence only real
collisions.

### Other facts `/stint` asks for

- No external services or test accounts; tests are hermetic (tempdirs). There
  is no reset step.
- A same-titled orchestrator run in a sibling repo is not this repo's run.
  Cross-repo campaigns spawn look-alike runs in each repo; identify a run by
  its working directory and `git worktree list`, never by its title.
