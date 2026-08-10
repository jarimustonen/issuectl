# AGENTS.md

Guidance for AI agents (and humans) working **inside this repo**. The
canonical entry points are below; click through if you need detail.

## What this project is

`issuectl` — AI-first CLI for managing markdown-based issues with
frontmatter. See [README.md](README.md) for a user-facing overview.

## Design principles

All CLI changes must follow [AGENTS-AI-FIRST-CLI.md](AGENTS-AI-FIRST-CLI.md):
strict input validation, structured (`--json`) output, no interactive
prompts, informative errors, composable commands.

## Critical rule: keep the skill in sync with the CLI

The `/issue` skill template is shipped with the binary via
`include_str!` in `src/skill.rs` and installed by `issuectl skill
install`. It tells consumer-side agents how to use this CLI. **It is
the only contract those agents see.**

**Whenever you change the CLI in a way an agent would notice, update
the skill templates in the same commit.** Triggers include:

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
- `issue-new-skill.md` — `/issue-new`, the intake **filing** skill (Claude-only)
- `issue-intake-skill.md` — `/issue-intake`, the intake **processing** skill
  (Claude-only)

They are dogfooded into this repo via `issuectl skill install --agent all
--force`:

| Template | Dogfooded copy |
|---|---|
| `issue-skill.md` | `.claude/skills/issue/SKILL.md` |
| `issue-prompt.md` | `.codex/prompts/issue.md` |
| `issue-new-skill.md` | `.claude/skills/issue-new/SKILL.md` |
| `issue-intake-skill.md` | `.claude/skills/issue-intake/SKILL.md` |

The two intake skills are **Claude-only** — they ship whenever the Claude
agent is selected (`--agent claude` or `all`), have no Codex variant, and are
skipped on a Codex-only install. After editing any template, re-run the install
command so the local copies don't drift from `templates/`. The
`skill::tests::dogfooded_copies_match_templates` test enforces this for **all
four** copies — it fails if a committed copy no longer matches its rendered
template (and `standalone_intake_skills_are_wellformed` additionally pins the
intake skills' filing/processing split). `/triage-bugs` is a repo-local-only
deprecation alias — it is **not** a binary-shipped template.

If the two variants would otherwise drift, regenerate the Codex one
from the Claude one with:

```sh
tail -n +5 templates/issue-skill.md > templates/issue-prompt.md
```

(That strips the Claude-specific YAML frontmatter, leaving the body.)

## Other conventions

- **Always `--json`** when scripting `issuectl` from another tool or
  agent. The human-readable mode is for terminal users.
- **`--json` output contract (the agent-facing contract).** One shape
  across every command so consumers parse them uniformly:
  - **Success (exit 0)** → one JSON value on **stdout**: the resource
    (`show`), an array of resources (`ls`/`search`), or a result object
    (action verbs).
  - **Error (exit ≠ 0, nothing produced)** → one object on **stderr**:
    `{"error":{"code":"<kebab>","message":"…"}}`, with optional extra
    keys inside `error` (e.g. `matches`), and **empty stdout**. The
    bubble-up path in `fn main` wraps a generic `anyhow` error as
    `code:"command-failed"`, but downcasts a mutate-layer `NotFound`
    (raised on a missing slug by the mutate-layer write verbs —
    `update`/`close`/`set`/`note`/`check`/`label`/`depend`/`body set`) to
    `code:"not-found"` so those writes classify a missing issue exactly
    like the read paths (verbs that resolve the slug earlier at the repo
    layer, e.g. `triage`, still report `command-failed`);
    explicit `process::exit` sites use the shared `fail()` helper; clap
    usage errors are caught in `fn main` and re-emitted as
    `code:"usage-error"` (exit 1) — all so the one error shape holds
    regardless of where the failure originates.
  - **Partial success (exit ≠ 0, work landed)** → the command still
    prints its normal **result object on stdout** (e.g. `import` with
    `created`/`failed`), not the error envelope. The non-zero exit is
    the actionable signal; the body carries what happened. Branch on the
    exit code first, then decide which stream to parse.
  - **Exit codes**: `0` success · `2` refused-but-actionable (duplicate
    precheck → error envelope; partial import → result object) · `1`
    everything else (validation, not-found, usage, conflict).
  - **Shared field vocabulary**: `slug`, `title`, `version`, `dir` (issue
    directory), `path` (a file), `dry_run`, `diff`, `warnings`. `open`
    uses `is_dir` (bool) to avoid colliding with the `dir` string field.
  New commands MUST reuse these keys rather than invent synonyms
  (`final_dir`/`issue_dir`/`item_path` were unified to `dir`/`path`).
- **Tests live next to the code** in `#[cfg(test)] mod` blocks by
  default. New features add tests; bug fixes add regression tests.
  - **Exception — `tests/` integration tests:** use only for
    black-box behaviour that no inline test can observe: process
    exit code, byte-level stdout/stderr, argument parsing performed
    by the built binary, and `main()`'s `anyhow::Error` rendering.
    Anything reachable through a `pub(crate)` entry point belongs in
    an inline `#[cfg(test)]` module.
- **New mutation verbs go in `mutate.rs`, CLI handlers stay thin.**
  Every write path (CLI subcommand or web endpoint) routes through a
  function in `src/mutate.rs` so a) every writer obtains the same
  repo-wide `flock`, b) every writer emits the same canonical version
  token, and c) schema validation runs in exactly one place. Add new
  domain logic as a public function in `mutate.rs` (or a sibling
  domain module) and keep the `cmd_*` handler in `main.rs` to
  argument parsing + JSON / human formatting (≤30 lines is the
  target). Do **not** reach into `write::*` directly from `main.rs`
  for new write paths — that bypasses the lock and the schema check.
