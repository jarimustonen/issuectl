# issuectl

[![CI](https://github.com/jarimustonen/issuectl/actions/workflows/ci.yml/badge.svg)](https://github.com/jarimustonen/issuectl/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust: 2021](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)

> AI-first CLI for managing markdown-based issues — no database, no server,
> just files in your repo.

`issuectl` tracks issues, tasks, features, and epics as plain markdown
files with YAML frontmatter, stored under
`issues/open/<slug>/item.md`. Each issue gets a random
`intensifier-adjective-noun` slug (e.g. `extremely-quiet-otter`) so that
work in parallel branches and worktrees never collides.
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
- **Collision-free by construction.** Random `intensifier-adjective-noun`
  slugs (~100M combinations) replace sequential numbering. Two branches
  creating issues independently can be merged in any order without
  renaming.

## Features

- `list` / `show` / `search` / `stats` — browse with filters and JSON output
- `new` / `update` / `close` — create, mutate, and resolve issues with strict validation
- `doctor` — health-check the repo (slug sanity, duplicates, orphans)
- `skill install` / `skill print` — install or preview the `/issue` skill
  template for Claude Code or Codex CLI (or both)
- `serve` — run a local Trello-style web board (read-only)
- `fmt` — normalize `item.md` files (canonical key order, sorted arrays,
  ATX headings) so reviews focus on real changes
- `merge-driver` — opt-in git custom merge driver that union-merges
  `labels` / `related` / `blocked_by` / `commits` and picks the newer
  `updated:` instead of conflicting on every cross-branch edit
- `--root <PATH>` — operate on an external repo from any working directory

## Install

Pick whichever channel suits your platform. After installing, verify with:

```sh
issuectl --version
```

### Homebrew — macOS and Linux

```sh
brew install jarimustonen/issuectl/issuectl
```

The first run automatically taps `jarimustonen/homebrew-issuectl`. To
upgrade later: `brew upgrade issuectl`.

### Cargo — any platform with a Rust toolchain

```sh
cargo install issuectl
```

### Shell installer — any platform, no toolchain required

Downloads the prebuilt binary for your OS/arch and drops it in
`~/.cargo/bin` (or equivalent):

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
    https://github.com/jarimustonen/issuectl/releases/latest/download/issuectl-installer.sh | sh
```

Or grab a tarball manually from the
[releases page](https://github.com/jarimustonen/issuectl/releases) —
binaries are signed-checksummed and available for macOS (Intel +
Apple Silicon) and Linux x86_64.

### From source

```sh
git clone https://github.com/jarimustonen/issuectl
cd issuectl
cargo install --path .
```

## Quick start

After installing, set up your repo and create your first issue:

```sh
cd path/to/your/repo

# Install the /issue skill so Claude Code or Codex CLI can drive issuectl.
# This also creates issues/AGENTS.md and issues/{open,closed}/ if they
# don't exist yet.
issuectl skill install --agent all

# Create your first issue (random slug auto-generated):
issuectl new --type bug --title "Login loops on Safari" \
    --reporter alice --assignee bob --priority high
# → Created extremely-quiet-otter: Login loops on Safari
#     /your/repo/issues/open/extremely-quiet-otter/item.md

# Browse:
issuectl list
issuectl show extremely-quiet-otter

# Move it through the workflow:
issuectl update extremely-quiet-otter --status in-progress
issuectl update extremely-quiet-otter --add-commit "abc1234:fix redirect state init"
issuectl close extremely-quiet-otter                       # status → fixed (default for bugs)
```

JSON output for any command (for scripting and AI agents):

```sh
issuectl --json list -t bug --status open
issuectl --json show extremely-quiet-otter
```

## Usage

### Browse

```
issuectl list                          List open issues (default)
issuectl ls -a alice                   Filter by assignee
issuectl ls -t bug -p high             Combine filters
issuectl ls --all                      Include closed issues
issuectl ls --closed --json            Closed issues, machine-readable
issuectl show <slug>                   Show single issue details
issuectl search redirect [--all]       Keyword search in title/slug/body
issuectl stats [--json]                Summary statistics
```

Filter flags: `-a/--assignee`, `-t/--type`, `-p/--priority`,
`-s/--status`, `-e/--epic` (slug), `-l/--label`, `--all`, `--closed`.

### Write

```
issuectl new --type bug --title "Login loops" \
    --reporter alice --assignee bob
# Random slug auto-generated; pass --slug <kebab> to override.

issuectl new --type epic --title "API v2 migration" \
    --owner cara --priority high

issuectl update <slug> --status in-progress
issuectl update <slug> --add-commit "abc123:fix login state" --add-label frontend
issuectl update <slug> --add-related "@other-slug" --epic api-v2-migration
issuectl update <slug> --no-epic --remove-label stale

issuectl close <slug>                   Defaults: `fixed` for bugs, `done` otherwise
issuectl close <slug> --status wontfix --commit "abc123:design decision"
```

Cross-references in body markdown use `@<slug>` (e.g. `@extremely-quiet-otter`).
The `epic:` and `related:` frontmatter fields store bare slugs / `@<slug>`.

Strict validation: invalid `--type`, `--priority`, or `--status` values
are rejected with the list of valid options. Closing statuses (`done`,
`fixed`, `wontfix`, `duplicate`, `cannot-reproduce`, `obsolete`)
automatically move the directory from `open/` to `closed/` and stamp
`closed:` with today's date. Setting a non-closing status on a closed
issue moves it back to `open/` and clears `closed:`.

### Maintenance

```
issuectl doctor                        Read-only health-check report
issuectl --json doctor                 Machine-readable report
issuectl skill install                 Install /issue skill (default: Claude Code)
issuectl skill install --agent codex   Install Codex prompt instead
issuectl skill install --agent all     Install both
issuectl skill print [--agent codex]   Preview the template without installing
```

`doctor` performs the following checks:

- **Slug sanity.** Flags slugs that don't pass `is_valid()` (lowercase
  ASCII kebab-case, at least two segments, letters/digits only).
- **Duplicates.** Flags any slug used twice across `open/` + `closed/`.
- **Missing item.md.** Flags directories without an `item.md`.
- **Orphan epic refs.** Flags `epic:` values that don't resolve to an
  existing slug.

### Web view

```
issuectl serve                         Start the local board on http://127.0.0.1:7878
issuectl serve --port 9000             Pick a different port
issuectl serve --host 0.0.0.0          Bind to all interfaces (LAN access)
```

`serve` runs a small read-only web server that renders `issues/` as a
Trello-style Kanban board (Open / In progress / Testing / Closed columns,
plus an "Other" catchall for unrecognised statuses). Filter by type,
assignee, epic, or label and search across slug and title; filter state
persists in the URL, so reloads and bookmarks work. Click any card to
open a modal with the rendered markdown body, frontmatter, and any
additional `*.md` files alongside `item.md`. The server re-reads the
filesystem on every request, so editing an `item.md` and refreshing the
browser shows the change without restarting.

Bind defaults to `127.0.0.1` (local-only). `--host 0.0.0.0` exposes the
board to your network: there is **no authentication and no TLS**, so use
it only on trusted networks — `serve` prints a stderr warning when bound
to a non-loopback interface as a reminder. Edits via the browser will
land in a follow-up release.

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
epic: api-v2-migration
related: ["@notably-brave-otter", "@simply-fierce-comet"]
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

### `issuectl fmt` — normalize on-disk files

```sh
issuectl fmt                    # rewrite every issues/<slug>/item.md
issuectl fmt some-slug          # specific slug(s)
issuectl fmt --check            # exit non-zero if anything would change (CI)
issuectl fmt --diff             # print a unified diff, no writes
issuectl --json fmt --check     # per-file JSON results
```

`fmt` is idempotent — `issuectl fmt && issuectl fmt --check` always
exits 0. Normalises:

- **frontmatter key order**: `created`, `updated`, `closed`, `type`,
  `status`, `priority`, `reporter`, `assignee`, `owner`, `epic`,
  `blocked_by`, `related`, `labels`, `commits`, then any unknown keys
  alphabetically;
- **arrays** (`labels` / `related` / `blocked_by`) sorted;
  `commits` keeps its order (it's chronological);
- markdown setext headings (`====`) rewritten to ATX (`#`);
- one blank line between `---` close and the body, no trailing
  whitespace, single final newline.

### Optional git merge driver

`issuectl merge-driver` is a custom three-way merge driver for
`issues/**/*.md`. It union-merges `labels` / `related` / `blocked_by`,
keeps `commits` as a hash-keyed log, and picks the newer `updated:` —
mitigating the most common cross-branch conflict mode for file-based
issue trackers. Scalar fields that diverge on both sides still produce
a conflict (the driver never silently picks a side).

To enable it for a repo:

```sh
# Add to .gitattributes (commit this):
echo 'issues/**/item.md merge=issuectl-yaml' >> .gitattributes

# Configure the driver locally (per-clone, not committed):
git config merge.issuectl-yaml.driver \
    "issuectl merge-driver --base %O --ours %A --theirs %B --output %A"
```

Or print and apply via:

```sh
issuectl install-merge-driver           # print the snippets
issuectl install-merge-driver --apply   # also run `git config` for you
```

`install-merge-driver` never modifies `.gitattributes` — that file is
shared, so its contents are your decision.

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
