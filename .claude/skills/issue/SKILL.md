---
name: issue
description: Manage issues and epics in issues/. Use when creating, searching, updating, or closing issues and epics.
---

# Issue Management

Manage issues and epics in `issues/` using the `issuectl` CLI as the primary
interface. The user's message determines the action:

- **Create**: user describes a problem, task, or feature → `issuectl new ...`
- **Search/list**: user asks to find, list, or check issues → `issuectl ls`, `issuectl show`, `issuectl search`
- **Close**: user says an issue is done/resolved → `issuectl close <slug>`
- **Update**: user wants to change status, assignee, or other details → `issuectl update <slug> ...`

Determine the action from the user's message and arguments. If unclear, ask.

**Always pass `--json`** to every `issuectl` command. The output is
structured and reliable to parse; the human-readable mode is for
terminal users only. All examples below already include `--json`.

`issuectl` validates inputs strictly (rejects unknown values for `--type`,
`--priority`, `--status`, etc.) and exits non-zero on errors. Read stderr
when a command fails — the error message names the offending value and
the valid alternatives.

## Identifiers

Issues are identified by random `intensifier-adjective-noun` slugs (e.g.
`extremely-quiet-otter`). The slug is the primary key in every command
that takes an issue argument. Body cross-references use `@<slug>` form
(e.g. `@extremely-quiet-otter`). The `epic:` and `related:` frontmatter
fields store bare slugs / `@<slug>` strings (no leading `#NN`).

## Arguments

Argument: $ARGUMENTS

## Actions

### Action: Search / List

Use the CLI rather than greppa hakemistoa. The CLI knows the frontmatter schema.

- List open issues: `issuectl --json ls`
- Filter: `issuectl --json ls -t bug -p high -a alice`
  - `-t/--type`: bug, task, feature, improvement, chore, epic
  - `-p/--priority`: normal, high
  - `-s/--status`: open, in-progress, testing, done, fixed, wontfix, duplicate, cannot-reproduce, obsolete
  - `-a/--assignee USERNAME` (matches `assignee` for issues, `owner` for epics)
  - `-l/--label LABEL`
  - `-e/--epic <slug>` (children of an epic)
- Include closed: `--all` (both) or `--closed` (only closed)
- Show details for one: `issuectl --json show <slug>`
- Keyword search across title/slug/body: `issuectl --json search KEYWORD [--all]`
- Stats: `issuectl --json stats`

**Default scope**: `ls` and `search` cover open issues only. Add `--all` when
the user asks for "all issues", "closed issues", or "history of @<slug>".

Process the JSON with `jq` to extract what the user asked for. Format the
result as a compact list when displaying back to the user (e.g.
`@<slug> — Title (type, status, assignee)`), not the raw JSON.

### Action: Close

Closing means setting a **closing status** and moving the issue to `closed/`.
The CLI does both atomically — never `git mv` by hand.

- `issuectl --json close <slug>` — defaults to `fixed` for bugs, `done` otherwise
- `issuectl --json close <slug> --status wontfix` — explicit closing status
- `issuectl --json close <slug> --commit HASH:summary` — also record a commit (repeatable)

Output shape:

```json
{ "slug": "extremely-quiet-otter",
  "final_dir": "/abs/path/issues/closed/extremely-quiet-otter",
  "moved_to_closed": true }
```

**Closing statuses** (any of these triggers move to `closed/`):

- `done` — work completed successfully (tasks, features, chores, epics)
- `fixed` — bug fix committed and verified
- `wontfix` — decided not to fix (by design, out of scope, etc.)
- `duplicate` — duplicate of another issue (also `--add-related "@<slug>"` via update first)
- `cannot-reproduce` — bug could not be reproduced
- `obsolete` — no longer relevant

**Steps**:
1. Determine the appropriate closing status from the user's message
2. Run `issuectl --json close <slug> [--status X] [--commit HASH:summary]`
3. **If closing an epic**: update the `## Issues` list in the epic's item.md with final statuses of all child issues (the CLI does not edit body markdown)
4. **If the issue belongs to an epic** (has `epic:` in frontmatter): update the parent epic's `## Issues` list to reflect the closed status
5. Confirm to user with the slug, title, closing status, and new location

**Batch close**: if the user provides multiple slugs, run `issuectl
--json close` for each. Confirm each one.

### Action: Update

Use `issuectl --json update <slug>` with one or more flags. The CLI updates
frontmatter and bumps `updated:` automatically. If the new status is a
closing status, the issue is also moved to `closed/` (same as `close`).

Common flags:

- `--status STATUS` (active or closing)
- `--assignee USER` / `--owner USER` (epics)
- `--priority normal|high`
- `--epic <slug>` / `--no-epic`
- `--add-label LABEL` / `--remove-label LABEL` (repeatable)
- `--add-related "@<slug>"` / `--remove-related "@<slug>"` (repeatable; bare slug also accepted)
- `--add-commit HASH:summary` (repeatable)

