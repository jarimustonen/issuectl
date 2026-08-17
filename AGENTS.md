# AGENTS.md

Guidance for AI agents (and humans) working **inside this repo**. The
canonical entry points are below; click through if you need detail.

## What this project is

`issuectl` — AI-first CLI for managing markdown-based issues with
frontmatter. See [README.md](README.md) for a user-facing overview.

## CLI Design Principles

Use the `/ai-first-cli-canon` skill shipped by `project-canon` as the maintained AI-first CLI canon. It is the binding reference for CLI surface work: strict input validation, `--json` output, JSONL logs, no interactive prompts, informative errors and composable commands. Do not keep or edit a repo-local `ai-first-cli-canon` copy; update the canon through the `project-canon` tool and reinstall the released skill.


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

Like `/issue`, each intake skill ships in **both** formats — a Claude skill
(`--agent claude`) and a Codex prompt (`--agent codex`); `all` installs both.
The Codex prompt is the Claude one with its YAML frontmatter stripped (body
identical). After editing any template, re-run the install command so the local
copies don't drift from `templates/`. The
`skill::tests::dogfooded_copies_match_templates` test enforces this for **all
six** copies — it fails if a committed copy no longer matches its rendered
template (and `standalone_intake_skills_are_wellformed` additionally pins the
intake skills' filing/processing split). `/triage-bugs` is a repo-local-only
deprecation alias — it is **not** a binary-shipped template.

### pi.dev dual-home (`~/.pi/agent/skills/`)

Whenever the Claude layout is installed (`skill install`, `--force`, `--agent
all`, or `issuectl init`), each Claude `SKILL.md` is **also** written to pi.dev's
global skill corpus at `~/.pi/agent/skills/<name>/SKILL.md` (`issue`,
`issue-new`, `issue-intake`) so the skills are discoverable under the pi.dev
harness (invoked there as `/skill:<name>`; bare `/name` cross-references resolve
via pi's injected available-skills list, so **only the install target differs —
no body/link rewrite**). The mirror is byte-identical to the repo-local Claude
copy. **Vendored filter: only `SKILL.md` is mirrored** — the Codex prompts and
the `issues/AGENTS.md` scaffold are not, matching homebase `dotfiles link`; a
`--agent codex` install writes no pi copy. Note the asymmetry: the Claude/Codex
targets are **repo-local** (rooted at the target repo), while the pi mirror is
**home-global** (rooted at `$HOME`, resolved by the binary via
`skill::pi_skills_root`; skipped when `$HOME` is unset). Each pi copy is written
independently, so it never gates the repo-local install — a present pi copy
still lets a plain install repair a deleted Claude skill (a non-force run leaves
the pi copy in place; `--force` refreshes it). See `skill.rs`
`install_skill_summary`.

#### pi corpus lifecycle (provenance · drift · prune)

The global pi copies are otherwise unmanaged, so `skill.rs` adds an
observability layer on top of the mirror (issue `pidev-pi-skill-lifecycle`):

- **Provenance manifest.** Every pi mirror pass also writes
  `~/.pi/agent/skills/.issuectl-manifest.json` (`PI_MANIFEST_FILE`) — a
  tool-namespaced JSON map of `skill name → { version }` recording which corpus
  entries issuectl wrote and at which version. This is out-of-band: the
  `SKILL.md` bodies stay byte-identical to the Claude copies, so provenance
  lives in the manifest, not a marker inside the skill. `orchestratectl` keeps
  its **own** `.orchestratectl-manifest.json`; neither tool prunes the other's
  entries. **Provenance follows real write events, never on-disk existence:**
  `record_pi_provenance` records only the skills a run actually created/
  overwrote (threaded via a `written` set from the mirror loop), stamped with
  the running version. A managed-name file that already existed and was left in
  place — a non-force install, or a hand-authored copy — is **not** adopted, so
  `pi-prune` can never later delete a file issuectl did not write. The manifest
  is written atomically (temp + rename); manifest keys are validated to safe
  single path components on load (`is_valid_skill_name`) so a tampered key can't
  steer a delete outside the corpus, and the strict loader refuses a corrupt/
  foreign/unsupported manifest rather than acting on an empty misread of it.
- **`issuectl skill pi-status`** (read-only) classifies every corpus entry:
  `up-to-date` · `stale` (issuectl-owned, on-disk differs, recorded version ≠
  running — a different binary wrote it) · `modified` (differs but recorded
  version == running — hand-edited/corrupted) · `missing` (manifest row, file
  gone) · `orphan` (manifest row for a skill issuectl no longer ships, e.g.
  `/triage-bugs`) · `unmanaged` (on disk, not in the manifest — hand-authored or
  another tool's). Supports `--json`.
- **`issuectl skill pi-prune`** removes `orphan` entries (deletes the mirrored
  `SKILL.md`, drops the dir only if now empty, clears the manifest row) and
  clears `missing` rows. **Dry-run by default; `--force` applies.** It never
  touches `unmanaged` entries and never deletes a *current* skill — a `stale` or
  `modified` copy is refreshed via `skill install --force`, not pruned. Deletion
  is guarded: only a regular-file `SKILL.md` in a dir holding nothing else is
  removed; a symlinked or sibling-laden orphan is reported under `skipped` and
  left for the user. Prune refuses to run at all on a corrupt/untrusted
  manifest.

**Reconciliation policy: always-on-force (not overwrite-only-if-newer).** The
write path is deliberately unchanged — a non-force install leaves an existing pi
copy alone; `--force` overwrites it unconditionally to the running binary's
version. This matches the repo-local Claude/Codex targets exactly (force means
force) and avoids both a surprising "your `--force` did nothing" outcome and
brittle version-ordering at write time. The known cost — an *older* binary's
`--force` can rewrite the global copy to an older version — is handled by making
drift **visible** (`pi-status` flags a recorded-version mismatch) and
**reversible** (re-run `skill install --force` from the newest binary, or
`pi-prune` for orphans), not by guarding the write. Chosen because the pi corpus
is a derived convenience whose ground truth is any repo's current binary; a
write-time newer-only guard would add a second, subtly-different overwrite rule
for one of the several install targets and still couldn't order dev builds
reliably.

The **`uninstall` gap** (no `skill uninstall`, and what it should do with the
shared global pi copy that can't be reference-counted across repos) is a
documented follow-up, out of scope for this lifecycle layer.

If a Claude/Codex pair would otherwise drift, regenerate the Codex one from
the Claude one by stripping its YAML frontmatter:

```sh
tail -n +5 templates/issue-skill.md         > templates/issue-prompt.md
tail -n +6 templates/issue-new-skill.md     > templates/issue-new-prompt.md
tail -n +6 templates/issue-intake-skill.md  > templates/issue-intake-prompt.md
```

(`/issue`'s frontmatter is 4 lines; the intake skills carry an extra
`argument-hint` line, so their frontmatter is 5 lines — hence `+6`.)

## Other conventions

- **Always `--json`** when scripting `issuectl` from another tool or
  agent. The human-readable mode is for terminal users.
- **`--json` output contract (the agent-facing contract).** Every success, including partial success (`import`, exit 2), is `{"schema_version":1,"data":…, "warnings":[]}` on stdout. Read domain fields only from `data`; non-fatal warnings are exclusively top-level `warnings`. Every no-work error is `{"schema_version":1,"error":{"code":"<kebab>","message":"…",…}}` on stderr with empty stdout; this includes bubble-up, not-found, `fail()`, clap usage errors, and doctor `--fix` errors (whose stable `details` remains inside `error`). Read-only doctor remains a stdout result regardless of its exit status, now inside `data`. `schema_version` is the CLI output API version, independent from `SUPPORTED_SCHEMA_VERSION`; bump it only for breaking output changes, never additive fields. `issuectl version --json` reports both `supported_schemas` and bundled `skills[{name,cli_version,schema_version}]`.
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
  - **Two canon-§22 asks are deliberately REJECTED — do not "fix" them.**
    The AI-first CLI canon's §22 (library-first layout) additionally asks
    for a core with **no I/O** and a cli crate named `*-cli`. Both were
    considered and declined (2026-08-16, `@cli-canon-s22`):
    - **I/O stays in `issuectl-core`.** issuectl is a filesystem-backed
      tracker whose markdown files *are* the domain; ~27 core modules
      touch `std::fs` by design. Hiding the disk behind a trait would be
      a full rewrite of core for no testability gain — the tests are
      already hermetic via tempdirs. The §22 rationale (unit-testable
      domain without the CLI shell) is already satisfied: core has **no
      `clap` dependency**, and `Clock` covers the one genuinely
      untestable ambient dependency.
    - **The binary crate stays `issuectl`, not `issuectl-cli`.** It is
      published on crates.io under that name; renaming breaks the
      published name for cosmetic conformance.
    What §22 *did* yield is the `Clock` seam (below).
    See the repo-wide *verify-before-acting* rule below — the finding that
    filed `@cli-canon-s22` was itself wrong.
- **Verify a reported finding against the tree before you act on it — a
  scan, audit, or pre-check is a recommendation, not evidence.** Any
  finding produced from a partial read (a conformance audit, a triage
  pre-scan, a review pass, another agent's summary) states a claim; confirm
  the claim yourself before you lane work off it, scope work down because
  of it, or report its conclusion onward. This has now been wrong in **both
  directions**, so neither a positive nor a negative finding is
  self-certifying:
  - **False positive.** `project-canon review --assume-defaults` filed
    `@cli-canon-s22` reporting *"no `crates/` directory — no core/cli
    split"*. Simply false: the split long predates the audit and core was
    already clap-free. Acting on it would have meant a pointless
    restructure.
  - **False negative.** The triage pre-scan for `@audit-no-user-specifics`
    concluded the public package *"looks clean"*. The audit worker, briefed
    to redo the sweep itself rather than trust the hint, found real
    maintainer-specific defaults and examples. Accepting the pre-scan as
    the result would have shipped them.
  Practical consequence when briefing a worker: tell it to **redo the
  check**, and say explicitly that any prior scan is a hint, not a result.
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
- **`lane` / `collision` / `lane_seq` follow the `closed_by` (typed)
  precedent, not the `blocked_by` (stays-in-`extra`) one.** The
  scheduling-DAG fields are typed `Option`s on `Issue`, lifted from the
  raw mapping by the parser (a *string* `lane:`, a *list of strings*
  `collision:`, an *integer* `lane_seq:`; malformed shapes stay in
  `extra`) and projected into `canonical_hash` **only when `Some`** — so
  an issue that sets none hashes identically to the pre-field shape
  (pinned by `no_lane_collision_hashes_identically` +
  `no_lane_seq_hashes_identically` + the unchanged `golden_hash_with_title`
  vector). No `SUPPORTED_SCHEMA_VERSION` bump: they are additive optional
  fields inside the v1 format, and bumping would reject every repo's
  `version: 1` `.schema.yaml`. All three are reserved custom-field keys —
  the only writers are `update --lane` / `--add-collision` /
  `--lane-seq`. Note `lane`/`collision` are *declared* in
  `DEFAULT_SCHEMA_YAML` but `lane_seq` is **not**: it is numeric and the
  v1 string validator would reject the YAML integer (same reason `commits`
  and `estimate` are undeclared) — so it is instead added to doctor's
  hardcoded known-key list. The DAG *computation* lives in `crate::dag`
  (head-of-line + spawnability computed on read, nothing stored); `cmd_dag`
  in `main.rs` stays thin. Two DAG semantics live only in `crate::dag`,
  not in stored state: an `in-progress` issue **is still `spawnable`**
  (in-progress means *started, not done* — `dag` is consulted only when
  nothing is running, so an in-progress head is an idle, resumable
  candidate that must surface; preventing a double-spawn is the caller's
  reservation responsibility, not the dag's); and the reserved lane value
  **`lane: unlaned`** (`dag::UNLANED`) is a first-class *parallel-safe*
  marker — its members surface as unscheduled, each its own head-of-line
  and independently spawnable (never serialized with each other),
  distinct from an **absent** lane which means "unclassified". Reservations
  are a caller-supplied input (`--reservations`), never read from an
  orchestrator — issuectl stays orchestrator-agnostic.
- **The CLI default slug is title-derived; random is the opt-in/
  fallback.** `issuectl create "<title>"` with no `--slug` derives a
  descriptive 2–3 word kebab slug from the title (the pure
  `slug::derive_from_title` helper in `issuectl-core`), lowercasing,
  stripping stop-words, and trimming to a clean slug. The random
  `intensifier-adjective-noun` form is reachable explicitly via
  `--slug-random` and is the automatic fallback when the title yields
  no sensible slug (empty/all-stop-words/non-ASCII). `--slug <x>` stays
  authoritative when passed. Three collision paths, three shapes, do
  **not** cross-wire them: the explicit-`--slug` arm of `do_new` errors
  on collision; the derived-default path disambiguates silently with a
  numeric suffix (`-2`, `-3`, …) in its own `claim_derived_slug` loop;
  the random path retries internally in `claim_random_slug`. The
  non-`create` programmatic callers choose deliberately: `intake file` and
  recurring occurrences force `slug_random` (untrusted/sensitive titles;
  many occurrences of one title), while `import` inherits the
  title-derived default.
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
  `close_issue`, `note_issue`, `toggle_checkbox`, `do_new`) and the
  config-taking read paths (`repo::load_issues_with_warnings_via`,
  `repo::load_issues_with_config`) take a `&dyn ConfigSource`
  parameter. The sole implementation is `UncachedConfig` (re-parse on
  every call — fine for a short-lived CLI process); callers pass
  `&UncachedConfig`. `schema::load(root)` and `transitions::load(root)`
  are the uncached value carriers behind it. The trait is kept as the
  load-site seam so a future caching implementation can be slotted in
  without re-threading every signature. For new read helpers, follow
  the `load_issues_with_warnings_via(root, config)` pattern: the `_via`
  variant takes the config; the no-config alias delegates to
  `UncachedConfig` for CLI ergonomics.
- **Wall-clock time goes through `Clock`, never a bare `Utc::now()`.**
  `clock.rs` defines the `Clock` trait (`now_utc` / `today` /
  `today_string`) with `SystemClock` for production and `FixedClock` for
  deterministic tests — the same load-site-seam idiom as `ConfigSource`
  above. Time-dependent domain paths (`write`, `doctor`, `stale`,
  `query`, `report`, `recurrence`, `cycle`) take the clock rather than
  reading the system time themselves, so date-derived behaviour —
  `closed:`/`updated:` stamping, `issues/archive/YYYY/MM/` bucketing,
  doctor's today-fallback when stamping a coerced legacy status,
  staleness windows — is pinnable in tests. **The only legitimate
  `Utc::now()` in `issuectl-core` is inside `SystemClock`**; a new one
  anywhere else is the warning sign that a path skipped the seam (grep
  `Local::now()\|Utc::now()` under `crates/issuectl-core/src` — it should
  match exactly once).
  - **Timezone asymmetry, deliberate:** `SystemClock::today()` converts
    to `Local` before taking the date (persisted `closed:` / `updated:`
    values historically use the local calendar), while `FixedClock::today()`
    takes the date straight off its UTC instant. So a `FixedClock` pinned
    near midnight UTC does not necessarily reproduce what `SystemClock`
    would report in a non-UTC zone. Pin test instants mid-day UTC unless
    the test is specifically about a date boundary, and construct the
    boundary case deliberately rather than assuming the two agree.
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
  demand via `ensure_issue_subdir` (not eagerly by `issuectl create`,
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

## Operating policy (for `/stint`)

`/stint` reads this section for how to run a work-session in this repo. The work queue and session handoff
live in [TODO.md](TODO.md); this section is the project's operating
policy.

- **What "deploy" means here.** This is a Rust CLI, not a server. The release
  path is the `ossctl` engine (`/oss-release` → `ossctl release plan|cut`),
  reading the approved [OSS-RELEASE.md](OSS-RELEASE.md) contract. Steps:
  **(1)** bump `version` in root `Cargo.toml` `[workspace.package]` (and, on a
  minor/major boundary only — the caret gotcha — the internal dep `version` in
  `crates/issuectl/Cargo.toml`), `cargo update --workspace` to sync
  `Cargo.lock`; **(2)** finalize `CHANGELOG.md` `[Unreleased] → [X.Y.Z] -
  <date>`; **(3)** `git commit -am "release: X.Y.Z"`; **(4)** `ossctl release
  plan` (seals a content-addressed plan; phases `dry-run-all →
  build-all → publish-all → tag → dist`), then `ossctl release cut --plan <id>`.
  Neither takes a `--version` flag (0.6.1); **do the bump by hand in steps 1–3
  and never pass `plan --bump`** — see the gotcha bullet below.
  `ossctl release cut` publishes the version already in
  `Cargo.toml` (that is why the bump + CHANGELOG finalize are the `release:`
  commit in steps 1–3, before the cut); it owns the **crates.io publish** of
  both crates (`issuectl-core` before `issuectl`, adapter `cargo-publish`) and
  the `vX.Y.Z` tag. If a cut is interrupted, `ossctl release resume`; `ossctl
  release verify` is a read-only reconcile against the registry. The tag then
  triggers
  `cargo-dist` (`.github/workflows/release.yml`) → GitHub Release with
  cross-platform binaries + shell/powershell installers, while the same tag
  also triggers `.github/workflows/publish-crates.yml` for an idempotent
  crates.io publish in dependency order (`issuectl-core` first, then
  `issuectl`). `release.yml` is a **binary-only backend** — it does NOT
  publish to crates.io, so there is no double-publish inside cargo-dist; the
  separate crates workflow tolerates versions that the engine already
  published. Full steps: [CONTRIBUTING.md](CONTRIBUTING.md) "Per-release steps".
- **Two ossctl 0.6.1 release gotchas, both verified on the 0.14.1 cut. A cut is
  NOT done when the engine prints `release complete` — verify the targets.**
  - **Never use `ossctl release plan --bump <level>`.** It seals a plan that
    `release cut` always rejects as `plan_stale`, because the staleness check
    recomputes the hash *without* the bump. Worse, the rejection names a
    `current_plan_id` that is the **no-bump** plan — following it attempts to
    **republish the version already on the registry**. On 0.14.1 that attempt
    only failed safely because ossctl compares the registry artifact's sha256
    against the one it would upload and refuses to skip. Bump by hand (steps
    1–3), then `plan` with no flags. Upstream: ossctl
    `@release-bump-plan-uncuttable`.
  - **The `tag` phase pre-creates the GitHub Release, which breaks cargo-dist.**
    cargo-dist's `host` job then fails with `a release with the same tag name
    already exists`, and everything downstream — **including
    `publish-homebrew-formula`** — is skipped, while the cut still reports every
    phase green. Recovery: delete the asset-less release object (`gh release
    delete vX.Y.Z --yes`; the git tag is untouched), then `gh run rerun <id>
    --failed`. Upstream: ossctl `@release-tag-preempts-cargo-dist`.
  - **Post-cut verification is mandatory**, since the failure above is silent:
    `gh release view vX.Y.Z --json assets --jq '.assets|length'` must be
    **non-zero** (compare against the previous tag rather than a fixed number —
    the count tracks `dist-workspace.toml`'s target/installer set, which
    changes: Windows + the powershell installer were dropped 2026-08-17), and
    confirm the Homebrew tap formula advanced to the new version. The tap sat
    at 0.11.0 through three releases because nobody checked
    (`@homebrew-tap-stale`).
- **Homebrew publishing is cargo-dist's, driven by `dist-workspace.toml`.** The
  `homebrew` installer + `tap` + `publish-jobs = ["homebrew"]` live there, and
  `HOMEBREW_TAP_TOKEN` is configured on the repo. ossctl's own homebrew leg is
  **inert**: `OSS-RELEASE.md` has no `distribution` block, so `ossctl contract
  show` reports `homebrew_tap: null`. **Do not run `ossctl dist generate`** to
  "fix" that without a deliberate decision — it would strip this repo's
  self-hosted macOS ARM64 runner override (`[dist.github-custom-runners]`,
  the `hauis` runner: ~67 s macOS builds versus the 45+ min hosted-queue
  allocation that motivated it), and `/oss-dist` additionally refuses to emit a
  runner override at all.
- **Releases MAY be cut automatically whenever there is something to release** (maintainer
  decision, 2026-08-05). Publishing `issuectl` itself (crates.io / GitHub Release / Homebrew)
  no longer requires an explicit per-release go: when `main` carries unreleased user-facing
  changes, `/stint` may bump the version, finalize the CHANGELOG, and run the release recipe
  as an owned Phase-3 act — no confirmation needed. Preconditions still hold: the green gate
  passes, and `cargo publish` runs `--dry-run` first. crates.io publishes are irreversible
  (yank-only), so never publish red, and report each step.
- **The ENGINE-DRIVEN cut (`ossctl release cut`) is fully autonomous — NO go/no-go checkpoint,
  ever** (maintainer decision, 2026-08-06). Running the release *through the engine* — the full
  multi-target flow (crates.io ×2 + cargo-dist binaries + the Homebrew tap) — requires **no
  permission and no pause before the irreversible publish**, not for the first-ever engine cut,
  not for the homebrew leg (the homebrew leg is the most important target — it must be cut, not
  dropped). Do **not** stop to ask "shall I cut?" — just run the recipe end to end and report
  as you go. The safety is structural, not a human gate: `ossctl release plan` seals a
  content-addressed plan (a side-effect-free preview the agent inspects), the coordinator runs
  `dry-run-all` before any publish, `issuectl-core`→`issuectl` ordering + index-wait guard the
  crates.io partial-publish case, and `ossctl release resume`/`abandon` recover an interrupted
  run. Still: green gate first, dry-run/plan first, never publish red, report each phase.
- **Git: `pull --rebase` → `push` is always allowed, no confirmation** (maintainer
  decision, 2026-08-05). On this repo the agent may run the pull-rebase-push sequence
  (`git pull --rebase origin main` then `git push origin main`, and pushing tags) on its own
  whenever `main` is clean and green — publishing commits to the remote does not need a
  separate go. Still: never force-push a shared branch, and never push a red tree.
- **Live-version check.** Shipped: `git tag --sort=-creatordate | head
  -1` and `grep '^version' Cargo.toml`. Published: crates.io / the
  Homebrew tap. Compare against `main` before recommending a release.
- **Green gate** (must pass before a unit counts as landed):
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cargo build --workspace` (release build not required per-unit)
  - `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — **CI runs this and it
    is easy to miss locally**: broken intra-doc links (`[`Foo`]` to a moved/renamed/private
    item, redundant explicit link targets) fail the `docs` job even when tests pass. Run it
    before landing any unit that touches doc comments (`//!` / `///`).
- **Hot files (sequence, do not parallelise).** `crates/issuectl/src/main.rs`
  (all `cmd_*` handlers + clap structs), `crates/issuectl-core/src/mutate/`
  (every write path routes here), `crates/issuectl-core/src/schema.rs`,
  and the skill templates (`templates/issue-skill.md`,
  `templates/issue-prompt.md`, `templates/issue-new-skill.md`,
  `templates/issue-new-prompt.md`, `templates/issue-intake-skill.md`,
  `templates/issue-intake-prompt.md`, kept in sync per the rule above).
  Two worktrees editing any one of these will collide on rebase.
- **Test-account reset: n/a.** No external services or test accounts;
  tests are hermetic (`cargo test` uses tempdirs). No reset step.
- **Parallelism preference: launch all disjoint lanes at once.** When the
  DAG's lanes touch no shared hot file, default to spawning them all in
  parallel rather than proposing one lane and waiting — and don't hold back
  parked / low-priority (`build-only-if`) items when the user asks to "run
  everything". The user favors maximal parallelism; sequence only genuine
  hot-file collisions.
- **A same-titled `orchestratectl` run in a sibling repo is NOT this repo's
  issue.** Cross-repo campaigns (e.g. the pi.dev-migration WS4 "propagate to
  every binary-owned skill installer") spawn a *separately-titled-alike* run in
  each binary's repo. Verify which repo a run targets with `git worktree list`
  (in this repo) and the run's tmux-pane cwd — **never infer from the run
  title**. A live `dual-home-skills` worktree under `orchestratectl__worktrees/`
  does **not** mean issuectl's `pidev-dual-home-skills` is being worked; each
  binary needs its own worktree here. (Learned the hard way twice, 2026-08-11.)

## When in doubt

Run `issuectl --help` and `issuectl <subcommand> --help`. The CLI
help is the source of truth for currently-accepted flags.
