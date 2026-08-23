# AGENTS.md

Guidance for AI agents (and humans) working **inside this repo**. This file
holds the *rules*; the reasoning behind a rule lives in `docs/decisions/`
(ADRs) and `docs/design/` — follow the links when you need the why.

## What this project is

`issuectl` — AI-first CLI for managing markdown-based issues with
frontmatter. See [README.md](README.md) for a user-facing overview.

## CLI Design Principles

Use the `/ai-first-cli-canon` skill shipped by `project-canon` as the
maintained AI-first CLI canon. It is the binding reference for CLI surface
work: strict input validation, `--json` output, JSONL logs, no interactive
prompts, informative errors and composable commands. Do not keep or edit a
repo-local `ai-first-cli-canon` copy; update the canon through the
`project-canon` tool and reinstall the released skill.

## Critical rule: keep the skill in sync with the CLI

The `/issue` skill template is shipped with the binary via `include_str!` in
`src/skill.rs` and installed by `issuectl skill install`. It tells
consumer-side agents how to use this CLI. **It is the only contract those
agents see.**

**Whenever you change the CLI in a way an agent would notice, update the
skill templates in the same commit.** Triggers include:

- New subcommand or flag added → add a usage example
- Flag renamed, removed, or its accepted values changed → update examples
- Output shape changed for `--json` → update the documented shape
- Install destination or filename changed → update the install table
- Default values changed → update the prose
- Error messages or exit-code semantics changed in a way agents handle

The template files (all under `crates/issuectl-core/templates/`, all
`include_str!`-embedded in `src/skill.rs`):

- `issue-skill.md` — `/issue`, Claude Code variant (YAML frontmatter)
- `issue-prompt.md` — `/issue`, Codex CLI variant (plain markdown)
- `issue-new-skill.md` — `/issue-new`, the intake **filing** skill, Claude variant
- `issue-new-prompt.md` — `/issue-new`, Codex CLI variant (plain markdown)
- `issue-intake-skill.md` — `/issue-intake`, the intake **processing** skill,
  Claude variant
- `issue-intake-prompt.md` — `/issue-intake`, Codex CLI variant (plain markdown)

They are dogfooded into this repo via `issuectl skill install --agent all
--force`:

| Template | Dogfooded copy |
|---|---|
| `issue-skill.md` | `.claude/skills/issue/SKILL.md` |
| `issue-prompt.md` | `.codex/prompts/issue.md` |
| `issue-new-skill.md` | `.claude/skills/issue-new/SKILL.md` |
| `issue-new-prompt.md` | `.codex/prompts/issue-new.md` |
| `issue-intake-skill.md` | `.claude/skills/issue-intake/SKILL.md` |
| `issue-intake-prompt.md` | `.codex/prompts/issue-intake.md` |

Each skill ships in **both** formats — a Claude skill (`--agent claude`) and a
Codex prompt (`--agent codex`); `all` installs both. The Codex prompt is the
Claude one with its YAML frontmatter stripped (body identical). After editing
any template, re-run the install command so the local copies don't drift from
`templates/`. The `skill::tests::dogfooded_copies_match_templates` test
enforces this for **all six** copies (and
`standalone_intake_skills_are_wellformed` additionally pins the intake skills'
filing/processing split). `/triage-bugs` is a repo-local-only deprecation
alias — it is **not** a binary-shipped template.

If a Claude/Codex pair would otherwise drift, regenerate the Codex one from
the Claude one by stripping its YAML frontmatter:

```sh
tail -n +5 templates/issue-skill.md         > templates/issue-prompt.md
tail -n +6 templates/issue-new-skill.md     > templates/issue-new-prompt.md
tail -n +6 templates/issue-intake-skill.md  > templates/issue-intake-prompt.md
```

(`/issue`'s frontmatter is 4 lines; the intake skills carry an extra
`argument-hint` line, so their frontmatter is 5 lines — hence `+6`.)

### pi.dev dual-home

Claude-layout installs also mirror each `SKILL.md` into pi.dev's global corpus
at `~/.pi/agent/skills/<name>/SKILL.md`, byte-identical, with an out-of-band
provenance manifest plus `skill pi-status` / `skill pi-prune` for drift and
cleanup. Reconciliation is **always-on-force** (a `--force` overwrite never
version-checks — deliberate). Full mechanics, guarantees, and the reasoning:
[docs/design/pi-skill-mirror.md](docs/design/pi-skill-mirror.md).

## Other conventions

- **Always `--json`** when scripting `issuectl` from another tool or
  agent. The human-readable mode is for terminal users.