Example flows:

- `issuectl --json update extremely-quiet-otter --status in-progress`
- `issuectl --json update extremely-quiet-otter --assignee alice --status testing`
- `issuectl --json update extremely-quiet-otter --add-commit "abc123:fix login state"`
- `issuectl --json update extremely-quiet-otter --add-label backend --add-label api`

Output shape:

```json
{ "slug": "extremely-quiet-otter", "final_dir": "/abs/path/...",
  "moved_to_closed": false, "moved_to_open": false }
```

**Adding the issue to an epic**: also update the parent epic's `## Issues` list
in its item.md (CLI handles frontmatter only, not body sections).

### Action: Create

#### 1. Gather Information

If `$ARGUMENTS` already provides enough context, use it. Otherwise ask the user
interactively for missing details. Tailor questions to the issue type.

Possible questions:

- **What type?** — bug, task, feature, improvement, chore, or epic (infer from
  context: X is broken = bug, we need to build Y = feature/task, set up Z = chore)
- **What is the problem/goal?** — clear description
- **Where does it happen?** — service / page / feature → `--source`
- **How to reproduce?** — bugs only; goes into the body `## Reproduction` section
- **Reporter** — `whoami` or ask
- **Assignee** — ask if not known
- **Priority** — normal or high (default normal)
- **Epic** — does this belong to an existing epic? Check with `issuectl --json ls -t epic`

**Epic suggestion**: if the user describes a multi-week, 3+ task initiative,
suggest creating an epic instead.

#### 2. Create with the CLI

```
issuectl --json new \
    --type bug \
    --title "Login redirect loops on safari" \
    --reporter alice \
    --assignee bob \
    --priority normal \
    --source "frontend/login" \
    --description "Users get stuck in a 302 loop after SSO redirect."
```

For epics, use `--owner` instead of `--reporter`/`--assignee`:

```
issuectl --json new --type epic --title "API v2 migration" --owner cara --priority high
```

Output shape:

```json
{ "slug": "extremely-quiet-otter",
  "title": "Login redirect loops on safari",
  "item_path": "/abs/path/issues/open/extremely-quiet-otter/item.md",
  "dir": "/abs/path/issues/open/extremely-quiet-otter" }
```

The CLI:
- Generates a random `intensifier-adjective-noun` slug automatically
- Writes `issues/open/<slug>/item.md` with the right frontmatter
- Returns the slug and path in `--json` (parse `.slug`)
- Optionally accepts `--slug <kebab>` to override the auto-generated value

Other useful flags: `--epic <slug>`, `--label X` (repeatable), `--related "@<slug>"` (repeatable).

#### 3. Flesh out the body

`issuectl new` writes a minimal body (`# Title`, optional `_Source: ..._`,
`## Description`). For bugs, append `## Reproduction` and `## Quick Test`
sections by editing the item.md directly (use the `dir` or `item_path`
from the JSON output to find it). For epics, add `## Goal`, `## Issues`,
`## Phases`, and `## Notes` sections — the CLI does not write these.

#### 4. Copy Screenshots

If the user provides image file paths, convert them to AVIF and copy them
into the issue directory. Reference them in item.md with relative paths.

#### 5. Confirm

Show the created issue/epic path and a brief summary.

### Action: Doctor (repository health-check + migration)

If the user asks to "check the repo" or "migrate legacy issues", use
`issuectl doctor`:

- Read-only report: `issuectl --json doctor`
- Apply migrations and fixes: `issuectl --json doctor --fix`

Doctor migrates legacy `<NN>-<slug>/` directories to slug-only layout,
rewrites `number:` → `slug:` in frontmatter, migrates `epic:` and
`related:` references, and rewrites `#NN` body refs to `@<slug>`. It
also flags invalid slugs, duplicates, missing item.md files, and orphan
epic refs.

## Notes

- **Today's date** is set automatically by the CLI for `created`/`updated`
- Write issue content in English; Finnish text is fine in the body
- Slugs are random `intensifier-adjective-noun`; override via `--slug` only when there's a strong reason
- Default priority is `normal`; default status is `open`
- There is no default type — always pass `--type`
- All images must be AVIF — convert PNG/JPG/WebP first
- **Epic linkage**: prefer the `epic:` frontmatter field, value is the parent epic's slug
- **Closing statuses** also move the directory to `closed/`. Use `issuectl
  --json close` (or `update --status`) — never `git mv` by hand
- For raw filesystem operations, `issues/open/<slug>/item.md` is the format;
  but prefer the CLI for anything it supports
- **Always `--json`** when invoking `issuectl` from this skill
