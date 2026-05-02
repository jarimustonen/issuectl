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
- **Worktree-friendly context handoff.** In worktree-based agent flows,
  an issue body doubles as a durable, self-contained prompt: one agent
  investigates and writes up `## Reproduction` / `## Analysis` /
  `## Scope`, then a follow-up agent in a fresh worktree reads the
  issue and implements directly from it. Frontmatter carries the
  routing (assignee, status, epic, related); the body *is* the work
  order. No external task tracker to sync, no chat history to
  reconstruct — the file in `issues/` is the context.
- **Round-trip safe.** Frontmatter mutations preserve field order and
  unknown keys. Body text is left verbatim.
- **Renumber on merge.** When two branches both create issue #14,
  `issuectl renumber` resolves the conflict (preserving unique numbers,
  spilling duplicates) and rewrites `#NN` / `epic:` cross-references
  across the whole repo.

## Features

- `list` / `show` / `search` / `stats` — browse with filters and JSON output
- `new` / `update` / `close` — create, mutate, and resolve issues with strict validation
- `renumber` — uniquify numbers and fix cross-references after merges
- `skill install` / `skill print` — install or preview the `/issue` skill
  template for Claude Code or Codex CLI (or both)
- `--root <PATH>` — operate on an external repo from any working directory

## Install

### Homebrew (macOS / Linux)

```sh
brew tap jarimustonen/issuectl
brew install issuectl
```

### Pre-built binary (any platform)

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
    https://github.com/jarimustonen/issuectl/releases/latest/download/issuectl-installer.sh | sh
```

Or download a tarball directly from the [releases page](https://github.com/jarimustonen/issuectl/releases).

### crates.io

```sh
cargo install issuectl
```

### From source

```sh
git clone https://github.com/jarimustonen/issuectl
cd issuectl
cargo install --path .
```

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
issuectl renumber                      Resolve duplicate numbers (preserve unique)
issuectl renumber --dry-run            Preview the plan without modifying anything
issuectl renumber --scope crates       Limit reference rewriting to one subtree
issuectl renumber --pin 26=multi-tenant  Pin a specific dir to keep its number
issuectl --json renumber [--dry-run]   Structured plan + report (for pipelines)
issuectl skill install                 Install /issue skill (default: Claude Code)
issuectl skill install --agent codex   Install Codex prompt instead
issuectl skill install --agent all     Install both
issuectl skill print [--agent codex]   Preview the template without installing
```

`renumber` is **minimal by default**: unique issue numbers keep their
numbers, and only duplicates (multiple directories sharing one number,
typical after merging two branches that each created issue #14) are
renumbered. The first by sort order keeps the number and the rest spill
above the current max — e.g. three #14's become #14 (kept), #192, #193.

References to *unique* numbers stay valid automatically (since the
numbers don't move). References to *duplicate* numbers (`#14`,
`epic: 14`, `related: ["#14"]`) are reported as ambiguous and left
unchanged for manual review — `issuectl` cannot guess which of the
three the writer meant.

If the repo's docs reference a duplicate number meaning a *specific*
one of the dirs (say, `#26` always meant the multi-tenant epic, not
alphabetically-first `infra-email`), pin it with
`--pin NUMBER=SLUG_SUBSTRING`. The pinned dir keeps the number and the
others spill. Repeatable for multiple numbers. Errors if the substring
matches zero or multiple dirs in the group.

By default, reference rewriting scans the whole repo for `.md` files
(skipping `.git/`, `target/`, `node_modules/`, `.cargo/`, `dist/`,
`build/`) so that monorepo references in `CLAUDE.md`, `AGENTS.md`,
per-crate docs, etc. stay consistent. Use `--scope <PATH>` (repeatable)
to narrow the search.

Always run `--dry-run` first on a real repo to see the plan.

### Pointing to an external repo

```sh
issuectl --root ~/code/some-other-project list
issuectl --root /path/to/another/repo stats
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

## Agent integration

`issuectl skill install` writes a `/issue` skill template into a target
repo so an AI agent can drive issue management through `issuectl`
rather than poking at the filesystem directly. Two formats are
supported:

| Agent             | Destination                       | Format                                 |
| ----------------- | --------------------------------- | -------------------------------------- |
| Claude Code       | `.claude/skills/issue/SKILL.md`   | YAML frontmatter + markdown body       |
| Codex CLI         | `.codex/prompts/issue.md`         | Plain markdown prompt                  |

```sh
issuectl skill install                  # Claude Code skill (default)
issuectl skill install --agent codex    # Codex prompt
issuectl skill install --agent all      # both
issuectl skill print                    # preview Claude template to stdout
issuectl skill print --agent codex      # preview Codex template
```

The skill instructs the agent to delegate Search/List/Show/Create/Update/Close
to `issuectl`, but leaves body markdown editing (`## Reproduction`,
epic `## Issues`/`## Phases` sections, screenshot attachments) to the
agent since those are out of scope for the CLI.

Source templates live at
[`templates/issue-skill.md`](templates/issue-skill.md) (Claude) and
[`templates/issue-prompt.md`](templates/issue-prompt.md) (Codex) if you
want to customize before installing.

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
```

See [AGENTS-AI-FIRST-CLI.md](AGENTS-AI-FIRST-CLI.md) for the design
principles every command follows.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the
PR process, dev setup, and coding conventions.

To report a security vulnerability, please follow the process in
[SECURITY.md](SECURITY.md).

## License

MIT — see [LICENSE](LICENSE).
