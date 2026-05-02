# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- `issuectl skill install` to bootstrap a target repo with the `/issue`
  Claude Code skill and `issues/AGENTS.md`.
- 95 unit and integration tests covering pure helpers, frontmatter
  round-trip, command flows (tempdir-backed), and renumber edge cases.
- End-to-end test fixture at `tests/fixtures/grooveserve/` (~144 issues,
  4 epics, duplicate-numbering edge cases, Finnish content).

### Changed
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
  the grooveserve fixture).

### Removed
- `issuectl dedup` stub — moved to roadmap until properly implemented.

## [0.1.0] - Initial scaffold

- Minimal `cargo new` scaffold with `clap`, `serde_yaml`, `regex`.
- Stub commands; no functional implementation.

[Unreleased]: https://github.com/jarimustonen/issuectl/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jarimustonen/issuectl/releases/tag/v0.1.0
