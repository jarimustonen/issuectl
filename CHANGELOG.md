# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/jarimustonen/issuectl/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/jarimustonen/issuectl/compare/v0.3.1...v0.5.0
[0.3.1]: https://github.com/jarimustonen/issuectl/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/jarimustonen/issuectl/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jarimustonen/issuectl/releases/tag/v0.2.0
[0.1.0]: https://github.com/jarimustonen/issuectl/releases/tag/v0.1.0