- **Domain code lives in `issuectl-core`; `issuectl` only owns CLI
  dispatch.** The repo is a Cargo workspace with two crates:
  `crates/issuectl-core` (library) owns every domain module —
  `mutate`, `write`, `repo`, `parser`, `schema`, `body_sections`,
  `query`, `canonical`, `transitions`, `doctor`, `issue_fields`,
  `migrate_layout`, etc. — and `crates/issuectl` (binary) owns
  clap structs, `find_root`, the top-level `cmd_*` handlers, and
  `fn main`. State-changing logic (lock acquisition, schema
  validation, slug claiming, atomic writes) lives in
  `crates/issuectl-core/src/mutate/`. Pure on-disk render/serialize
  primitives live in `crates/issuectl-core/src/write.rs`. Shared
  domain helpers (issue enums like `ISSUE_TYPES`/`PRIORITIES`,
  status classification, ref normalization) live in their own
  domain module (`issue_fields.rs`, `refs.rs`, etc.). The bin and
  lib are **separate crates**: domain modules cannot reach
  `crate::foo` to call something defined in the binary, because
  `crate::` inside the lib resolves to `issuectl-core`'s `lib.rs`.
  If a `mutate::*` or `write::*` site needs a helper, that helper
  belongs in a domain module. The `_pub` re-export wrapper
  anti-pattern is the warning sign that a private root helper is
  leaking. `issuectl-core` is **published but explicitly internal**
  (see its `lib.rs` doc comment) — `pub` items there are *not* a
  semver contract. The semver contract lives in the `issuectl`
  binary's CLI surface.
- **`blocked_by` stays in `extra`; its JSON top-level is a *canonical
  projection*, not a typed field.** Unlike `closed_by` (typed) or
  `related`/`labels` (plain-serialized), `blocked_by` is deliberately
  kept as a raw `extra` map entry: it is folded into `canonical_hash`
  as the raw user-written value, so promoting it to a typed
  `Frontmatter` field would change **every existing issue's version
  token**. `show` / `ls` / `search --json` surface it at top level via
  the shared `project_blocked_by` helper (sorted/deduped/`@`-prefixed
  canonical list; the raw `extra.blocked_by` is stripped so there is one
  wire representation, plus a derived `blocks` reverse view on `show`).
  Do **not** "fix" the historical top-level-`null` shape by typing the
  field — that regression was considered and rejected in
  `@show-json-omits-blocked-by` / `@json-blocked-by-null-top-level`
  (the same conclusion the four-way `closed_by` review reached).
  `@intensely-blushing-galley` is the contrasting case where the hash
  impact of typing *was* acceptable.
