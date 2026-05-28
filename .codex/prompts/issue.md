
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

## Install or upgrade `issuectl`

This skill was installed for `issuectl 0.5.2`. On the
first invocation in a session, run `issuectl --version` and compare:

- **Missing**: install one of:
  - **Homebrew** (macOS/Linux): `brew install jarimustonen/issuectl/issuectl`
  - **Cargo** (any platform with a Rust toolchain): `cargo install issuectl`
  - **Shell installer** (no toolchain):
    `curl -LsSf https://github.com/jarimustonen/issuectl/releases/latest/download/issuectl-installer.sh | sh`
- **Older than `0.5.2`**: tell the user the skill expects
  `0.5.2` and suggest upgrading via the same channel
  they originally used (`brew upgrade jarimustonen/issuectl/issuectl`,
  `cargo install issuectl --force`, or re-run the shell installer).
  Stop and wait — schema/CLI surface may have changed.
- **Equal or newer**: proceed normally.

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
- Filter via flags: `issuectl --json ls -t bug -p high -a alice`
  - `-t/--type`: bug, task, feature, improvement, chore, epic
  - `-p/--priority`: normal, high
  - `-s/--status`: open, in-progress, testing, done, fixed, wontfix, duplicate, cannot-reproduce, obsolete
  - `-a/--assignee USERNAME` (matches `assignee` for issues, `owner` for epics)
  - `-l/--label LABEL`
  - `-e/--epic <slug>` (children of an epic)
- Filter via query string (same syntax as `search` and web `?q=`):
  - `issuectl --json ls "status:in-progress assignee:alice"`
  - `issuectl --json ls "-label:wontfix updated:<-14d"` (negation, relative date)
  - `issuectl --json ls "assignee:none"` (`any` / `none` for present/absent)
  - `issuectl --json ls 'text:"phrase to match"'` (quote multi-word text)
  - Supported fields: `status`, `type`, `priority`, `assignee`, `owner`,
    `epic`, `label`, `slug`, `folder`, `updated`, `created`, `closed`,
    `text`. Bareword (no `field:` prefix) is treated as `text:`.
  - Date filters use relative offsets: `<-14d` (strict), `<=-14d`
    (inclusive), and the same for `>` / `>=`. Anchor is today
    (local timezone — same as how `created`/`updated` are written).
    Use `<=0d` for "today or earlier" (don't write `+0d` in URLs;
    `+` URL-decodes to space).
  - Multiple terms AND together; no OR / parens in v1.
  - Escape inside an unquoted value: `\:` literal colon, `\\` literal
    backslash, `\ ` literal space, `\"` literal quote, `\-` at token
    start to escape negation. Or quote the whole value:
    `text:"foo:bar"`. Inside `"..."` only `\\` and `\"` are escapes
    — every other backslash is preserved literally, so paths and
    regex fragments survive (`text:"C:\temp"` matches `C:\temp`).
  - When a positional query is given to `ls`, the implicit "open
    only" default is dropped — combine with `--all`/`--closed` or
    `folder:`/`status:` to scope. Plain `--status fixed` (no
    positional query) still implies open-only, matching the old
    behavior.
- Include closed: `--all` (both) or `--closed` (only closed)
- Show details for one: `issuectl --json show <slug>`
- Search (same query syntax; bareword shorthand): `issuectl --json search KEYWORD [--all]`
  - Also: `issuectl --json search "deadlock text:flock"`
- Stats: `issuectl --json stats`
- Find likely duplicates (local heuristics — title/label/body-token overlap, no remote AI):
  - All open pairs: `issuectl --json duplicates` (alias `dups`)
  - Against one issue: `issuectl --json duplicates <slug>`
  - `--threshold 0.0..1.0` tunes sensitivity (default `0.30`); `--all` includes closed.
  - JSON (all-pairs): `[{a_slug,a_title,b_slug,b_title,score,title_overlap,body_overlap,label_overlap}]`, highest score first.
  - JSON (single `<slug>`): `[{slug,title,score,title_overlap,body_overlap,label_overlap}]`.

**Default scope**: `ls` (without a positional query) and `search` cover open
issues only. Add `--all` when the user asks for "all issues", "closed
issues", or "history of @<slug>".

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
- `-t/--type TYPE` (bug, task, feature, improvement, chore, epic, or any value the repo's `.schema.yaml` adds to `fields.type.enum` — rejected with `SchemaViolation` if the new type's required body sections are missing, with a list of `## <Section>` headings to add first; rejected if combined with a close→open reopen on the same call; rejected if the resulting type+`assignee`/`owner`/`reporter` combination violates the epic↔non-epic invariants `new` enforces)
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

