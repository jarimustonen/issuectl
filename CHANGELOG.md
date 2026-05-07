# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Shared query engine (`src/query.rs`) used by `issuectl ls`, `issuectl
  search`, and the web `/api/issues?q=` endpoint. Syntax: `field:value`,
  `-field:value` (negation), `text:"phrase"`, bareword (treated as
  `text:`), `field:any`/`field:none`, and relative dates
  (`updated:<-14d`, `created:>=-30d`, anchor: today UTC, inclusive).
  Multiple terms AND together; no OR/parens in v1. The existing
  flag-based `ls` invocations remain backwards-compatible — flags
  translate to query terms internally.
- Web kanban: cards are draggable between status columns. Dropping on an
  active column (Open / In progress / Testing) issues a status PATCH
  immediately; dropping on Closed opens a small picker so the user
  selects a closing status (`done`/`fixed`/`wontfix`/...). Optimistic
  UI with revert-on-failure, version-aware concurrency, and toast
  notifications.

### Changed
- API: `IssueSummary` (returned from `GET /api/issues` and embedded in
  SSE `IssueUpserted` events) gained a non-optional `version` field
  carrying the issue's canonical content hash. The web client uses it as
  `expected_version` for drag-and-drop PATCHes without per-card GETs.
  External consumers deserializing the response with `deny_unknown_fields`
  will need to add the field; permissive deserializers are unaffected.

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

[Unreleased]: https://github.com/jarimustonen/issuectl/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/jarimustonen/issuectl/releases/tag/v0.2.0
[0.1.0]: https://github.com/jarimustonen/issuectl/releases/tag/v0.1.0
