# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `issuectl bulk '<query>'` applies one mutation to every issue matching a
  query (same syntax as `ls`/`search`), in a single batch the user commits
  together. Supports `--set key=value` / `--clear key` (built-in fields route
  through their typed slots, anything else is a custom field) plus
  `--add-label` / `--remove-label` / `--add-related` / `--remove-related`.
  `--dry-run` prints the affected slugs and a per-issue unified diff without
  writing. Real runs first validate every target as a dry-run, so a bad value
  aborts the whole batch before any file is written.

### Changed
- `issuectl new --slug <existing>` now fails with an actionable error that
  names the colliding slug and path and suggests retrying with a different
  `--slug` or omitting it for a random auto-generated one (was a terse
  `target directory already exists: <path>`).
- `/issue` skill now derives a descriptive 2-3 word `--slug` from the issue
  title on create, falling back to the random `intensifier-adjective-noun`
  slug only when no obvious short slug exists. The CLI default (random slug
  when `--slug` is omitted) is unchanged.

## [0.5.2] - 2026-05-13

Bug-fix and quality-of-life release driven by the 3DBear monorepo
adoption feedback plus an internals cleanup wave. No breaking changes.
Highlights:

- One-command bootstrap: `issuectl init`.
- Git-native commit linking: `Refs-Issue:` / `Fixes-Issue:` trailers
  + `issuectl sync-commits`.
- `doctor --fix` no longer all-or-nothing — flat-layout migration
  runs regardless of unrelated schema findings, and
  `issues/.schema.yaml` bootstrap is unconditional.
- Frontmatter parsing hardened against fenced YAML in bodies; short
  hashes (`315194e2`) are always quoted; UTF-8 titles render cleanly
  in `issuectl list`; `agents init` logs which schema it used; doctor
  flags gitignored canonical files.
- Internals: thread-local config cache replaced with explicit
  injection; `parse_section` returns structured diagnostics; PATCH/PUT
  writes get `AbortController`/timeout; canonical version tokens carry
  a `sha256:v1:` scheme marker.