Prefer commit trailers over manual `--add-commit`. Add
`Refs-Issue: @<slug>` (or `Fixes-Issue: @<slug>` to also signal
"close when verified") as the last paragraph of the commit
message, then run `issuectl sync-commits` to walk
`<merge-base..HEAD>` and append matching commits to each issue's
`commits[]`. Idempotent — safe to re-run. `--dry-run` previews
the plan; `--no-branch-fallback` disables the implicit
"branch named after a slug" attribution.

Output shape:

```json
{ "slug": "extremely-quiet-otter", "final_dir": "/abs/path/...",
  "moved_to_closed": false, "moved_to_open": false }
```

**Adding the issue to an epic**: also update the parent epic's `## Issues` list
in its item.md (CLI handles frontmatter only, not body sections).

### Action: Note

Append a timestamped block to an issue's `## Comments` section
(creating it if missing). Same flock + optimistic-version contract
as `update`; body-only mutation.

- `issuectl --json note <slug> --as <user> "<message>"`
- `--decision` appends to `## Decisions` instead.
- `--agent-run` appends to `## Agent Runs` instead.
- `--dry-run` prints a unified diff and exits 0 without writing.
- `--expected-version <token>` is required with `--json` (fetch via
  `show --json`).
- Transition-rule mismatches detected by `note` and `check` are
  emitted as warnings (stderr; `warnings` array in `--json`) — the
  write goes through. The unified `apply` path keeps rule violations
  as hard errors so they can be fixed in the same transaction.

Block shape (auto-generated):

```
### 2026-05-07T12:00:00Z · @alice

<message>
```

Reopen flow: `update --status <active>` on a closed issue
auto-appends a `## Reopen Notes — <today>` section in the same
write — no extra CLI step is needed.

### Action: Set / Check / Label / Apply (focused mutation verbs)

These wrap `update` for the common single-field and body-toggle
cases agents reach for. They share `update`'s flock + optimistic
concurrency contract — `--expected-version` is required with
`--json`, and every verb supports `--dry-run` (prints a unified
diff, no write).

- **`issuectl set <slug> <field> <value>`** — set a single
  frontmatter field. Built-in keys (`status`, `priority`,
  `assignee`, `owner`, `epic`) take the typed path; other keys go
  through the schema-validated `custom_fields` slot. Use
  `--clear` to remove a (non-status) field. Reserved keys like
  `labels` / `related` / `type` / `title` error with a hint
  pointing at the right flag.
- **`issuectl check <slug> "<task substring>"`** — toggle a
  unique `- [ ]` / `- [x]` line in the issue body. Errors when
  zero or multiple checkbox lines match the substring.
- **`issuectl label <slug> add|remove <label>`** — idempotent
  label add / remove.