- **`--json` output contract (the agent-facing contract).** Every success,
  including partial success (`import`, exit 2), is
  `{"schema_version":1,"data":…, "warnings":[]}` on stdout. Read domain fields
  only from `data`; non-fatal warnings are exclusively top-level `warnings`.
  Every no-work error is
  `{"schema_version":1,"error":{"code":"<kebab>","message":"…",…}}` on stderr
  with empty stdout; this includes bubble-up, not-found, `fail()`, clap usage
  errors, and doctor `--fix` errors (whose stable `details` remains inside
  `error`). Read-only doctor remains a stdout result regardless of its exit
  status, inside `data`. `schema_version` is the CLI output API version,
  independent from `SUPPORTED_SCHEMA_VERSION`; bump it only for breaking
  output changes, never additive fields. `issuectl version --json` reports
  both `supported_schemas` and bundled
  `skills[{name,cli_version,schema_version}]`.
- **Tests live next to the code** in `#[cfg(test)] mod` blocks by default.
  New features add tests; bug fixes add regression tests. Exception —
  `tests/` integration tests are only for black-box behaviour no inline test
  can observe: process exit code, byte-level stdout/stderr, argument parsing
  by the built binary, and `main()`'s `anyhow::Error` rendering.
- **New mutation verbs go in `mutate/`, CLI handlers stay thin.** Every write
  path routes through a function in `issuectl-core/src/mutate/` so a) every
  writer obtains the same repo-wide `flock`, b) every writer emits the same
  canonical version token, and c) schema validation runs in exactly one
  place. Keep the `cmd_*` handler to argument parsing + JSON/human formatting
  (≤30 lines target). Do **not** reach into `write::*` directly from the
  binary crate — that bypasses the lock and the schema check.
- **Domain code lives in `issuectl-core`; `issuectl` only owns CLI
  dispatch.** The binary crate owns clap structs, `find_root`, the `cmd_*`
  handlers, and `fn main`; everything else is a core domain module. The bin
  and lib are separate crates — a domain module cannot call into the binary.
  If a `mutate::*`/`write::*` site needs a helper, the helper belongs in a
  domain module (`issue_fields.rs`, `refs.rs`, …); a `_pub` re-export wrapper
  is the warning sign of a leak. `issuectl-core` is published but explicitly
  internal — its `pub` items are *not* a semver contract; the semver contract
  is the binary's CLI surface. Two canon-§22 asks (no-I/O core, `*-cli`
  binary name) are deliberately **rejected** — do not "fix" them:
  [ADR 0002](docs/decisions/0002-io-stays-in-core.md).
- **Verify a reported finding against the tree before you act on it — a
  scan, audit, or pre-check is a recommendation, not evidence.** Confirm the
  claim yourself before you lane work off it, scope work down because of it,
  or report its conclusion onward. This has been wrong in **both directions**:
  a canon audit filed a false "no core/cli split" finding (`@cli-canon-s22`),
  and a triage pre-scan falsely cleared the public package
  (`@audit-no-user-specifics`) — the worker that redid the sweep found real
  leaks. When briefing a worker: tell it to **redo the check**, and say
  explicitly that any prior scan is a hint, not a result.
- **`blocked_by` stays in `extra`; its JSON top-level is a canonical
  projection, not a typed field.** Typing it would change every issue's
  version token — considered and rejected. Do not "fix" it. The `lane` /
  `collision` / `lane_seq` scheduling fields are the contrasting typed case,
  hash-projected only when `Some`. Full reasoning and the DAG semantics:
  [ADR 0003](docs/decisions/0003-frontmatter-field-typing.md) and
  [docs/design/lane-design.md](docs/design/lane-design.md).
- **The CLI default slug is title-derived; random is the opt-in/fallback.**
  `create` derives a kebab slug from the title (`slug::derive_from_title`);
  `--slug-random` gives the random form, which is also the fallback for
  unusable titles; explicit `--slug` stays authoritative. Three collision
  paths, three shapes — do **not** cross-wire them: explicit `--slug` errors
  on collision, the derived default disambiguates with `-2`/`-3` suffixes
  (`claim_derived_slug`), the random path retries internally
  (`claim_random_slug`). `intake file` and recurring occurrences force
  `slug_random` (untrusted/sensitive titles; repeated titles); `import`
  inherits the title-derived default.
- **Doctor `--fix` is forward-progress only** — it never rolls back partial
  progress; scripted callers branch on `stop_phase` (`ok` / `preflight` /
  `post_apply`). Preflight blockers are layout-fatal only, the `--fix --json`
  error envelope has stable codes (`doctor-blocked` / `doctor-partial` /
  `doctor-apply-error`), and schema `required_when` + status/type aliases
  drive coercion. Details: [docs/design/doctor-fix.md](docs/design/doctor-fix.md).