### Added
- `issuectl init` — one-command bootstrap for a fresh repo. Runs the
  schema scaffold, `.issuectl/AGENTS.md`, and the `/issue` skill
  (Claude + Codex by default; override with `--agent`). Pre-commit
  hook (`--with-hooks`) and YAML merge driver (`--with-merge-driver`)
  are opt-in. Idempotent: re-running on an already-initialized repo
  reports each step as "already exists" and exits 0. `--json` emits
  a structured `steps[]` summary with per-artifact status, typed
  effects (e.g. `git_config` writes), and machine-parseable
  `next_steps[]`. `--force` regenerates the managed block in
  `.issuectl/AGENTS.md` while preserving user prose, and overrides
  refusal to clobber an existing differing
  `merge.issuectl-yaml.driver` git-config value. Quotes the binary
  path in the merge-driver invocation so installs survive paths with
  spaces. (#totally-protective-wing)
- `issuectl sync-commits` walks `git log <range>` (default
  `<merge-base of HEAD and main/master>..HEAD`) and appends commits
  to each issue's `commits[]` based on `Refs-Issue:` / `Fixes-Issue:`
  trailers in the message body. Idempotent — `write::add_commit` now
  skips entries whose hash (or hex-prefix-equivalent abbreviation)
  is already present, so re-running the same range is a no-op.
  `--dry-run` previews the plan; `--no-branch-fallback` disables the
  implicit "branch named after a known slug → attribute commits to
  that slug" attribution. `Fixes-Issue:` triggers a stderr `Hint:`
  suggesting `issuectl close <slug>` rather than auto-transitioning.
  The opt-in pre-commit hook also prints a non-blocking reminder
  when the current branch resolves to a known slug, nudging the
  user toward `Refs-Issue:` trailers. (#strikingly-absorbing-cows)
- `issuectl agents init` now logs which schema source it used —
  `Using project schema at issues/.schema.yaml.` or `Using built-in
  default schema (issues/.schema.yaml not found).` — so the silent
  default-fallback path is no longer invisible. `--json` includes a
  `schema_source: "default" | "project"` field in the same envelope.
  (#eminently-dramatic-anger)
- `issuectl doctor` warns when canonical issuectl-tracked files
  (`.issuectl/AGENTS.md`, `issues/.schema.yaml`) match a `.gitignore`
  pattern. Asymmetric footgun: works locally, fails for teammates and
  CI. Surfaced in `--json` as `gitignored_paths: [...]`.
  (#simply-workable-umbrella)

### Changed
- `commits[].hash` quoting is now applied at the AST level (only to
  the `commits` sequence), not via post-serialization line rewriting.
  Top-level / nested user `extra` fields named `hash` are no longer
  silently coerced to strings.
- `read_item` now goes through the strict shared splitter
  (`item_text::split`); previously it kept its own naive scanner that
  could disagree with the parser/doctor and corrupt files on write.
- `doctor` no longer passes `--no-index` to `git check-ignore`, so
  tracked files matching a `.gitignore` pattern are not falsely
  flagged as "agents on other machines won't see it".
- `item_text::split` strips a leading UTF-8 BOM so editor-prepended
  `\u{feff}` no longer breaks frontmatter detection.
- `agents init` schema-source detection is now a typed `SchemaSource`
  enum tested directly; the previous string-compare branched
  silently on unknown variants.
- `truncate` returns the empty string for `max_len = 0` instead of
  `"…"`; documented that the helper counts scalar values, not
  terminal-display columns.
- `issuectl doctor --fix` no longer refuses the flat-layout migration
  when schema-shape findings (schema violations, broken cross-refs,
  dependency cycles, status/timestamp consistency) are present.
  Layout migration is a directory rename and is independent of
  frontmatter content; gating it on schema cleanliness was the
  largest single adoption blocker reported in 3DBear 0.5.1 feedback
  (240 dirs need layout migration, 216 schema violations — `--fix`
  would refuse until every violation was hand-fixed first against the
  pre-migration layout). Schema findings still drive exit-1 so they
  remain visible as forward work, surfaced against post-migration
  paths. Layout-fatal preflight blockers (flat-layout conflicts,
  duplicate slugs, slug present in both legacy folders, conflict
  markers, unparseable frontmatter, missing `item.md`, symlinked
  dirs, `## Notes` / `## Comments` ambiguity, malformed AGENTS.md,
  schema parse error) still bail with no writes. (issue:
  `@staggeringly-important-zoo`)
- `issuectl doctor --fix` now bootstraps `issues/.schema.yaml`
  unconditionally, even when other preflight blockers refuse the
  rest of the apply pipeline. The read-only output already
  advertised auto-creation on first `--fix`; gating bootstrap on an
  empty blocker list broke that promise. The operation is
  idempotent. (issue: `@unreasonably-attractive-star`)
- **JSON envelope:** `--json --fix` runs that hit a preflight
  refusal can now report `apply_outcome.fix_applied: true` when only
  the schema bootstrap landed. The `(stop_phase: "preflight",
  fix_applied: true, schema_bootstrapped: true)` combination is
  intentional and reflects the unconditional bootstrap behaviour.
  Scripted callers that previously read `fix_applied` as "every
  pending fix ran" should branch on `stop_phase` instead — `"ok"`
  remains the only phase that means "no blockers, pipeline
  completed".
- `issuectl doctor` collapses long warning lists (more than ten
  entries) to a one-line count by default. Re-run with `--verbose`
  to print the full list. The previous behaviour filled the screen
  with the same 240-line layout-migration list every iteration of
  "fix-something-rerun-doctor" loops. (issue:
  `@ridiculously-outrageous-fold`)

### Fixed
- `issuectl list` no longer panics on non-ASCII titles. The table
  truncation helper now operates on Unicode scalar values instead of
  byte indices, so titles like `Käyttäjän kirjautuminen…` render
  cleanly. Regression of the 0.3.1 byte-boundary issue that resurfaced
  when the table renderer was refactored.
  (#marginally-receptive-kettle)
- Frontmatter splitter is now fence-aware: a `---` line inside a
  fenced code block (` ```yaml ... ``` `) can no longer be mistaken
  for the closing frontmatter marker. Doctor's "unknown frontmatter
  key" warnings no longer fire on `shortname:` / `course_id:` /
  similar lines that live inside body code blocks. Routed all callers
  through the strict shared splitter in `item_text` so the reader,
  writer, formatter, and merge driver all agree on the same boundary.
  (#virtually-callous-rainstorm)
- Frontmatter writes now force-quote `commits[].hash` values so YAML
  1.2 implicit typing cannot coerce a short hash like `315194e2`
  into a float (`31519400.0`). Previously such hashes round-tripped
  losslessly only by accident of `serde_yaml`'s emission heuristics;
  the contract is now explicit.
  (#thoroughly-kaput-pocket)

### Internals
- Replaced the thread-local `RepoConfigCache` activation slot
  (`repo_config::enter` / `current` / `ActiveGuard`) with explicit
  dependency injection. Every mutate entry point
  (`update_issue`, `new_issue`, `update_body`, `close_issue`,
  `note_issue`, `toggle_checkbox`, `do_new`, `boards::load`) and
  the per-request server read path
  (`repo::load_issues_with_warnings_via`) now takes a
  `&dyn ConfigSource` parameter. The CLI passes `&UncachedConfig`;
  the server passes its `Arc<RepoConfigCache>` directly into
  `spawn_blocking`. Removes the `!Send` ambient guard, the
  spawn-blocking worker-reuse footgun, and the
  thread-local-vs-static accident risk; the cache now reaches the
  load site through the type signature so the failure mode for
  "forgot to install" is a compile error rather than a silent
  fallback to uncached parsing. `schema::load` and
  `transitions::load` no longer consult any cache and always
  re-parse — they remain as the CLI default; callers that want
  caching go through `ConfigSource::schema` / `::rules`.
  (#hugely-madly-haircut)
- `body_sections::parse_section` now returns a structured
  `ParsedSection { found, blocks, warnings }` (with a
  `duplicate_section_count()` accessor) instead of a bare
  `Vec<Block>`. The previous shape collapsed five distinct outcomes —
  section absent, present-but-empty, all-headings-malformed,
  swallowed-by-unclosed-fence, duplicate sections — into a single
  empty vec, which sister tickets (`decide`, `agent-run`) cannot
  tell apart. `ParseWarning::{MalformedBlockHeading, UnclosedFence,
  DuplicateSection}` carries the diagnostics. `MalformedBlockHeading`
  also carries `folded_into_previous_block: bool` so consumers can
  distinguish "content was preserved in the prior block's body" from
  "content orphaned before the first valid block." Success-case
  behaviour is unchanged: `.blocks` matches the prior return value
  byte-for-byte and a missing section still produces no warnings.
  (#totally-placid-push)
- Web client (`board.js`): PATCH and PUT write requests now run with
  an `AbortController` and a 30 s timeout each. A hung server
  previously left `pending_writes[slug] > 0` forever and queued every
  same-slug SSE event in `deferred_events` until the user reloaded;
  on timeout the request now aborts, the queue drains, and the user
  gets a "timed out" toast / save-status. A `pagehide` listener
  also aborts every outstanding fetch so a tab close stops the client
  from waiting on a response it can't act on. **Note:** aborting the
  client fetch does not stop the server: if the axum handler already
  received the request body and started a `spawn_blocking` mutation,
  that write will still land on disk. The aborted client just stops
  waiting and drains its pending-write state. (#absolutely-aberrant-caption)
- `parser::deser_epic` now strict-errors on malformed `epic:` shapes
  (sequence, mapping, bool, tagged value, empty string) instead of
  silently coercing them to `None`. The malformed value previously
  flowed through to disk via the raw mapping but never reached
  `canonical_hash`, leaving a blind spot for optimistic concurrency.
  `null` / absent / non-empty string / legacy numeric (for
  `doctor --fix` migration) remain accepted. Wraps through the existing
  typed-frontmatter fallback, so affected files surface as
  `fm_typed_error` and route to `MutateError::Corrupt`.
  (#especially-bumpy-way)
- Documented the watcher stale-snapshot race in
  `parse_slug_state` (concurrent PATCH bursts can publish V1 after
  V2 at a higher seq). Recovery is not spontaneous: the cached
  client state sticks at V1 until the user's next mutation hits a
  409 carrying the server's current state (which the SPA's
  conflict-recovery path re-syncs), or another filesystem event
  re-publishes. Decision is to monitor rather than fix preemptively
  — the window is narrow and local-loopback single-user usage rarely
  tickles it. Mitigations (hub-level version dedup, client-side
  recent-version cache, read-flock around parse) are catalogued in
  the doc comment. (#incredibly-real-hour)
- Canonical version tokens now carry a scheme marker:
  `sha256:v1:<64hex>` instead of `sha256:<64hex>`. Tokens are still
  compared as opaque strings on the hot path; the marker exists for
  forensics (logs and bug reports can distinguish schemes at a
  glance) and as the foundation for a later `classify(token)` helper
  if/when a v2 transition needs a typed "old-scheme" error path.
  **Deploy note:** existing browser sessions hold pre-upgrade tokens
  of the form `sha256:<hex>`; the first write attempt after upgrade
  will see a 409 `VersionMismatch`. The SPA's existing conflict
  handler refreshes from the server's `current` payload, so users
  experience this as a one-time "this issue changed externally"
  toast on their first save, not as lost work. Operators rolling
  out the new binary during active sessions should expect this
  one-shot 409. (#singularly-melodic-haircut)

## [0.5.1] - 2026-05-10

Cleanup release for the `issues/AGENTS.md` scaffold and skill version
hygiene. No behavior change in the CLI core.

### Changed
- `issues/AGENTS.md` template (installed by `issuectl skill install`)
  rewritten as a minimal pointer to the `/issue` skill and
  `.issuectl/AGENTS.md`. The pre-v0.5.0 scaffold described the
  numbered `<NN>-<slug>/` layout with `open/`/`closed/` subdirs and a
  sequential numbering section — none of which apply since v0.2.0
  (slugs) and v0.5.0 (flat layout). The new template is a few lines
  pointing readers at the supported tooling.
- `/issue` skill and Codex prompt templates now include a version-check
  section. The on-disk skill is pinned to the `issuectl` version that
  wrote it (`{{ISSUECTL_VERSION}}` substituted at install time); on
  first invocation in a session the agent runs `issuectl --version`
  and prompts the user to upgrade if the runtime is older. `skill
  install` and `skill print` both render the template with the current
  version.

### Added
- `issuectl doctor` flags a stale `issues/AGENTS.md` (pre-v0.5.0
  scaffold detected via legacy markers — `## Issue Numbering`,
  `open/<NN>` paths, etc.) and `--fix` rewrites it with the current
  pointer template. Customized `issues/AGENTS.md` files without legacy
  markers are left alone. Reflected in `--json` as
  `legacy_issues_agents_md` (read-only flag) and
  `apply_outcome.issues_agents_md_rewritten` (post-fix).
- `issuectl doctor` reports when `.issuectl/AGENTS.md` is absent so
  users can opt in via `issuectl agents init` (informational only;
  `--fix` does not create the file because the policy is opt-in).
  Reflected in `--json` as `agents_md_missing`.

## [0.5.0] - 2026-05-10

The "writable, agent-safe kanban" release. The web board moves from
read-only browser to a full editing surface; the CLI gains a
mutation/validation toolkit (doctor, fmt, set/note/check/label/apply,
transition rules, schema, AGENTS.md); query, search, and `?q=` share
one language; and the file format is hardened so concurrent edits via
web, CLI, `$EDITOR`, and `git pull` all converge.

**Breaking change:** repos using the legacy `issues/{open,closed}/<slug>/`
layout must run `issuectl doctor --fix` once. The new canonical layout
is flat — `issues/<slug>/item.md` — with status carried only in
frontmatter. `IssueSummary` (returned from `GET /api/issues` and
embedded in SSE `IssueUpserted` events) gained a non-optional `version`
field; consumers using `deny_unknown_fields` need to add it. The
`renumber` flow and `<NN>-` prefixes are gone.

### Added — Web edit/sync
- **Writable web kanban.** Drag-and-drop between status columns
  (closing-status picker on drop into Closed), inline frontmatter
  edits, and a body editor (textarea + preview, localStorage drafts,
  three-way merge UI on conflict). Optimistic UI with revert-on-failure
  and toast notifications.
- **Live updates.** Server-Sent Events (`/events`) push board mutations
  to all open browsers. EventHub (parking_lot mutex over seq+ring),
  notify-debouncer-full file watcher with consecutive-failure backoff,
  Last-Event-ID resume with scan-on-lagged semantics, `--watch-poll-ms`
  fallback, and a `Degraded` banner when the watcher gives up.
- **Concurrency-safe writes.** Every mutation goes through one shared
  `mutate.rs` (CLI and server), guarded by an `flock` on
  `.issuectl/write.lock`, with `expected_version` optimistic
  concurrency. CSRF + Host-header validation on the HTTP surface.

### Added — Agent-safe CLI
- **`issuectl doctor`.** Full validation suite (invalid slugs,
  duplicates, missing item.md, orphan epic refs, frontmatter parse
  errors, schema/transition violations) with `--fix` for the legacy
  layout migration and installable git hooks. Single-pass scanner;
  structural Findings → Actions → ApplyOutcome pipeline with
  `stop_phase` discriminator and partial-outcome preservation on
  mid-run failures. Non-zero exit on apply errors.
- **Focused mutation verbs.** `set`, `note`, `check`, `label`, and
  transactional `apply <patch.yaml>` (multi-field + body ops under one
  flock with rollback). All support `--dry-run` (unified diff, no
  write) and require `--expected-version` with `--json`. `note` writes
  timestamped blocks into standardized body sections (Comments /
  Decisions / Agent Runs / Reopen Notes); idempotent `set_checkbox`.
- **`issuectl fmt`** — canonicalize frontmatter ordering and YAML
  style. Optional YAML merge driver to make `git pull` collapse
  reorder-only conflicts cleanly.
- **Status transition rules.** `.issuectl/transitions.yaml` declares
  legal status edges; `update`/`set`/web all enforce them. Per-type
  body section linting rejects `--type` changes that leave required
  sections missing, with a hint listing the headings to add. CLI
  accepts custom statuses defined by `.schema.yaml`.

### Added — Discoverability & agent integration
- **Shared query engine** used by `issuectl ls`, `issuectl search`, and
  the web `/api/issues?q=` endpoint. Syntax: `field:value`,
  `-field:value` (negation), `text:"phrase"`, bareword (treated as
  `text:`), `field:any`/`field:none`, and relative dates
  (`updated:<-14d` strict, `<=-14d` inclusive, anchor: today in local
  timezone). Backslash-escapes (`\:`, `\\`, `\ `, `\"`, `\-`) inside
  unquoted values. Multiple terms AND together; no OR/parens in v1.
  The HTTP `?q=` endpoint enforces a 4096-byte / 64-term cap and
  surfaces parse errors as a JSON error envelope. Existing flag-based
  `ls` invocations remain backwards-compatible — `ls -s fixed` still
  implies open-only unless `--all`/`--closed` is explicit; only a
  *positional* query opts out of the open default.
- **`issuectl context <slug>`** — deterministic agent context bundle
  (issue + parent epic + blockers + related + acceptance criteria +
  recorded commits + schema rules) as markdown or JSON. JSON includes
  the same `version` token as `show --json` for one-shot
  `--expected-version` use. Cache to `.issuectl/cache/agent/<slug>/`
  with `--write`. Read-only.
- **`issuectl prompt <template> <slug>`** — repo-local prompt
  templates at `.issuectl/prompts/<name>.md` with `{{key}}`
  substitution against the context bundle. Any `## H2` heading in the
  body is reachable via its snake-cased name.
- **`.schema.yaml`** for declaring required/optional frontmatter
  fields, custom field types, custom statuses, and per-type required
  body sections. Lifecycle classification (open vs. closing status) is
  derived from the schema, not hard-coded.
- **`.issuectl/AGENTS.md`** — committed agent policy file.

### Changed
- **Repo layout is flat:** `issues/<slug>/item.md`. Status lives only
  in frontmatter; the `open/` and `closed/` subdirs are gone. The
  `folder` reported in `--json` is computed from status. `doctor --fix`
  performs the migration and rewrites `epic:`, `related:`, and
  `blocked_by:` legacy refs alongside `#NN` body refs to `@<slug>`.
- **Workspace split** into `issuectl` (binary) and `issuectl-core`
  (library) with constants relocation, centralized custom-field-key
  validation, and `do_new_locked` extracted from `main.rs` into a
  domain module. CLI golden-test harness for error output.
- `IssueSummary` (HTTP + SSE) gained a non-optional `version` field
  carrying the canonical content hash. The web client uses it as
  `expected_version` for drag-and-drop PATCHes without per-card GETs.
- Server caches the schema and transitions config in serve mode.

### Fixed
- **Canonical hash now covers `title` and all unknown frontmatter
  keys**, so reorder-only and unknown-key edits no longer produce
  spurious "modified externally" conflicts and version tokens stay
  stable across CLI, server, and watcher.
- **`mutate::new_issue` publishes the upsert event before releasing
  the flock**, closing a window where a subscriber could observe the
  file before the event landed.
- `issuectl ls` no longer drops the first character of the H1 title
  in CLI display.
- Reopening a closed issue auto-appends a `## Reopen Notes — <today>`
  section in the same write.
- `doctor` re-checks `critical_blockers` after the flat-layout
  migration before any NN-rename, runs `rename_notes_to_comments`
  after lifting legacy dirs (so freshly-flat dirs get the one-shot
  rename), and preserves partial outcomes on mid-loop failure.
- `legacy_number_from_mapping` data-loss when `number:` and `slug:`
  coexisted in legacy frontmatter.

### Removed
- `issues/{open,closed}/` subdirectories (folded into the flat layout
  by `doctor --fix`). The legacy migration path is preserved — repos
  on 0.2.0/0.3.x just need to run `issuectl doctor --fix` once.

### Internals
- Refactor of the doctor apply pipeline into Findings/Actions/Outcome
  with golden-JSON tests; lifecycle classification derived from
  schema; multi-LLM review-driven hardening across roughly 30
  spin-off issues. See `@exorbitantly-ill-apples` for the full
  trail.

## [0.3.1] - 2026-05-06

The 0.3.0 release pipeline published binaries built from the
pre-restoration commit (missing the doctor parse-warnings surfacing
shipped on top). 0.3.1 republishes from the actual head with all
intended changes included; no behavior changes vs. the source tree
that 0.3.0 was supposed to ship.

### Fixed
- `issuectl doctor` now surfaces YAML parse warnings in both the text
  report (new "Parse warnings:" section) and JSON output (new
  `parse_errors` field), instead of silently printing to stderr while
  the summary claimed "Repository OK". Skipped for legacy `<NN>-slug`
  dirs since the migration pass rewrites their frontmatter.

## [0.3.0] - 2026-05-06

This release adds a local web board for browsing issues visually and a
bundled docs command. `issuectl doctor --fix` is preserved as the
upgrade path for repos still on the legacy `<NN>-<slug>` layout.

### Added
- `issuectl serve` — a local, read-only web board (Trello-style) for
  the current repo's `issues/`. Open + closed columns, issue detail
  view with rendered markdown, and side docs from any sibling `*.md`
  files in the issue directory. Defaults to `127.0.0.1:7878`;
  `--host`/`--port` flags available, with a warning when bound to a
  non-loopback address. JSON API under `/api/issues[/...]` for
  programmatic use. Defense-in-depth security headers (CSP,
  X-Content-Type-Options, Referrer-Policy, X-Frame-Options) on every
  response, slug validation before any filesystem access, and rejection
  of symlinks that escape an issue directory.
- `issuectl docs [topic]` — bundled long-form documentation. First
  topic `kanban` covers the web board (usage, scope, security, routes).
  New topics drop into `templates/docs/` and register in `src/docs.rs`.
- `/issue` skill: install instructions for teammates who land in a
  repo using issuectl but haven't installed it yet (Homebrew, Cargo,
  shell installer); pointer to `issuectl serve` + `issuectl docs
  kanban` for visual browsing.

### Fixed
- `issuectl serve`: hardened against XSS via raw HTML in markdown
  bodies (ammonia sanitization), tightened CSP, and rejected symlink
  escape attempts in the side-docs endpoint.

## [0.2.0] - 2026-05-05

This release replaces sequential issue numbering with random word slugs.
**Breaking change** to repo layout: existing repos must run `issuectl
doctor --fix` once to migrate.

### Added
- `issuectl doctor` — repository health-check that detects legacy
  `<NN>-<slug>/` directories, invalid/duplicate slugs, missing `item.md`,
  and orphan epic references. Read-only by default; `--fix` performs
  the one-shot migration: renames dirs to slug-only, drops `number:`
  from frontmatter and writes `slug:`, and rewrites `#NN` body
  references to `@slug` form (scoped to `issues/`). `--json` for
  machine-readable output.
- `src/slug` module with adjective-adjective-noun generator and shared
  validation. Wordlists vendored under `src/slug/wordlists/` with
  attribution in `NOTICE` (EFF Long Wordlist CC-BY 3.0, moby
  Apache 2.0, `names` crate MIT).
- Detection of legacy dirs even when `item.md` lacks a `number:`
  field — falls back to parsing the dirname pattern, and tolerates
  missing/malformed YAML frontmatter. Verified end-to-end against
  real-world repos (~161 and ~240 legacy issues) where some items
  had no frontmatter at all.

### Changed
- **Issue identifier is now a random `adjective-adjective-noun` slug**
  (e.g. `quiet-brave-otter`) instead of a sequential integer. With
  ~500 × ~500 × ~1000 wordlists the collision space (~250M) is large
  enough that distributed/worktree workflows can land in any order
  without renumbering. Frontmatter field `number:` is replaced by
  `slug:`. Directory layout is `issues/{open,closed}/<slug>/`
  (numeric prefix dropped). `issuectl new` returns `slug` (string)
  in `--json` output instead of `number` (integer). All commands
  (`show`, `update`, `close`) accept slugs.
- Body cross-references use `@slug` instead of `#NN`. Markdown
  headings (`# Title`) are no longer rewritten as references.
- Slug claim at issue creation is now atomic (`mkdir`-based) so
  concurrent `issuectl new` runs cannot overwrite each other.
- Slug validation is unified across the crate; the `doctor` migration
  refuses to write paths that would escape `issues/`.

### Removed
- `issuectl renumber` — collisions are no longer possible by
  construction, so the band-aid is gone. Use `issuectl doctor --fix`
  for the one-shot migration from numbered repos.

### Fixed
- `doctor --fix` now migrates legacy directories correctly even when
  `item.md` has no `number:` (or no frontmatter at all) — previously
  these were left in place while the loader still printed
  legacy-numeric warnings, producing a contradictory "Repository OK"
  summary.

## [0.1.0] - 2026-05-02

Initial public release.

### Added
- `issuectl new` — create issues and epics with strict validation, automatic
  numbering, and kebab-case slug generation (preserves Finnish characters).
- `issuectl update` — edit frontmatter fields (status, assignee, owner,
  priority, epic, labels, related, commits) with round-trip preservation
  of unknown keys and field order.
- `issuectl close` — set a closing status and atomically move the issue to
  `closed/`. Defaults to `fixed` for bugs, `done` otherwise.
- `--root <PATH>` global flag to operate on an external repo without
  changing cwd.
- Strict input validation via `clap` `PossibleValuesParser` for `--type`,
  `--priority`, `--status`. Empty/whitespace-only string arguments are
  rejected with clear errors.
- `--json` output for `list`, `show`, `search`, `stats`.
- `issuectl skill install --agent claude|codex|all` to bootstrap a target
  repo with the `/issue` skill template (Claude Code at
  `.claude/skills/issue/SKILL.md`, Codex at `.codex/prompts/issue.md`,
  or both).
- `issuectl skill print --agent claude|codex` to preview the template
  on stdout without writing to disk.
- 95 unit and integration tests covering pure helpers, frontmatter
  round-trip, command flows (tempdir-backed), and renumber edge cases.
- End-to-end manual verification against a real-world ~144-issue
  monorepo with duplicate-numbering edge cases (fixture removed
  before publication).

### Changed
- **`issuectl renumber` is now minimal by default.** Unique issue numbers
  are preserved; only duplicates are renumbered, with the first by sort
  order keeping its number and the rest spilling above the current max.
  This drops 130 dir-renames to 22 on the real-world ~144-issue monorepo (~144
  issues, 19 duplicate numbers). The previous compact-1..N renumbering
  is no longer available — file an issue if you need it back.
- `issuectl renumber` now scans the **whole repo** for `.md` references
  by default (skipping `.git`, `target`, `node_modules`, `.cargo`,
  `dist`, `build`) instead of only `issues/`, so monorepo cross-references
  in `CLAUDE.md`, per-crate `AGENTS.md`, etc. stay consistent. Use
  `--scope <PATH>` (repeatable) to limit.
- `issuectl renumber --dry-run` previews the plan and ambiguous-reference
  list without modifying anything.
- `issuectl renumber --pin NUMBER=SLUG_SUBSTRING` (repeatable) tells the
  resolver which dir in a duplicate group keeps the original number.
  Substring is matched against the slug within the group; errors if it
  matches zero or multiple dirs (with the candidates listed). This
  matters when the repo's docs reference a duplicate number meaning a
  specific dir, not the alphabetically-first one. Wishlist item #12.
- `issuectl --json renumber` (with or without `--dry-run`) emits a
  structured report with the plan, ambiguous-mapping table, and
  per-step counts. Useful for pipelines and `sed`-script generation —
  agent feedback specifically asked for this.
- `--json` now also covers `new`, `update`, and `close`. The skill
  templates have been rewritten so every `issuectl` example uses
  `--json`, since the consumer is an AI agent that should never have
  to parse human-formatted output.
- `AGENTS.md` at the repo root captures the rule that any CLI surface
  change must update `templates/issue-skill.md` and
  `templates/issue-prompt.md` in the same commit, and re-run
  `issuectl skill install --agent all --force` so the in-repo
  `.claude/` and `.codex/` copies don't drift.
- Renumber's post-run report now lists each old number's spillover
  mapping (`#14 now maps to: #14 (kept) + #123 + #124`) and provides a
  ripgrep one-liner to find body-text references that need manual
  review.
- Re-opening a closed issue (setting an active status from `closed/`)
  now clears the `closed:` field automatically.
- Skill template (`templates/issue-skill.md`) rewritten to delegate
  Search/List/Show/Create/Update/Close to `issuectl` instead of raw
  filesystem operations.
- `issues/AGENTS.md` now drops legacy `# NN.` heading prefixes
  consistently with `renumber`'s behavior.

### Fixed
- `split_text` lost the blank line between frontmatter and body on
  round-trip, and produced a stray newline as the body for issues with
  empty bodies. Both fixed.
- Renumber's `rewrite_issue_dir_paths` recompiled directory regexes
  per (file × line × dir-map entry), which on real-world data with
  ~150 markdown files and ~20 dir-map entries produced ~330k regex
  compilations and never finished. Hoisted to once per file (~15s on
  the real-world ~144-issue monorepo).

### Removed
- `issuectl dedup` stub — moved to a future release until properly
  implemented.

[Unreleased]: https://github.com/jarimustonen/issuectl/compare/v0.5.2...HEAD
[0.5.2]: https://github.com/jarimustonen/issuectl/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/jarimustonen/issuectl/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/jarimustonen/issuectl/compare/v0.3.1...v0.5.0
[0.3.1]: https://github.com/jarimustonen/issuectl/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/jarimustonen/issuectl/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jarimustonen/issuectl/releases/tag/v0.2.0
[0.1.0]: https://github.com/jarimustonen/issuectl/releases/tag/v0.1.0