- **`lane` / `collision` follow the `closed_by` (typed) precedent, not
  the `blocked_by` (stays-in-`extra`) one.** The two scheduling-DAG
  fields are typed `Option`s on `Issue`, lifted from the raw mapping by
  the parser (a *string* `lane:`, a *list of strings* `collision:`;
  malformed shapes stay in `extra`) and projected into `canonical_hash`
  **only when `Some`** — so an issue that sets neither hashes identically
  to the pre-field shape (pinned by `no_lane_collision_hashes_identically`
  + the unchanged `golden_hash_with_title` vector). No
  `SUPPORTED_SCHEMA_VERSION` bump: they are additive optional fields
  inside the v1 format, and bumping would reject every repo's `version: 1`
  `.schema.yaml`. Both are reserved custom-field keys — the only writers
  are `update --lane` / `--add-collision`. The DAG *computation* lives in
  `crate::dag` (head-of-line + spawnability computed on read, nothing
  stored); `cmd_dag` in `main.rs` stays thin. Reservations are a
  caller-supplied input (`--reservations`), never read from an
  orchestrator — issuectl stays orchestrator-agnostic.
- **Descriptive slugs are derived in the `/issue` skill; the CLI
  default is random.** `issuectl new` emits a random
  `intensifier-adjective-noun` slug when `--slug` is omitted. The
  `--slug` collision error lives only in the explicit-`--slug` arm of
  `do_new`; the random path retries internally in `claim_random_slug`.
- **Doctor `--fix` is forward-progress only.** When the apply
  pipeline mutates the repo (flat-layout migration, status
  reconciliation, notes rename, ...) and a *later* phase finds a new
  critical blocker, doctor bails with the partial progress intact
  rather than rolling back. Rolling back N already-completed renames
  is itself a multi-step operation that can fail mid-rollback. The
  `apply_outcome` JSON envelope carries both the work that landed and
  the new blockers, distinguished by `stop_phase`:
  - `"ok"` — apply ran to completion (`blockers == []`).
  - `"preflight"` — refused to write; no mutations landed
    (`fix_applied: false`, `blockers != []`).
  - `"post_apply"` — partial-progress bail; some writes already
    landed (`fix_applied: true`, `blockers != []`). The user
    resolves the blockers and re-runs `--fix`.
  Scripted callers should branch on `stop_phase` rather than infer
  from `blockers` + `fix_applied`.
- **Preflight blockers are layout-fatal only.** Per-file manual-merge
  findings — `## Notes`/`## Comments` ambiguity, malformed
  `.issuectl/AGENTS.md`, drift-check-skipped — drive exit-1 via
  `critical_blockers` but are NOT in `apply_blockers`. They surface
  through `outcome.notes_conflicts_at_apply` (and the regen-gate on
  AGENTS.md flags inside `DoctorActions::from_findings`) instead of
  aborting the whole pass, so orthogonal auto-fixes (alias coercion,
  AGENTS.md schema-block regen, NN-rename) still run. Adding a new
  finding to `blockers_for(ApplyPreflight)` requires a one-line
  justification that it makes the repo genuinely unsafe for the apply
  pipeline (layout ambiguity, parse failure, symlink risk). See
  `@doctor-fix-noop`.
- **`doctor --fix --json` error envelope is scoped to `--fix`.** On
  non-zero exit, `--fix --json` emits
  `{"error":{"code","message","details"}}` on stderr (stdout empty);
  stable codes are `doctor-blocked` (preflight refusal),
  `doctor-partial` (Ok with manual leftovers, PostApply bail, or
  critical findings remain), `doctor-apply-error` (mid-pipeline
  failure). The full result object is nested under `details` so
  scripts still see what landed. Read-only `--json doctor` keeps the
  historical contract — full result on stdout regardless of exit
  code, so `issuectl --json doctor | jq …` on an unhealthy repo
  continues to work.