- **Config reads go through `ConfigSource`, not bare `schema::load`.** Every
  mutate entry point and config-taking read path takes `&dyn ConfigSource`;
  the sole implementation is `UncachedConfig` (re-parse per call — fine for a
  short-lived CLI). The trait is the load-site seam for a future cache. New
  read helpers follow the `load_issues_with_warnings_via(root, config)`
  pattern: the `_via` variant takes the config, the no-config alias delegates
  to `UncachedConfig`.
- **Wall-clock time goes through `Clock`, never a bare `Utc::now()`.**
  `SystemClock` for production, `FixedClock` for tests; time-dependent domain
  paths take the clock. **The only legitimate `Utc::now()` in `issuectl-core`
  is inside `SystemClock`** (grep `Local::now()\|Utc::now()` under
  `crates/issuectl-core/src` — exactly one match). Timezone asymmetry is
  deliberate: `SystemClock::today()` uses the local calendar,
  `FixedClock::today()` reads its UTC instant — pin test instants mid-day UTC
  unless the test is specifically about a date boundary.
- **Archived issues live at `issues/archive/YYYY/MM/<slug>/` and are
  repo-resident.** Bucketed by `closed:` (fallback `updated:`). Discovery is
  archive-aware, so `show` / `list` / queries find them transparently; an
  active+archived same slug is `Ambiguous`. A status mutation out of a
  closing status auto-unarchives (under the write flock); empty buckets are
  pruned. `issuectl archive` is **in use in this repo** — run it (default
  `--older-than 90d`) when the active tree accumulates closed issues.
- **The planning-doc-type list lives in the `init-project` skill, not
  here.** That convention is owned upstream; issuectl deliberately does not
  enumerate or enforce it. Do not add it here — let the upstream skill stay
  the single source.
- **Per-issue `attachments/` and `fixtures/` directories** are created on
  demand via `ensure_issue_subdir` (git drops empty dirs). Relative body refs
  resolve relative to the issue dir; the extractor is hardened against `../`
  and backslash traversal. `doctor` warns on large binaries (>1 MiB),
  non-AVIF raster images, and unresolved relative refs. `issuectl attach`
  copies files in, auto-renaming collisions.
- **Body-ref extraction uses pulldown-cmark, not regex** — the CommonMark
  parser skips code spans/blocks for free; do not "optimise" this back to a
  regex. `BodyRef.has_line_anchor` (GitHub-style `#L<n>` fragment) is the
  only gate for doctor's "cross-file code permalink → skip if it exists at
  the repo root" heuristic — an unconditional skip would mask missing
  attachments whose names collide with repo-root files (pinned by
  `broken_refs_still_flags_when_filename_collides_with_repo_root`).
- See [CONTRIBUTING.md](CONTRIBUTING.md) for dev setup, repo layout,
  PR process, and commit-message conventions.
- See [issues/AGENTS.md](issues/AGENTS.md) for how this project's own
  issue tracker is organized.

## Operating policy (for `/stint`)

`/stint` reads this section for how to run a work-session in this repo. The
work queue and session handoff live in [TODO.md](TODO.md); this section is the
project's operating policy.

- **What "deploy" means here.** This is a Rust CLI, not a server. The release
  path is the Shipshape engine (`/shipshape-release` → `shipshape release
  plan|cut`), reading the approved [OSS-RELEASE.md](OSS-RELEASE.md) contract.
  Ensure `[Unreleased]` is complete and `main` is clean, pushed, and green.
  Before a minor or major bump, update and commit the internal dependency
  requirement in `crates/issuectl/Cargo.toml` (the caret boundary gotcha).
  Then run `shipshape release plan --bump patch|minor|major`, inspect the
  sealed plan, and run `shipshape release cut --plan <id>`. The cut owns the
  workspace version bump, Cargo.lock refresh, CHANGELOG finalization, release
  commit, crates.io publishes (`issuectl-core` before `issuectl`), and
  `vX.Y.Z` tag. The tag triggers cargo-dist (`.github/workflows/release.yml`)
  for GitHub-Release binaries, the shell installer, and the Homebrew tap.
  Shipshape plans and verifies those delegated cargo-dist legs; a cut only
  reports complete after the verify barrier observes crates.io receipts,
  GitHub Release assets, and the tap formula. If a cut is interrupted, use
  `shipshape release resume <run_id>`; `shipshape release verify <run_id>` is
  the read-only reconcile. Full steps: [CONTRIBUTING.md](CONTRIBUTING.md)
  "Per-release steps".