- **`issuectl apply <patch.yaml>`** — multi-field transactional
  patch. The YAML file declares `slug:` plus any combination of
  built-in fields, `custom_fields:`, label / related list ops,
  commits, and a `body_ops:` list of body mutations applied in
  order under the same flock. Each body op is one of:

  ```yaml
  body_ops:
    - set_checkbox:
        match: "tests passing"
        checked: true            # idempotent: safe to retry
    - append_note:
        author: ci-bot
        message: "all checks green"
        section: agent_runs      # or comments (default) / decisions
  ```

  `set_checkbox` is idempotent — replaying the same op against an
  already-target body is a no-op (the box doesn't flip back). Rolls
  back cleanly on schema violation or any failed body op; the
  legacy → flat directory migration and default `.schema.yaml`
  bootstrap also defer until after validation passes, so a failing
  patch leaves no repo side effects.

JSON output shape (same envelope as `update`):

```json
{ "slug": "...", "version": "sha256:...",
  "moved_to_closed": false, "moved_to_open": false }
```

With `--dry-run`, the JSON envelope adds `"dry_run": true` and
`"diff": "<unified diff>"` and the on-disk file is untouched.

Output shape:

```json
{ "slug": "extremely-quiet-otter", "version": "sha256:...",
  "issue_dir": "/abs/path/issues/extremely-quiet-otter" }
```

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

**Prefer a descriptive slug.** Derive a short, human-readable 2-3 word
kebab-case slug from the title and pass it via `--slug` (e.g. "Login
redirect loops on safari" → `--slug login-redirect-loops`). Pick the words
that make the issue recognizable at a glance; drop filler. When no obvious
short slug exists (vague title, mostly stopwords, or fewer than two
meaningful words remain), **omit `--slug`** and let the CLI generate a
random `intensifier-adjective-noun` slug. Never put sensitive data
(customer names, emails, secrets) in the slug — it lands in the directory
name and git history; omit `--slug` for a random one instead. If `--slug`
collides with an existing issue, the CLI errors — retry with a different
descriptive slug or omit it for a random one.

```
issuectl --json new \
    --type bug \
    --title "Login redirect loops on safari" \
    --slug login-redirect-loops \
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
- Uses `--slug <kebab>` when given (validated: ≥2 lowercase ASCII kebab segments)
- Falls back to a random `intensifier-adjective-noun` slug when `--slug` is omitted
- Writes `issues/open/<slug>/item.md` with the right frontmatter
- Returns the slug and path in `--json` (parse `.slug`)

Other useful flags: `--epic <slug>`, `--label X` (repeatable), `--related "@<slug>"` (repeatable), `--field key=value` (repeatable; for custom frontmatter fields declared in `issues/.schema.yaml`, e.g. `--field team=payments`), `--check-duplicates` (refuse to create and exit non-zero — printing a JSON `{error:"duplicate-precheck",matches:[...]}` on stderr — when a strong duplicate already exists; re-run without the flag to create anyway).

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

### Action: View visually (kanban board)

If the user wants to **see** issues — "show me the board", "open the
kanban", "let me browse them visually" — start the read-only web board
and hand them the URL:

```
issuectl serve
# then open http://127.0.0.1:7878
```

The board is read-only; keep using the CLI for any create / update /
close action. For details (port/host flags, security model, routes),
run `issuectl docs kanban`.

### Action: Render an agent context bundle

When you (or another agent) need a deterministic snapshot of an issue and
its surroundings — parent epic, blockers, related issues, acceptance
criteria, recorded commits, and schema rules — use `issuectl context`:

- Markdown to stdout: `issuectl context <slug>`
- JSON to stdout: `issuectl --json context <slug>`
- Cache under `.issuectl/cache/agent/<slug>/` (gitignored): add `--write`

The bundle is byte-deterministic for a given issue state, which makes it
safe to cache. It is read-only — `issuectl context` never mutates files
under `issues/`. The JSON form includes a `version` token matching
`show --json`, so an agent can pass it straight to `--expected-version`
on a subsequent `update`/`close` without a separate `show` call.

### Action: Render a prompt template

Repo-local prompt templates live at `.issuectl/prompts/<name>.md` and
support `{{key}}` substitution against the context bundle (e.g.
`{{slug}}`, `{{title}}`, `{{body}}`, `{{version}}`, `{{epic_goal}}`,
`{{related}}`, `{{commits}}`, `{{context}}` for the full markdown
bundle). Any `## H2` heading in the issue body is also reachable via
its snake-cased name — `## Risks` → `{{risks}}`, `## Test Plan` →
`{{test_plan}}` — so templates can pull arbitrary sections without a
code change. Unknown keys are left intact so typos surface. Template
names must be plain filenames (no `/`, `\`, `..`, leading `.`).

- Print rendered prompt: `issuectl prompt <template> <slug>`
- Cache to `.issuectl/cache/agent/<slug>/prompts/<template>.md`: add `--write`

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

On `--fix`, the JSON envelope carries an `apply_outcome` object with a
`stop_phase` discriminator that you should branch on:

- `"ok"` — apply ran to completion; `blockers == []`.
- `"preflight"` — doctor refused to mutate the repo because of a
  critical finding. `fix_applied: false`, `blockers` lists the reasons.
  Resolve them and re-run.
- `"post_apply"` — `--fix` is forward-progress only: some phases
  already wrote to disk before a later safety re-check surfaced a new
  blocker. `fix_applied: true` AND `blockers != []` is the documented
  combination here. Resolve the blockers and re-run `--fix`; do NOT
  attempt to roll back the partial progress.

## Notes

- **Today's date** is set automatically by the CLI for `created`/`updated`
- Write issue content in English; Finnish text is fine in the body
- Prefer a descriptive 2-3 word `--slug` derived from the title (see Create → step 2); fall back to the random `intensifier-adjective-noun` slug only when no obvious short slug exists
- Default priority is `normal`; default status is `open`
- There is no default type — always pass `--type`
- All images must be AVIF — convert PNG/JPG/WebP first
- **Epic linkage**: prefer the `epic:` frontmatter field, value is the parent epic's slug
- **Closing statuses** also move the directory to `closed/`. Use `issuectl
  --json close` (or `update --status`) — never `git mv` by hand
- For raw filesystem operations, `issues/open/<slug>/item.md` is the format;
  but prefer the CLI for anything it supports
- **Always `--json`** when invoking `issuectl` from this skill