- **Config reads go through `ConfigSource`, not bare `schema::load`.**
  Every mutate entry point (`update_issue`, `new_issue`, `update_body`,
  `close_issue`, `note_issue`, `toggle_checkbox`, `do_new`) and every
  server-side read path (`repo::load_issues_with_warnings_via`,
  `boards::load`) takes a `&dyn ConfigSource` parameter. CLI callers
  pass `&UncachedConfig`; server handlers pass `&*state.config` (their
  `Arc<RepoConfigCache>`) into `spawn_blocking`. `schema::load(root)`
  and `transitions::load(root)` are the CLI-uncached fallback — do
  **not** call them from a new server hot path or a new mutate
  helper, or you'll silently bypass the per-request cache (this is
  exactly the regression `@hugely-madly-haircut` was meant to
  eliminate). For new read helpers, follow the
  `load_issues_with_warnings_via(root, config)` pattern: optional
  `_via` variant takes the config; the no-config alias delegates to
  `UncachedConfig` for CLI ergonomics.
- **Schema `required_when` + status/type aliases drive `doctor --fix`
  coercion.** A `FieldSpec.required_when: { status_class: <class> }`
  declares conditional required fields; built-in: `closed` is required
  when status_class is closing. `status_aliases` / `type_aliases`
  (top-level schema keys, per-key merge over built-in defaults) map
  legacy values to canonical ones (closed→done, resolved→fixed,
  refactor→chore, …); only `doctor --fix` consumes them and coerces —
  mutation commands still reject out-of-enum values, and the mutation
  RequiredWhen exemption is scoped to fields a write did **not** touch
  (so explicitly clearing `closed` on a closing-status issue is
  rejected). A coerced legacy status whose `closed:` is unset gets
  stamped from git history (`git log -1 --format=%aI` on `item.md`,
  falling back to mtime, then today).
- **Archived issues live at `issues/archive/YYYY/MM/<slug>/` and are
  repo-resident.** Bucketed by `closed:` (fallback `updated:`).
  `repo.rs` discovery is archive-aware: `discover_slugs` /
  `resolve_layout` treat archived issues as `LayoutState::Flat`
  candidates via a single-walk archive index, so `show` / `list` /
  `locate` / queries find them transparently. An active+archived same
  slug surfaces as `Ambiguous`. A status mutation that takes an
  archived issue out of a closing status auto-unarchives it (renames
  its dir back to the active root under the write flock); empty
  `YYYY/MM[/YYYY]` buckets are pruned.
- **The planning-doc-type list (`plan` / `analysis` / `validation` /
  `design` / `breakdown` / `todo`) lives in the `init-project` skill,
  not here.** That convention is owned upstream (a project-scaffolding
  template); `issuectl-core` deliberately does not enumerate or
  enforce it. Do not add it to issuectl-core or this repo's
  `AGENTS.md` — let the upstream skill stay the single source.
- **Per-issue `attachments/` and `fixtures/` directories.** Created on
  demand via `ensure_issue_subdir` (not eagerly by `issuectl new`,
  since git drops empty dirs). Relative body-image / link targets
  resolve relative to the issue dir; the extractor is hardened
  against `../` and backslash path traversal. `doctor` emits
  warning-only checks for large binaries (>1 MiB), non-AVIF raster
  images, and unresolved relative body refs. The `issuectl attach
  <slug> <file>…` command copies files into `attachments/` (creates
  the dir on demand, handles name collisions).
- **Body-ref extraction uses pulldown-cmark, not regex.** The CommonMark
  parser never emits `Tag::Link` / `Tag::Image` inside code spans or
  fenced/indented code blocks, so prose like `` `![alt](path)` `` is
  filtered for free — do not "optimise" this back to a regex. The
  extractor returns `BodyRef { path, has_line_anchor }`: the flag is
  set when the original URL ended in a GitHub-style `#L<n>` /
  `#L<n>-L<n>` fragment. `doctor`'s `broken_attachment_refs` check is
  the only place that gates the "looks like a cross-file code
  permalink → skip if it exists at the repo root" heuristic on that
  flag. An earlier unconditional repo-root existence skip silently
  masked any missing sibling attachment whose filename collided with a
  repo-root file (`README.md`, `Cargo.toml`, …); the `#L<n>` gate is
  what keeps bare filenames honest — pinned by
  `broken_refs_still_flags_when_filename_collides_with_repo_root`.