- **Post-cut backstop check — permanent, not a temporary measure.** The engine's
  own verdict has now been wrong in **both** directions on real cuts (0.15.0
  false-red: reported failed, everything delivered; 0.16.0 false-calm: `verify`
  correctly said `gh-releases` was missing but gave no cause, and the
  destination alone read the same as "still building"). So always check the
  channels directly: `gh release view vX.Y.Z --json assets --jq '.assets|length'`
  non-zero (compare against the previous tag — the count tracks
  `dist-workspace.toml`'s target/installer set), and the tap formula advanced.
  History: the tap once sat stale through three releases because nobody checked
  (`@homebrew-tap-stale`).
  - **A zero asset count is ambiguous** — "CI still building", "CI died", and
    "CI never ran" look identical from the destination. Resolve it at the
    *delegated run*, not the destination: `gh run list --workflow=release.yml
    --limit 1` then `gh run view <id> --json jobs` for the per-job breakdown. A
    cancelled build job skips every downstream job (`host`,
    `publish-homebrew-formula`), so there is no Release **and** a stale tap.
  - **Recovery for a cancelled/failed dist workflow is `gh run rerun <id>
    --failed`.** The crates.io publish and the tag are already done and are
    irreversible — never re-cut, never re-tag. Re-run only the delegated build.
  - **crates.io's API returns `null` for every field without a `User-Agent`
    header.** `curl -s -H 'User-Agent: <anything>'
    https://crates.io/api/v1/crates/issuectl | jq -r '.crate.max_version'`. A
    bare `curl` will make a successful publish look like a failed one.
- **Homebrew publishing is cargo-dist's, driven by `dist-workspace.toml`**
  (`homebrew` installer + `tap` + `publish-jobs`; `HOMEBREW_TAP_TOKEN` is
  configured on the repo). The contract's `distribution:` block declares this
  delegation so the engine verifies it. **Do not run `shipshape dist generate`**
  without a deliberate decision — it would strip this repo's self-hosted
  macOS ARM64 runner override (`[dist.github-custom-runners]`: fast local
  builds versus a 45+ min hosted-queue allocation), and `/shipshape-dist`
  refuses to emit a runner override at all.
- **Releases MAY be cut automatically whenever there is something to
  release** (maintainer decision, 2026-08-05). When `main` carries unreleased
  user-facing changes, `/stint` may bump, finalize the CHANGELOG, and run the
  release recipe — no confirmation needed. Preconditions: green gate passes,
  dry-run/plan first. crates.io publishes are irreversible (yank-only), so
  never publish red, and report each step.
- **The engine-driven cut is fully autonomous — no go/no-go checkpoint,
  ever** (maintainer decision, 2026-08-06). Do **not** stop to ask "shall I
  cut?" — run the recipe end to end and report as you go. The safety is
  structural: sealed content-addressed plan, `dry-run-all` before any
  publish, dependency-ordered crates.io publish, `resume`/`abandon` recovery,
  and (0.7.0) the verify barrier.
- **Git: `pull --rebase` → `push` is always allowed, no confirmation**
  (maintainer decision, 2026-08-05) whenever `main` is clean and green.
  Never force-push a shared branch; never push a red tree.
- **Live-version check.** Shipped: `git tag --sort=-creatordate | head -1`
  and `grep '^version' Cargo.toml`. Published: crates.io / the Homebrew tap.
  Compare against `main` before recommending a release.
- **Green gate** (must pass before a unit counts as landed):
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cargo build --workspace` (release build not required per-unit)
  - `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — CI runs
    this and it is easy to miss locally: broken intra-doc links fail the
    `docs` job even when tests pass. Run it before landing any unit that
    touches doc comments.
- **Hot files (sequence only when the same file overlaps).** Two worktrees
  editing the same `crates/issuectl/src/cmd/<family>.rs` collide; different
  command families are parallel-safe. The same rule applies to
  `crates/issuectl-core/src/mutate/mod.rs` plus the specific mutation verb
  file, and `crates/issuectl-core/src/doctor/mod.rs` plus the specific doctor
  module. `crates/issuectl-core/src/schema.rs` and the six skill templates
  under `crates/issuectl-core/templates/` remain hot files (the templates are
  kept in sync per the rule above).
- **Test-account reset: n/a.** No external services or test accounts; tests
  are hermetic (`cargo test` uses tempdirs). No reset step.
- **Parallelism preference: launch all disjoint lanes at once.** When the
  DAG's lanes touch no shared hot file, default to spawning them all in
  parallel rather than proposing one lane and waiting. The maintainer favors
  maximal parallelism; sequence only genuine hot-file collisions.
- **A same-titled orchestrator run in a sibling repo is NOT this repo's
  issue.** Cross-repo campaigns spawn similarly-titled runs in each repo.
  Verify which repo a run targets with `git worktree list` and the run's
  working directory — never infer from the run title.

## When in doubt

Run `issuectl --help` and `issuectl <subcommand> --help`. The CLI help is the
source of truth for currently-accepted flags.
