# issuectl

[![CI](https://github.com/jarimustonen/issuectl/actions/workflows/ci.yml/badge.svg)](https://github.com/jarimustonen/issuectl/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust: 2021](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)

> AI-first CLI for managing markdown-based issues — no database, no server,
> just files in your repo.

`issuectl` tracks issues, tasks, features, and epics as plain markdown
files with YAML frontmatter, stored under `issues/open/NN-slug/item.md`.
The CLI is designed to be driven by AI agents (e.g. via the `/issue`
[Claude Code](https://claude.com/claude-code) skill) — strict input
validation, structured JSON output, no interactive prompts — but humans
can use it from a terminal too.

## Why issuectl?

- **Zero infrastructure.** Issues live in your repo. Diff them, branch
  them, blame them, review them in PRs.
- **AI-friendly.** Every command speaks `--json`, validates inputs strictly,
  and returns meaningful exit codes. Designed to be a tool for agents
  rather than a UI for humans.
- **Markdown-first.** Issues are just files. Edit them in your editor,
  attach screenshots and analysis docs, search them with `grep`.
- **Round-trip safe.** Frontmatter mutations preserve field order and
  unknown keys. Body text is left verbatim.
- **Renumber on merge.** When two branches both create issue #14,
  `issuectl renumber` reassigns sequential numbers and rewrites
  `#NN` and `epic:` cross-references across all markdown files.

## Features

- `list` / `show` / `search` / `stats` — browse with filters and JSON output
- `new` / `update` / `close` — create, mutate, and resolve issues with strict validation
- `renumber` — uniquify numbers and fix cross-references after merges
- `skill install` — install the `/issue` Claude Code skill into a target repo
- `--root <PATH>` — operate on an external repo from any working directory

## Install

### From source

```sh
git clone https://github.com/jarimustonen/issuectl
cd issuectl
cargo install --path .
```

### From crates.io

Not yet published.

## Quick start

```sh
# In a fresh repo:
issuectl skill install

# Create your first issue:
issuectl new --type bug --title "Login loops on Safari" \
    --reporter alice --assignee bob --priority high

# Browse:
issuectl list
issuectl show 1

# Update:
issuectl update 1 --status in-progress \
    --add-commit "abc123:fix redirect state init"

# Close:
issuectl close 1
```

## Usage

### Browse

```
issuectl list                          List open issues (default)
issuectl ls -a alice                   Filter by assignee
issuectl ls -t bug -p high             Combine filters
issuectl ls --all                      Include closed issues
issuectl ls --closed --json            Closed issues, machine-readable
issuectl show 42                       Show single issue details
issuectl search redirect [--all]       Keyword search in title/slug/body
issuectl stats [--json]                Summary statistics
```

Filter flags: `-a/--assignee`, `-t/--type`, `-p/--priority`,
`-s/--status`, `-e/--epic`, `-l/--label`, `--all`, `--closed`.

### Write

```
issuectl new --type bug --title "Login loops" \
    --reporter alice --assignee bob

issuectl new --type epic --title "API v2 migration" \
    --owner cara --priority high

issuectl update 42 --status in-progress
issuectl update 42 --add-commit "abc123:fix login state" --add-label frontend
issuectl update 42 --add-related "#41" --epic 5
issuectl update 42 --no-epic --remove-label stale

issuectl close 42                       Defaults: `fixed` for bugs, `done` otherwise
issuectl close 42 --status wontfix --commit "abc123:design decision"
```

Strict validation: invalid `--type`, `--priority`, or `--status` values
are rejected with the list of valid options. Closing statuses (`done`,
`fixed`, `wontfix`, `duplicate`, `cannot-reproduce`, `obsolete`)
automatically move the directory from `open/` to `closed/` and stamp
`closed:` with today's date. Setting a non-closing status on a closed
issue moves it back to `open/` and clears `closed:`.

### Maintenance

```
issuectl renumber                      Uniquify numbers, fix #NN refs
issuectl skill install [--force]       Install /issue skill in current repo
```

`renumber` walks `issues/open/` and `issues/closed/`, assigns sequential
numbers across both folders, renames the directories, and rewrites
`#NN`, `epic: NN`, and `related: ["#NN"]` references in every markdown
file in `issues/`. References to numbers that had multiple old
directories are ambiguous and are reported instead of guessed.

### Pointing to an external repo

```sh
issuectl --root ~/code/some-other-project list
issuectl --root tests/fixtures/grooveserve stats
```

## File format

Issues are markdown files with YAML frontmatter:

```markdown
---
created: 2026-05-02
updated: 2026-05-02
type: bug
reporter: alice
assignee: bob
status: open
priority: normal
epic: 5
related: ["#3", "#7"]
labels: [frontend, auth]
commits:
  - hash: abc1234
    summary: "fix(auth): redirect after SSO"
---

# Issue title

_Source: which service / page / feature_

## Description

...
```

See [issues/AGENTS.md](issues/AGENTS.md) for the full schema reference,
status workflow, and conventions.

## Claude Code integration

`issuectl skill install` writes a `/issue` skill to
`.claude/skills/issue/SKILL.md` in the target repo. The skill instructs
Claude Code to delegate Search/List/Show/Create/Update/Close to
`issuectl` rather than poking at the filesystem directly. It still
handles body markdown (`## Reproduction`, epic `## Issues`/`## Phases`
sections, screenshot attachments) since those are not in the CLI's
scope.

The skill template is also available at
[`templates/issue-skill.md`](templates/issue-skill.md) if you want to
customize it before installing.

## Configuration

| Flag       | Scope     | Description                                           |
| ---------- | --------- | ----------------------------------------------------- |
| `--root`   | global    | Override repo root (the dir containing `issues/`)     |
| `--json`   | global    | Emit JSON to stdout instead of human-readable tables  |

Without `--root`, `issuectl` walks up from cwd looking for `issues/` or
`.git`.

## Development

Requires a Rust toolchain (2021 edition or newer).

```sh
cargo build
cargo test
cargo clippy --all-targets
cargo run -- --root tests/fixtures/grooveserve list
```

The `tests/fixtures/grooveserve/` directory contains a real-world
fixture (~144 issues, 4 epics, mixed types/statuses, Finnish slugs,
duplicate-numbering edge cases) used for end-to-end manual verification.

See [AGENTS-AI-FIRST-CLI.md](AGENTS-AI-FIRST-CLI.md) for the design
principles every command follows.

## Roadmap

- [ ] `issuectl dedup` — detect duplicate issues by title/body similarity
- [ ] Publish to crates.io
- [ ] Pre-built binaries for macOS / Linux / Windows
- [ ] Optional shell completions (`bash`, `zsh`, `fish`)

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the
PR process, dev setup, and coding conventions.

To report a security vulnerability, please follow the process in
[SECURITY.md](SECURITY.md).

## License

MIT — see [LICENSE](LICENSE).