- See [CONTRIBUTING.md](CONTRIBUTING.md) for dev setup, repo layout,
  PR process, and commit-message conventions.
- See [issues/AGENTS.md](issues/AGENTS.md) for how this project's own
  issue tracker is organized.

## Operating facts (for /stint)

Facts a `/stint` orchestrator needs. The work queue and session handoff
live in [TODO.md](TODO.md); this section is the project's operating
policy.

- **What "deploy" means here.** This is a Rust CLI, not a server. A
  release is: bump `version` in `Cargo.toml`, move `CHANGELOG.md`
  `[Unreleased]` items to a dated `[X.Y.Z]` section, `git commit -am
  "release: X.Y.Z"`, then `git tag -a vX.Y.Z -m "Release X.Y.Z" &&
  git push --follow-tags`. The tag triggers `cargo-dist`
  (`.github/workflows/release.yml`) → GitHub Release with binaries +
  shell installer, and the Homebrew formula pushed to
  `jarimustonen/homebrew-issuectl`. **crates.io does NOT publish on the
  tag** — `publish-crates.yml` is `workflow_dispatch`-only and
  `GITHUB_TOKEN` does not fire it, so every release needs a **manual
  trigger** (`gh workflow run "Publish to crates.io"`) once the release
  workflow succeeds. (This is the gap `@wire-oss-release-as-release-path`
  will close.) Full steps: [CONTRIBUTING.md](CONTRIBUTING.md)
  "Per-release steps".
- **Deploy/release autonomy: autonomous.** The conductor may cut and
  publish releases **without asking**, and may **decide autonomously
  whether** a release is warranted — bump the version, roll the
  CHANGELOG, commit, tag, `git push --follow-tags`, then fire the
  crates.io manual trigger once `release.yml` is green (see the
  crates.io caveat below). No go/no-go handoff is required. (Granted
  2026-08-10; supersedes the earlier go/no-go rule.) Use judgement on
  *timing* — batch trivial changes, don't tag a red or half-landed
  `main` — but the decision itself is yours. **Autonomous crates.io
  recipe:** after the tag push, background `gh run watch
  <release-run-id> --exit-status && gh workflow run "Publish to
  crates.io" --ref main` so the crates.io publish fires automatically
  the moment `release.yml` goes green — this satisfies the manual-trigger
  caveat without a human in the loop (used for 0.7.2).
- **Live-version check.** Shipped: `git tag --sort=-creatordate | head
  -1` and `grep '^version' Cargo.toml`. Published: crates.io / the
  Homebrew tap. Compare against `main` before recommending a release.
- **Green gate (must pass before recommending a release).** `cargo
  test` (unit + integration), `cargo clippy --all-targets` (no new
  warnings), `cargo fmt --all --check`. CI (`.github/workflows/ci.yml`)
  runs the same on every PR.
- **Hot files (sequence, do not parallelise).** `crates/issuectl/src/main.rs`
  (all `cmd_*` handlers + clap structs), `crates/issuectl-core/src/mutate/`
  (every write path routes here), `crates/issuectl-core/src/schema.rs`,
  and the skill templates (`templates/issue-skill.md`,
  `templates/issue-prompt.md`, `templates/issue-new-skill.md`,
  `templates/issue-intake-skill.md`, kept in sync per the rule above).
  Two worktrees editing any one of these will collide on rebase.
- **Test-account reset: n/a.** No external services or test accounts;
  tests are hermetic (`cargo test` uses tempdirs). No reset step.

## When in doubt

Run `issuectl --help` and `issuectl <subcommand> --help`. The CLI
help is the source of truth for currently-accepted flags.
