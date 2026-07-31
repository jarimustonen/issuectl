# issuectl

[![CI](https://github.com/jarimustonen/issuectl/actions/workflows/ci.yml/badge.svg)](https://github.com/jarimustonen/issuectl/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/issuectl.svg)](https://crates.io/crates/issuectl)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust: 2021](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)

> AI-first CLI for managing markdown-based issues — no database, no
> server, just files in your repo.

`issuectl` tracks issues, tasks, features, and epics as plain markdown
files with YAML frontmatter, stored under `issues/<slug>/item.md`.
Slugs are short kebab-case identifiers — prefer a descriptive 2-3 word
slug derived from the title (`login-redirect-loops`).

It is built for AI coding workflows. An issue's body is durable,
self-contained context: one agent investigates and writes up
`## Reproduction` / `## Analysis` / `## Acceptance Criteria` into
`issues/<slug>/item.md`; a follow-up agent in a fresh git worktree
reads the same issue and implements directly from it. Frontmatter
carries the routing (assignee, status, epic, related, blocked_by);
the body *is* the work order. `issuectl context <slug>` packages all
of that into a deterministic prompt bundle the agent can feed to
itself, `issuectl ready <slug>` reports Definition-of-Done completion
as a parseable result, and the `/issue` skill teaches Claude Code and
Codex CLI to drive the rest. Every command speaks `--json`, validates
strictly, and never prompts interactively — humans can use it from a
terminal too, but the design centre is the agent.

## Why issuectl?

- **Zero infrastructure.** Issues live in your repo. Diff them, branch
  them, blame them, review them in PRs.
- **AI-friendly.** Every command speaks `--json`, validates inputs
  strictly, and returns meaningful exit codes. `issuectl context
  <slug>` renders a deterministic prompt bundle that agents can feed
  to themselves; `issuectl ready <slug>` reports Definition-of-Done
  completion as a parseable result.
- **Lightweight planning, no SaaS.** Cycles, estimates, dependencies,
  reviewer state, recurring issues, DoD checklists — all in
  frontmatter or markdown, all offline.
- **Markdown-first.** Issues are just files. Edit them in your editor,
  attach screenshots and analysis docs alongside them, search them
  with `grep`.
- **Round-trip safe.** Frontmatter mutations preserve field order and
  unknown keys. Body text is left verbatim outside the sections you
  ask to touch.
- **Git is the event log.** No event database — `issuectl activity` /
  `timeline` / `changelog` / `metrics` derive everything from
  `git log` and `Refs-Issue:` / `Fixes-Issue:` commit trailers.
- **Collision-free by construction.** Two branches creating issues
  independently can be merged in any order: the random-slug fallback
  has ~100M combinations, and the optional YAML merge driver
  union-merges `labels` / `related` / `blocked_by` / `commits` and
  picks the newer `updated:` instead of conflicting.

## Features at a glance

**Core lifecycle.** `new`, `update`, `note`, `close`, `rename`,
`show`, `list`, `search`, `stats`, `fmt`.

**Lightweight planning.**
- `depend add/remove` — canonical `blocked_by:` arrays; reverse
  `blocks:` derived at runtime; doctor flags cycles and self-deps.
- `cycle current/plan/status` — Linear-style iterations via an
  optional `cycle: 2026-W22` frontmatter label.
- `size:` / `estimate:` frontmatter + `workload` (open + in-progress
  per assignee / cycle / epic) and `burndown --cycle <name>` (ASCII).
- `reviewer:` + `review_status:` for teams that review through PRs
  but want issue-level review visibility.
- `schedule list/run` — recurring issues defined in
  `.issuectl/recurrences/<name>.yaml` (cron expression), materialised
  as one file per occurrence.
- `ready <slug>` — Markdown DoD validation. Parses `## Acceptance
  Criteria` / `## Tests Run` / `## Implementation Notes` task lists;
  `→ done` transition warns (or blocks, with `dod.strict: true`) on
  unchecked acceptance criteria.

**Git-derived reporting.**
- `activity [--since 7d]` — recent commits that touched `issues/`,
  grouped back to slugs.
- `timeline <slug>` — status transitions reconstructed from
  `git log -p` on the issue's `item.md`.
- `changelog <ref>..<ref>` — markdown release notes built from
  `Refs-Issue:` / `Fixes-Issue:` trailers.
- `metrics [--since 30d]` — throughput, median/p90/mean cycle time,
  open/closed workload by assignee.

**CLI ergonomics.**
- `open <slug>` — launch `item.md` in `$EDITOR`; `--dir` for the
  directory.
- `attach <slug> <file>...` — copy files into `issues/<slug>/attachments/`.
- `bulk '<query>' --set/--add-label/...` — apply one mutation across
  every query-matched issue under a single repo-wide lock;
  `--dry-run` shows the per-issue diff.
- `pick [QUERY]` — interactive fuzzy picker; prints the chosen slug.
- `triage` / `new --inbox` — `issues/inbox/<slug>/` landing zone for
  drafts; `triage <slug>` promotes one to the canonical layout.
- `scan-todos` — finds `// TODO(issue: <slug>)` markers in source;
  reports stale, untracked, and unknown hits;
  `--create-inbox` files untracked ones.
- `completions {bash,zsh,fish,powershell,elvish}` — shell completion
  scripts with dynamic value completion for slugs / statuses /
  labels / users.
- Slug prefix matching — `issuectl show login-redirect` resolves to
  the unique match; ambiguous prefixes list candidates.
- `note <slug> --stdin` / `--from-file PATH` — pipe a note into the
  `## Comments` section.

**Content & interop.**
- First-class `issues/<slug>/attachments/` and `fixtures/`
  directories. Doctor warns on path-traversal patterns and oversized
  binaries.
- `duplicates [<slug>]` — heuristic local-only duplicate detection
  (title-token overlap, shared labels, body tokens).
- `import json|github` / `export json|csv|markdown` — portable
  snapshots; GitHub import uses `gh`.

**Maintenance.**
- `stale [--days N]` — issues with no recent activity.
- `archive [--older-than N]` — moves closed issues to
  `issues/archive/YYYY/MM/`. All read commands consult both the
  active and archive roots.
- `doctor` / `doctor --fix` — health-check the repo, coerce legacy
  enum values via schema aliases, regenerate the AGENTS.md
  schema-derived block, fix layout drift, migrate legacy numbered
  layouts.

**Schema & validation.**
- `issues/.schema.yaml` declares required fields, enum constraints,
  `required_when` conditional rules, and `status_aliases` /
  `type_aliases` for migration.
- `doctor` enforces all of these. `doctor --fix` applies the alias
  coercions and regenerates the `.issuectl/AGENTS.md` agent-policy
  block.
- `issuectl context <slug>` injects schema-declared constraints into
  the agent context bundle as system instructions, so AI agents
  can't invent values outside the schema.

**Agent integration.**
- `skill install --agent claude|codex|all` — install the `/issue`
  skill template into the current repo (Claude Code or Codex CLI).
- `context <slug>` — render a deterministic prompt bundle (issue +
  parent epic + related/blocking refs + commits + schema rules).
- `prompt <template> <slug>` — render repo-local prompt templates
  (`.issuectl/prompts/<template>.md`) with `{{key}}` substitution.
- `sync-commits` — walk git history and attach commits to issues via
  `Refs-Issue:` / `Fixes-Issue:` trailers.

**Web view.** `serve` — Trello-style kanban board at
`http://127.0.0.1:7878` with drag-and-drop status changes and an
in-browser body editor (read-only when bound to a non-loopback
address).

**Cross-repo & customisation.**
- `--root <PATH>` — operate on an external repo from any working
  directory.
- `--json` — unified JSON envelope across every mutating command.
- `merge-driver` — opt-in git custom merge driver for
  `issues/**/item.md` that union-merges list fields.
- `fmt [--check] [--diff]` — normalise on-disk files for clean
  diffs.

## Install

Pick whichever channel suits your platform. After installing, verify
with:

```sh
issuectl --version
```

### Homebrew

```sh
brew install jarimustonen/issuectl/issuectl
```

The first run automatically taps `jarimustonen/homebrew-issuectl`. To
upgrade later: `brew upgrade issuectl`.

### Cargo

```sh
cargo install issuectl
```

### Shell installer

Downloads the prebuilt binary for your OS/arch and drops it into
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
cargo install --path crates/issuectl
```

## Quick start

After installing, bootstrap a repo and walk through one issue's
lifecycle:

```sh
cd path/to/your/repo

# One-shot bootstrap: writes issues/.schema.yaml, .issuectl/AGENTS.md,
# and the /issue skill for Claude Code + Codex.
issuectl init

# Create your first issue with a descriptive 2-3 word slug from the title.
issuectl new --type bug \
    --slug login-redirect-loops \
    --title "Login loops on Safari after SSO" \
    --reporter alice --assignee bob --priority high
# → Created login-redirect-loops: Login loops on Safari after SSO
#     /your/repo/issues/login-redirect-loops/item.md

# Browse:
issuectl list
issuectl show login-redirect-loops

# Move it through the workflow:
issuectl update login-redirect-loops --status in-progress
issuectl note login-redirect-loops --as alice "Repros on Safari 17.0; works on 16.x"
issuectl update login-redirect-loops --add-commit "abc1234:fix(auth): redirect after SSO"
issuectl close login-redirect-loops                # status → fixed (default for bugs)
```

Every command speaks `--json`:

```sh
issuectl --json list -t bug --status open
issuectl --json show login-redirect-loops
issuectl --json update login-redirect-loops --status testing \
    --expected-version $(issuectl --json show login-redirect-loops | jq -r .version)
```

### Agent-driven example

The skill that `issuectl init` installs teaches an agent to turn a
natural-language request into the right command. A typical exchange:

> **User:** "There's a bug where the login loops on Safari after SSO. I want to track it."

The agent reads `/issue`, picks a descriptive slug, and runs:

```sh
issuectl --json new --type bug \
    --slug login-redirect-loops \
    --title "Login loops on Safari after SSO" \
    --reporter alice --assignee bob --priority high
```

Later, asked to start implementation in a worktree:

> **User:** "Pick up login-redirect-loops and implement."

The agent generates a context bundle, hands it off to itself in the
worktree, and ticks off Acceptance Criteria as it goes:

```sh
issuectl --json context login-redirect-loops > /tmp/issue-context.json
# …work happens…
issuectl --json check login-redirect-loops "Redirect chain unwinds on Safari"
issuectl --json ready login-redirect-loops      # exits 0 when AC is complete
issuectl --json close login-redirect-loops \
    --expected-version $(issuectl --json show login-redirect-loops | jq -r .version)
```

## Usage

### Browse, search, and inspect

```sh
issuectl list                            # open issues (default scope)
issuectl ls -a alice                     # filter by assignee
issuectl ls -t bug -p high               # combine filters
issuectl ls "label:auth -label:wontfix updated:<-14d"  # query language
issuectl ls --all                        # include closed
issuectl ls --closed --json              # closed only, machine-readable

issuectl show <slug>                     # full details
issuectl search redirect [--all]         # keyword search across title/slug/body
issuectl stats [--json]                  # repo-wide rollup

issuectl duplicates                      # likely-duplicate pairs across all open issues
issuectl duplicates <slug>               # candidates similar to one issue

issuectl pick "auth"                     # interactive fuzzy picker; prints chosen slug
```

Filter flags: `-a/--assignee`, `-t/--type`, `-p/--priority`,
`-s/--status`, `-e/--epic`, `-l/--label`, `--all`, `--closed`. The
query language additionally supports `reviewer:`, `review_status:`,
`cycle:`, `blocked_by:`, `blocks:`, `size:`, `estimate:`, negation
(`-label:wontfix`), and relative date filters (`updated:<-14d`,
`created:>=-7d`).

`issuectl --json ls/search/show` is the contract surface for agents —
output is stable and documented.

### Create, mutate, and resolve

```sh
issuectl new --type bug \
    --slug login-redirect-loops \
    --title "Login loops on Safari" \
    --reporter alice --assignee bob

issuectl new --type epic --title "API v2 migration" \
    --slug api-v2-migration \
    --owner cara --priority high

issuectl update <slug> --status in-progress
issuectl update <slug> --add-commit "abc1234:fix login state"
issuectl update <slug> --add-label frontend --add-related "@another-slug"
issuectl update <slug> --epic api-v2-migration
issuectl update <slug> --no-epic --remove-label stale

# Single-field focused verbs (also flock-and-version-safe):
issuectl set    <slug> assignee bob
issuectl label  <slug> add frontend
issuectl check  <slug> "Redirect chain unwinds on Safari"

# Notes / decisions / agent runs are appended to the body, not frontmatter:
issuectl note     <slug> --as alice "Repros on Safari 17.0"
issuectl note     <slug> --decision --as alice "We'll ship the fix as a hotfix"
echo "log…" | issuectl note <slug> --as ci-bot --stdin

# Multi-field transactional patch:
issuectl apply patch.yaml                # slug + fields + body_ops in one flock

# Close:
issuectl close <slug>                    # → `fixed` for bugs, `done` otherwise
issuectl close <slug> --status wontfix --commit "abc1234:design decision"
```

Cross-references in body markdown use `@<slug>`. `epic:`, `related:`,
and `blocked_by:` frontmatter fields store bare slugs / `@<slug>`.

Strict validation: invalid `--type`, `--priority`, `--status`, or
`--size` values are rejected with the list of valid options. Closing
statuses (`done`, `fixed`, `wontfix`, `duplicate`,
`cannot-reproduce`, `obsolete`) stamp `closed:` with today's date.

### Dependencies

```sh
issuectl depend add    <slug> --blocked-by another-slug
issuectl depend remove <slug> --blocked-by another-slug
issuectl ls "blocked_by:any"             # everything that's blocked
issuectl ls "blocks:<slug>"              # what does this slug block?
issuectl ls "blocked_by:none"            # ready to start
```

`blocked_by:` is canonical (a `[slug]` array in frontmatter); the
reverse `blocks:` relationship is derived at runtime to avoid drift.
`doctor` reports missing referenced slugs, self-dependencies, and
cycles.

### Cycles, estimates, workload

```sh
issuectl cycle current                   # today's ISO-week label (e.g. 2026-W22)
issuectl cycle plan 2026-W22             # what's slotted for this cycle
issuectl cycle status [--all] [--json]   # open/closed rollup by cycle
issuectl set <slug> cycle 2026-W22       # assign

issuectl set <slug> size M               # S | M | L | XL (point-equivalents)
# or for free-form numeric estimates:
issuectl set <slug> estimate 3

issuectl workload [--json]               # open + in-progress points by assignee, cycle, epic
issuectl burndown --cycle 2026-W22       # ASCII burndown across the cycle's days
```

### Reviewer field

```sh
issuectl set <slug> reviewer alice
issuectl set <slug> review_status requested  # requested|in-review|approved|changes-requested
issuectl ls "reviewer:me"                # resolves via $ISSUECTL_USER → git config user.name
```

### Definition-of-Done

Standardise body sections so DoD is machine-checkable:

```markdown
## Acceptance Criteria
- [x] Redirect chain unwinds on Safari 17
- [ ] Error case shows friendly message
- [ ] Manual test on Safari 16.x

## Tests Run
- [ ] cargo test passes
```

```sh
issuectl ready <slug>                    # exit 0 only if AC is fully checked
issuectl --json ready <slug>             # parseable totals + per-section breakdown
```

Set `dod.strict: true` in `issues/.schema.yaml` to upgrade the
`→ done` warning to a hard block.

### Recurring / scheduled issues

```sh
issuectl schedule list                   # loaded recurrence definitions + materialisation state
issuectl schedule run                    # materialise occurrences whose cron has fired
```

Definitions live at `.issuectl/recurrences/<name>.yaml`:

```yaml
title: Weekly dependency review
schedule: "0 9 * * MON"                  # cron (UTC)
type: chore
labels: [maintenance, weekly]
assignee: alice
description: |
  Review npm and cargo dependency updates; bump security patches.
```

Each fire produces a fresh file with `recurrence_of:` and
`occurrence:` frontmatter — never overwrites a previous one, so git
history of each occurrence is preserved.

### Git-derived reporting

```sh
issuectl activity --since 7d             # commits touching issues/, grouped to slugs
issuectl timeline <slug>                 # status transitions from git log -p
issuectl changelog v0.5.2..v0.6.0        # release-note markdown from commit trailers
issuectl metrics --since 30d             # throughput, cycle time, workload
```

All four honour `--json`. Frontmatter timestamps win when rebases
have reshaped history.

### Bulk mutations

```sh
issuectl bulk "status:open label:auth" --add-label v0.6.0 --dry-run
issuectl bulk "status:open label:auth" --add-label v0.6.0
issuectl bulk "epic:api-v2-migration"  --set assignee=bob
```

The whole batch runs under a single repo-wide lock and validates
every target before any write lands. `--dry-run` shows affected
slugs plus a per-issue unified diff.

### Maintenance & cleanup

```sh
issuectl doctor                          # read-only health report
issuectl doctor --fix                    # apply migrations + alias coercions + AGENTS.md regen
issuectl stale --days 90                 # issues with no recent activity
issuectl archive --older-than 180        # move old closed issues to issues/archive/YYYY/MM/
issuectl rename old-slug new-slug        # rewrites every reference across the repo
issuectl fmt [--check] [--diff]          # normalise on-disk files
issuectl scan-todos [--create-inbox]     # find TODO(issue:slug) markers in source
```

`doctor --fix` is conservative: notes/comments merges that need
human judgement, malformed `AGENTS.md`, schema parse errors are
surfaced as findings rather than aborting the whole apply pass.

### Content & interop

```sh
issuectl attach <slug> screenshot.avif logs.txt
# Copies into issues/<slug>/attachments/; collisions auto-rename (shot-1.png, …).

issuectl attach <slug> --fixtures sample.json
# Targets issues/<slug>/fixtures/ instead.

issuectl import json --file dump.json
issuectl import github --repo owner/name           # via the `gh` CLI

issuectl export json    > snapshot.json
issuectl export markdown > status-report.md
issuectl export csv     > export.csv
```

### Web view

```sh
issuectl serve                           # http://127.0.0.1:7878
issuectl serve --port 9000               # different port
issuectl serve --host 0.0.0.0            # LAN access (no auth, no TLS — trusted networks only)
```

`serve` renders `issues/` as a Trello-style kanban (Open /
In progress / Testing / Closed + an "Other" catchall). On a loopback
bind, cards can be dragged between columns (PATCH-back to the on-disk
frontmatter) and bodies edited inline in the browser. Filter by type /
assignee / epic / label / cycle / reviewer; search across slug + title;
URL-encoded state survives reloads. The board falls back to read-only
when bound to a non-loopback address (no auth, no TLS, so writes are
gated to local-only by default — opt in explicitly for LAN access).
For the full security model and write semantics, run
`issuectl docs kanban`.

### Pointing to an external repo

```sh
issuectl --root ~/code/some-other-project list
issuectl --root /path/to/another/repo stats
```

## File format

Issues are markdown files with YAML frontmatter at
`issues/<slug>/item.md`. Optional sibling files:
`issues/<slug>/attachments/`, `issues/<slug>/fixtures/`, plus any
free-form `*.md` (e.g. `plan.md`, `analysis.md`) the agent or you
write.

A full-featured example:

```markdown
---
created: 2026-05-15
updated: 2026-05-31
type: bug
status: in-progress
priority: high
reporter: alice
assignee: bob
reviewer: cara
review_status: requested
epic: api-v2-migration
cycle: 2026-W22
size: M
related: ["@notably-brave-otter"]
blocked_by: ["@simply-fierce-comet"]
labels: [frontend, auth]
commits:
  - hash: abc1234
    summary: "fix(auth): redirect after SSO"
---

# Login loops on Safari after SSO

_Source: frontend/login_

## Description

Users get stuck in a 302 redirect loop after the SAML POST-back from
the IdP. Affects Safari 17 only.

## Reproduction

1. Open the app in Safari 17.0
2. Click "Sign in with SSO"
3. Complete the IdP flow
4. Observe the URL bar bouncing between `/auth/callback` and `/home`

## Acceptance Criteria

- [ ] Redirect chain unwinds on Safari 17
- [ ] Error case shows a friendly message
- [ ] Manual test on Safari 16.x

## Tests Run

- [ ] cargo test passes
- [ ] integration test for the redirect path added
```

### `issuectl fmt` — normalise on-disk files

```sh
issuectl fmt                             # rewrite every issues/<slug>/item.md
issuectl fmt some-slug another-slug      # specific slugs
issuectl fmt --check                     # CI: exit non-zero if anything would change
issuectl fmt --diff                      # print unified diff, no writes
issuectl --json fmt --check              # per-file JSON results
```

`fmt` is idempotent. It normalises:

- frontmatter key order (canonical sequence then unknown keys
  alphabetically),
- arrays (`labels` / `related` / `blocked_by`) sorted; `commits` is
  preserved in chronological order,
- markdown setext headings (`====`) rewritten to ATX (`#`),
- one blank line between `---` close and the body, no trailing
  whitespace, single final newline.

### Optional git merge driver

`issuectl merge-driver` is a custom three-way merge driver for
`issues/**/item.md`. It union-merges `labels` / `related` /
`blocked_by`, keeps `commits` as a hash-keyed log, and picks the
newer `updated:` — eliminating the most common cross-branch conflict
mode for file-based issue trackers. Scalar fields that diverge on
both sides still produce a conflict.

To enable:

```sh
# Add to .gitattributes (commit this):
echo 'issues/**/item.md merge=issuectl-yaml' >> .gitattributes

# Configure the driver locally (per-clone, not committed):
git config merge.issuectl-yaml.driver \
    "issuectl merge-driver --base %O --ours %A --theirs %B --output %A"

# Or print + apply for you:
issuectl install-merge-driver --apply
```

`install-merge-driver` never modifies `.gitattributes` itself —
that file is shared, so its contents are your decision.

## Schema & validation

`issues/.schema.yaml` declares the validation surface: required
fields, enum constraints, conditional rules, and migration aliases.
A repo-local schema layers on top of the built-in defaults — declare
only what you want to add or override.

```yaml
version: 1

fields:
  type:
    required: true
    enum: [bug, task, feature, improvement, chore, epic]
  status:
    required: true
    enum: [open, in-progress, testing, done, fixed, wontfix, duplicate, cannot-reproduce, obsolete]
  priority:
    required: true
    enum: [low, normal, high]

# A closing status implies the closed: date is set.
required_when:
  closed:
    when:
      status: [done, fixed, wontfix, duplicate, cannot-reproduce, obsolete]

# Legacy values doctor --fix coerces during migration:
status_aliases:
  closed: done
  resolved: fixed
  in_progress: in-progress
type_aliases:
  enhancement: improvement
  refactor: chore

# Optional: block --status done transitions on unchecked AC.
dod:
  strict: false
```

`doctor` enforces all of this read-only. `doctor --fix` applies the
alias coercions, fills in derived `closed:` dates, regenerates the
`.issuectl/AGENTS.md` schema-derived block, and migrates legacy
numbered or `open/`+`closed/` layouts to the canonical flat layout.

`issuectl context <slug>` reads the schema and injects the enum
constraints into the agent context bundle as system instructions, so
AI agents working from the bundle can't invent values outside the
schema.

## Agent integration

`issuectl init` (and `issuectl skill install`) writes a `/issue`
skill template into a target repo so an AI agent can drive issue
management through `issuectl` rather than poking at the filesystem.

| Agent       | Destination                       | Format                              |
| ----------- | --------------------------------- | ----------------------------------- |
| Claude Code | `.claude/skills/issue/SKILL.md`   | YAML frontmatter + markdown body    |
| Codex CLI   | `.codex/prompts/issue.md`         | Plain markdown prompt               |

```sh
issuectl skill install                   # Claude Code skill (default)
issuectl skill install --agent codex     # Codex prompt
issuectl skill install --agent all       # both
issuectl skill install --force           # refresh when binary > skill version
issuectl skill print [--agent codex]     # preview without installing
```

The skill instructs the agent to:

- delegate Search / List / Show / Create / Update / Close to
  `issuectl --json …`;
- prefer a descriptive 2-3 word `--slug` derived from the title;
- write body markdown (`## Reproduction`, `## Analysis`, epic
  `## Issues`/`## Phases` sections) directly, since structured body
  editing is out of scope for the CLI;
- when the installed binary is newer than the skill's pinned
  version, re-run `issuectl skill install --force` and
  `issuectl doctor` so instructions and repo schema both catch up.

Source templates live at
[`crates/issuectl-core/templates/issue-skill.md`](crates/issuectl-core/templates/issue-skill.md)
(Claude) and
[`crates/issuectl-core/templates/issue-prompt.md`](crates/issuectl-core/templates/issue-prompt.md)
(Codex) if you want to customize before installing.

### Context bundles

`issuectl context <slug>` renders a deterministic prompt bundle for
an issue: the issue body, parent epic, related and blocking refs,
acceptance criteria, recorded commits, and the schema rules an agent
must obey when proposing edits.

```sh
issuectl context login-redirect-loops                    # markdown to stdout
issuectl --json context login-redirect-loops             # JSON to stdout
issuectl context login-redirect-loops --write            # cache under .issuectl/cache/agent/<slug>/
```

The JSON form includes the same `version` token as
`issuectl --json show`, so an agent can pass it to
`--expected-version` on a follow-up `update` / `close` without a
second `show` round-trip.

### Repo-local prompt templates

`.issuectl/prompts/<template>.md` are markdown files with `{{key}}`
substitution against the context bundle (e.g. `{{slug}}`,
`{{title}}`, `{{body}}`, `{{epic_goal}}`, `{{acceptance_criteria}}`).
Any `## H2` heading in the issue body is reachable via its
snake-cased name — `## Risks` → `{{risks}}`, `## Test Plan` →
`{{test_plan}}`.

```sh
issuectl prompt implement login-redirect-loops
issuectl prompt implement login-redirect-loops --write    # cache to .issuectl/cache/agent/<slug>/prompts/
```

### Commit trailers and `sync-commits`

Add `Refs-Issue: @<slug>` (or `Fixes-Issue: @<slug>` to signal
"close-when-verified") to commit messages, then:

```sh
issuectl sync-commits                    # walk merge-base..HEAD and attach commits to issues
issuectl sync-commits --dry-run          # preview without writing
```

Idempotent — safe to re-run. The pre-commit hook
(`issuectl hooks install`) optionally runs `issuectl doctor` on
staged issue files so frontmatter problems surface before the commit
lands.

## Configuration

| Flag / env var                | Scope   | Description                                                                |
| ----------------------------- | ------- | -------------------------------------------------------------------------- |
| `--root <PATH>`               | global  | Override repo root (the dir containing `issues/`)                          |
| `--json`                      | global  | Emit a JSON envelope to stdout instead of human tables                     |
| `$ISSUECTL_USER`              | env     | `me:` query resolution and `--as` default for `note`                       |
| `$EDITOR` / `$VISUAL`         | env     | Used by `issuectl open <slug>`                                             |
| `$GIT_AUTHOR_NAME`            | env     | Fallback after `$ISSUECTL_USER` for `me:`                                  |

Without `--root`, `issuectl` walks up from the cwd looking for
`issues/` or `.git`.

`--json` is the contract surface for agents and CI:

- success (exit 0) → a single JSON value on stdout (object for action
  commands, array for list commands);
- error (exit ≠ 0) → an envelope on stderr
  `{"error":{"code":"<stable-kebab-code>","message":"…"[,...]}}`;
  stdout is empty. Validation errors, not-found, conflicts, and even
  bad flags (`code:"usage-error"`) all flow through this contract.

## Development

Requires a Rust toolchain (2021 edition, MSRV `1.82`).

```sh
cargo build
cargo test --workspace
cargo clippy --all-targets
cargo fmt --all --check
```

The workspace is `crates/issuectl-core` (library) +
`crates/issuectl` (CLI binary).

See
[AGENTS-AI-FIRST-CLI.md](AGENTS-AI-FIRST-CLI.md) for the design
principles every command follows, and
[docs/](docs/) for additional design notes and per-release digests
(e.g.
[`docs/releases/v0.6.0.md`](docs/releases/v0.6.0.md)).

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for
the PR process, dev setup, and coding conventions.

To report a security vulnerability, please follow the process in
[SECURITY.md](SECURITY.md).

## License

MIT — see [LICENSE](LICENSE).
