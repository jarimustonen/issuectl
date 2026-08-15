
# Issue Management

Manage issues and epics in `issues/` using the `issuectl` CLI as the primary
interface. The user's message determines the action:

- **Create**: user describes a problem, task, or feature → `issuectl new "<title>" ...` (the slug is derived from the title by default; see "Identifiers" for slug policy)
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

### `--json` output contract

Every command follows one contract so you can consume any of them the
same way:

- **Success (exit 0)** → a single JSON value on **stdout**. Read commands
  (`show`, `ls`, `search`) print the issue (object) or issues (array);
  action commands (`new`, `update`, `close`, `note`, `apply`, …) print a
  result object describing what changed.
- **Error (exit ≠ 0, nothing produced)** → a single object on
  **stderr**: `{"error":{"code":"<stable-kebab-code>","message":"…"}}`,
  sometimes with extra keys inside `error` (e.g. `matches` for a
  duplicate precheck). stdout is empty. This covers validation errors,
  not-found, conflicts, and even bad flags (`code:"usage-error"`).
- **Partial success (exit ≠ 0, work landed)** → the command still prints
  its normal **result object on stdout** (e.g. `import` with
  `created`/`failed` counts), not the error envelope.
- **Exit codes**: `0` success · `2` refused-but-actionable (duplicate
  precheck strong match → error envelope on stderr; partial import where
  some records landed → result object on stdout) · `1` everything else
  (validation error, not-found, bad flag, conflict). **Branch on the
  exit code first**, then decide whether to read stdout or stderr.
- **Shared field vocabulary** (same key, same meaning everywhere):
  `slug`, `title`, `version` (optimistic-concurrency token — pass back as
  `--expected-version`), `dir` (the issue's directory), `path` (a single
  file), `dry_run` (bool), `diff` (unified-diff string), `warnings`
  (string array). `open` uses `is_dir` (bool: was `--dir` requested) so it
  never collides with the `dir` directory field.

## Install or upgrade `issuectl`

This skill was installed for `issuectl 0.10.0`. On the
first invocation in a session, run `issuectl --version` and compare:

- **Missing**: install one of:
  - **Homebrew** (macOS/Linux): `brew install jarimustonen/issuectl/issuectl`
  - **Cargo** (any platform with a Rust toolchain): `cargo install issuectl`
  - **Shell installer** (no toolchain):
    `curl -LsSf https://github.com/jarimustonen/issuectl/releases/latest/download/issuectl-installer.sh | sh`
- **Older than `0.10.0`**: tell the user the skill expects
  `0.10.0` and suggest upgrading via the same channel
  they originally used (`brew upgrade jarimustonen/issuectl/issuectl`,
  `cargo install issuectl --force`, or re-run the shell installer).
  Stop and wait — schema/CLI surface may have changed.
- **Newer than `0.10.0`**: the installed binary is ahead
  of what this skill was written for. Tell the user to refresh the
  skill so the instructions match the CLI surface they actually have:
  `issuectl skill install --force` (Claude Code; add `--agent codex`
  for Codex or `--agent all` for both). Then run `issuectl doctor`
  (or `issuectl doctor --fix`) — a newer binary often ships schema
  rules or migrations the repo hasn't picked up yet. Continue with
  the task once both are done.
- **Equal**: proceed normally.

## Identifiers

Issues are identified by short kebab-case slugs (the primary key in
every command that takes an issue argument). By default `issuectl new`
**derives a descriptive 2-3 word slug from the title** (e.g.
`login-redirect-loops`) — the slug shows up in directory names, branch
names, and every agent command, so a recognizable one pays off. Pass
`--slug` to override the derived default with an explicit slug. The CLI
falls back to a random `intensifier-adjective-noun` slug (e.g.
`extremely-quiet-otter`) only when the title yields no sensible slug, or
when you force it with `--slug-random` (for a title that would leak
sensitive data) — see "Action: Create → step 2" for the operational
details. Body
cross-references use `@<slug>` form. The `epic:` and `related:`
frontmatter fields store bare slugs / `@<slug>` strings (no leading
`#NN`).

## Arguments

Argument: $ARGUMENTS

## Actions

### Action: Search / List

Use the CLI rather than greppa hakemistoa. The CLI knows the frontmatter schema.

- List open issues: `issuectl --json ls`
- Filter via flags: `issuectl --json ls -t bug -p high -a alice`
  - `-t/--type`: bug, task, feature, improvement, chore, epic
  - `-p/--priority`: low, normal, high
  - `-s/--status`: untriaged, open, in-progress, testing, needs-info, deferred, done, fixed, wontfix, duplicate, cannot-reproduce, obsolete (the `untriaged` / `needs-info` / `deferred` active states come from the standard intake flow — see "Action: Intake")
  - `-a/--assignee USERNAME` (matches `assignee` for issues, `owner` for epics)
  - `-l/--label LABEL`
  - `-e/--epic <slug>` (children of an epic)
  - Bare `ls` is open-only, but — absent an explicit `--all`/`--closed` —
    pinning `-s/--status` (or a `status:`/`folder:` query term) lifts that
    open-only default, so `ls -s done` / `ls -s fixed` list closed and
    archived issues too. `--all`/`--closed` still win: `ls --closed -s done`
    stays scoped to the closed folder.
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
- Epic hierarchy as a tree: `issuectl --json epic tree <slug>` renders the
  epic plus its child issues (issues whose `epic:` points at it; a child
  epic is expanded in turn). Read-only. `--json` emits it structurally —
  a node object `{slug,title,status,priority,type,children:[…]}` for one
  epic, or an array of such nodes for every top-level epic when the slug
  is omitted (`issuectl --json epic tree`). A missing slug exits `1` with
  the `not-found` error envelope.
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
- `issuectl --json close <slug> --as <user>` — record the closer as the `closed_by:` frontmatter field (optional; same author grammar as `note --as`)
- `issuectl --json close <slug> --commit HASH:summary` — also record a commit (repeatable)
- `issuectl --json close <slug> --stamp` — after closing, rewrite the current HEAD commit's message to append a `Fixes-Issue: @<slug>` trailer, so the trailer-driven `issuectl changelog` picks up the landing commit with zero manual trailer discipline. Run it **after** committing the fix (it stamps whatever HEAD is) and **before** pushing/merging (rewriting changes HEAD's sha). Message-only — tree, author, and dates are preserved and the index is untouched. Fail-safe: it never blocks the close — the `stamp` object in the JSON reports `{"status":"stamped","sha":...,"previous_sha":...}`, `{"status":"already_present","sha":...}`, or `{"status":"skipped","reason":...}` (HEAD detached / a merge commit / signed / mid rebase-cherry-pick-merge-revert / no commit to stamp). Cannot be combined with a `--commit` that resolves to HEAD (the rewrite would orphan that recorded sha).

Output shape (`closed_by` present only when `--as` is passed; `stamp` present only when `--stamp` is passed):

```json
{ "slug": "extremely-quiet-otter",
  "dir": "/abs/path/issues/closed/extremely-quiet-otter",
  "moved_to_closed": true, "version": "sha256:...", "closed_by": "jari" }
```

Reopening (`update --status <active>`) clears `closed_by` alongside `closed:`.

**Closing statuses** (any of these triggers move to `closed/`):

- `done` — work completed successfully (tasks, features, chores, epics)
- `fixed` — bug fix committed and verified
- `wontfix` — decided not to fix (by design, out of scope, etc.)
- `duplicate` — duplicate of another issue (also `--add-related "@<slug>"` via update first)
- `cannot-reproduce` — bug could not be reproduced
- `obsolete` — no longer relevant

**Steps**:
1. Determine the appropriate closing status from the user's message
2. Run `issuectl --json close <slug> [--status X] [--as <user>] [--commit HASH:summary]`
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
- `--priority low|normal|high`
- `--epic <slug>` / `--no-epic`
- `--add-label LABEL` / `--remove-label LABEL` (repeatable)
- `--add-related "@<slug>"` / `--remove-related "@<slug>"` (repeatable; bare slug also accepted)
- `--add-blocked-by "@<slug>"` / `--remove-blocked-by "@<slug>"` (repeatable; bare slug also accepted) — set/clear DAG dependency edges (this issue is blocked by `<slug>`). Same shape as `--add-related`; equivalent to `issuectl depend add/remove`.
- `--add-commit HASH:summary` (repeatable)
- `--lane NAME` / `--no-lane`, `--lane-seq <int>` / `--no-lane-seq`, and `--add-collision TOKEN` / `--remove-collision TOKEN` — optional scheduling-DAG hints (a lane is a spawn-time mutual-exclusion group; collision tokens are extra shared "hot files"; `lane_seq` is a coarse intra-lane precedence key consulted after `blocked_by` and priority but before the slug tie-break, so you can pin "do this member before that one" without a fake dependency). The reserved lane value `--lane unlaned` marks an issue *confirmed parallel-safe*: `dag` treats it as independently spawnable and never serializes it with siblings (distinct from an absent lane, which means "unclassified"). Only useful to an orchestrator; `issuectl dag [--json]` renders the resulting lanes, per-lane order, `blocked_by` mirror, and computed head-of-line (deterministic, AI-first). An issue whose `status` is `in-progress` is never `spawnable` (work already underway). Pass `dag --reservations <file|-|json>` to have spawnability account for lane/collision tokens in-flight runs hold.

Example flows:

- `issuectl --json update extremely-quiet-otter --status in-progress`
- `issuectl --json update extremely-quiet-otter --assignee alice --status testing`
- `issuectl --json update extremely-quiet-otter --add-commit "abc123:fix login state"`
- `issuectl --json update extremely-quiet-otter --add-label backend --add-label api`
- `issuectl --json update extremely-quiet-otter --add-blocked-by "@other-slug"` (gate this issue behind `@other-slug`)

Prefer commit trailers over manual `--add-commit`. Add
`Refs-Issue: @<slug>` (or `Fixes-Issue: @<slug>` to also signal
"close when verified") as the last paragraph of the commit
message, then run `issuectl sync-commits` to walk
`<merge-base..HEAD>` and append matching commits to each issue's
`commits[]`. Idempotent — safe to re-run. `--dry-run` previews
the plan; `--no-branch-fallback` disables the implicit
"branch named after a slug" attribution.

To seed the changelog trailer without any manual discipline, close
the issue with `issuectl close <slug> --stamp` right after committing
the fix — it stamps the `Fixes-Issue: @<slug>` trailer onto HEAD for
you (see the Close action above).

Output shape:

```json
{ "slug": "extremely-quiet-otter", "dir": "/abs/path/...",
  "version": "sha256:...", "moved_to_closed": false, "moved_to_open": false }
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
- `--expected-version <token>` is **optional** on `--json` (opt-in
  compare-and-swap): when passed, the write fails on a version mismatch;
  it is enforced whenever passed. **Pass it** when your flow is
  read-then-write and another writer could interleave (multiple agents,
  human + agent) — fetch the token from `show --json`
  and pass it back unchanged. Omit it only when you are the sole writer;
  omitting it is an unguarded write (a concurrent update can be lost —
  `flock` still prevents a corrupt/torn file, but it does not detect a
  stale read).
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
concurrency contract — `--expected-version` is **optional** on
`--json` (opt-in compare-and-swap, enforced when passed), and every
verb supports `--dry-run` (prints a unified diff, no write). The one
exception is `apply`: its patch file must still declare a non-empty
`expected_version:` when invoked with `--json` (the transactional
multi-field patch keeps its own concurrency contract).

- **`issuectl set <slug> <field> <value>`** — set a single
  frontmatter field. Built-in keys (`status`, `priority`,
  `assignee`, `owner`, `epic`) take the typed path; other keys go
  through the schema-validated `custom_fields` slot. Use
  `--clear` to remove a (non-status) field. Reserved keys like
  `labels` / `related` / `type` / `title` error with a hint
  pointing at the right flag.
- **`issuectl assign <slug> <user>`** — convenience wrapper for
  `set <slug> assignee <user>`; routes through the identical typed
  path, so validation, idempotency, and the
  `--json`/`--expected-version` contract are the same. Use
  `issuectl assign <slug> --clear` to unassign (mirrors
  `set --clear`).
- **`issuectl check <slug> "<task substring>"`** — toggle a
  unique `- [ ]` / `- [x]` line in the issue body. Errors when
  zero or multiple checkbox lines match the substring.
- **`issuectl label <slug> add|remove <label>`** — idempotent
  label add / remove. The operation also accepts the flag form
  `issuectl label <slug> --add|--remove <label>`; pass exactly one
  form. A malformed invocation under `--json` emits the standard
  error envelope (never a silent no-op).
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
  "dir": "/abs/path/issues/extremely-quiet-otter" }
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
- **Priority** — low, normal, or high (default normal)
- **Epic** — does this belong to an existing epic? Check with `issuectl --json ls -t epic`

**Epic suggestion**: if the user describes a multi-week, 3+ task initiative,
suggest creating an epic instead.

#### 2. Create with the CLI

**The slug is derived from the title by default.** `issuectl new "Login
redirect loops on safari"` auto-derives `login-redirect-loops` — lowercased,
stop-words stripped, trimmed to 2-3 words — so you normally don't pass
`--slug` at all. If the derived slug collides with an existing issue, the
CLI silently disambiguates with a numeric suffix (`-2`, `-3`, …). Pass an
explicit `--slug <kebab>` only to override the derived default with a
different descriptive slug; an explicit `--slug` that collides errors
(retry with a different slug). When the title would leak sensitive data
(customer names, emails, secrets) into the directory name and git history,
pass **`--slug-random`** to force a random `intensifier-adjective-noun`
slug instead. The CLI also falls back to a random slug automatically when
the title yields no sensible slug (empty, all stop-words, or non-ASCII).

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

(The slug is derived from the title — `login-redirect-loops` here — so no
`--slug` is needed. Add `--slug <kebab>` to override, or `--slug-random`
to force a random slug.)

`create` is accepted as an alias for `new`, and `--body` as an alias
for `--description`, so `issuectl --json create --type task --title X
--body "…"` works identically — canonical forms stay `new` /
`--description`. To set the initial body from a file instead of inline
text, use `--body-file <path>` (mutually exclusive with
`--description`/`--body` — combining them is a usage error); pass `-`
to read the body from stdin (use `./-` for a file literally named `-`).
The file's markdown is written below the `# <title>` heading, so a
fully-formed issue can be filed in one argv:

```
issuectl --json new --type feature --title "Bulk export" --body-file notes.md
printf '## Context\n\nPiped body.\n' | issuectl --json new --type task --title X --body-file -
```

The title may also be passed positionally
(`issuectl new "Login redirect loops" --type bug`) instead of via
`--title`; pass exactly one of the two (both/neither is an error).

For epics, use `--owner` instead of `--reporter`/`--assignee`:

```
issuectl --json new --type epic --title "API v2 migration" --owner cara --priority high
```

Output shape:

```json
{ "slug": "login-redirect-loops",
  "title": "Login redirect loops on safari",
  "item_path": "/abs/path/issues/open/login-redirect-loops/item.md",
  "dir": "/abs/path/issues/open/login-redirect-loops" }
```

The CLI:
- Uses `--slug <kebab>` when given (validated: ≥2 lowercase ASCII kebab segments)
- Otherwise derives a 2-3 word kebab slug from the title (numeric suffix on collision); `--slug-random`, or an unsluggable title, yields a random `intensifier-adjective-noun` slug instead
- Writes `issues/open/<slug>/item.md` with the right frontmatter
- Returns the slug and path in `--json` (parse `.slug`)

Other useful flags: `--epic <slug>`, `--label X` (repeatable), `--related "@<slug>"` (repeatable), `--field key=value` (repeatable; for custom frontmatter fields declared in `issues/.schema.yaml`, e.g. `--field team=payments`), `--check-duplicates` (refuse to create and exit 2 — printing the shared error envelope `{"error":{"code":"duplicate-precheck","message":...,"matches":[...]}}` on stderr — when a strong duplicate already exists; re-run without the flag to create anyway).

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

### Action: Intake (standard intake flow)

`issuectl intake` is the first-class flow for **filing and triaging** incoming
bug reports and feature requests, replacing the old ad-hoc label scheme
(`via:telegram` + `needs-triage`). Intake *state* lives in `status`, not in
labels. See `docs/design/intake-flow.md`. Two audiences share one namespace: a
**reporting agent** files; a **developer / product-manager** dispositions.

The flow adds three **active** statuses and a set of intake fields:

- **Statuses**: `untriaged` (filed, awaiting a triage decision — the reception
  state), `needs-info` (filed but un-actionable pending reporter input),
  `deferred` (worthwhile but intentionally not scheduled now). All three are
  *active* (not closing); they show up in `ls`/queries like any active issue.
- **Fields**: `provenance` (where it came from — telegram/email/github/…; a
  first-class field distinct from the body `--source` line, open-valued unless a
  repo declares an `enum:`), `provenance_detail` (free text for the `other`
  case), `source_ref` (external message id, the idempotency key),
  `disposition_reason` (enum `by-design | out-of-scope | wontfix | withdrawn |
  superseded` — the structured *why* for a closing disposition), `disposition_note`
  (free-text specifics), `duplicate_of` (directed slug link for a `duplicate`),
  `deferred_until` (wake-up date for a parked item).

#### Filing (reporting agent)

```
issuectl intake file --type bug --title "Login loops on Safari" \
  --body-file report.md --reporter alice \
  --provenance telegram --source-ref "chat:123/message:456" \
  [--provenance-detail "…"] [--priority high] [--slug login-loops] [--label …] --json
```

- Lands the item directly in `untriaged`; the filer never names the entry state.
- Takes any non-`epic` type. `--body-file -` reads stdin; `--body "<text>"` for a
  one-liner. Protected keys (`status`, `type`, `reporter`, `provenance`, …) are
  rejected via `--field` — use the dedicated flags.
- **Idempotent on `(provenance, source-ref)`**: a retry returns the existing item
  with `"deduplicated": true` (exit 0) instead of creating a second issue.
- Output: `{ "slug", "status": "untriaged", "dir", "version", "deduplicated" }`.
- `issuectl intake withdraw <slug> --reason "…"` retracts an untriaged report
  (`→ wontfix` with `disposition_reason: withdrawn`) — by convention the reporter
  uses it; the CLI does not enforce reporter identity.

The `/issue-new` skill wraps this filing path faithfully (verbatim capture +
`issuectl attach` for screenshots).

#### Inspecting the queue (developer / PM)

```
issuectl intake queue --json                     # default: untriaged, oldest first
issuectl intake queue --json --needs-analysis    # only items lacking a ## Triage analysis section
issuectl intake queue --json --state deferred    # a non-default view (deferred|needs-info)
issuectl intake queue --json --type bug --provenance telegram
issuectl intake show <slug> --json               # item + attachments + analysis section
```

`queue` is a stable projection of the actionable `untriaged` set (both bugs and
feature requests, every provenance). `deferred`/`needs-info` are excluded from
the default view. Each row carries `needs_analysis` (derived from the presence of
a `## Triage analysis` body section — there is no stored analysis state). `show`
adds `attachments` (names) and `analysis` (the section text, or `null`).

#### Dispositions (developer / PM — each a first-class transition)

```
issuectl intake accept    <slug> [--assignee <who>] [--priority low|normal|high] --json  # → open
issuectl intake defer     <slug> --reason "…" [--until <date>]       --json  # → deferred
issuectl intake need-info <slug> --reason "…"                        --json  # → needs-info
issuectl intake reject    <slug> --reason "…" [--kind by-design|wontfix|out-of-scope] --json  # → wontfix + disposition_reason
issuectl intake cannot-reproduce <slug> --reason "…"                 --json  # → cannot-reproduce (bug-only)
issuectl intake duplicate <slug> --of <canonical-slug>               --json  # → duplicate + duplicate_of
issuectl intake obsolete  <slug> --reason "…" [--superseded-by <slug>] --json  # → obsolete
issuectl intake retype    <slug> --to <type>                         --json  # reclassify the type hint
issuectl intake reopen    <slug> [--to untriaged|open] --reason "…"  --json  # closing → active
```

- `--reason` is **required** on `defer`, `need-info`, `reject`, `cannot-reproduce`,
  `obsolete`, `reopen`, `withdraw` — the *why* is captured structurally, not left
  in prose.
- `reject --kind` **defaults to `wontfix`** when omitted; pass `by-design` /
  `out-of-scope` explicitly for those. `reopen --to` defaults to `untriaged` when
  omitted. `cannot-reproduce` is bug-only.
- Each transition validates the source state intrinsically (you cannot `accept` a
  closed item, etc.) and returns `{ "slug", "status", "dir", "version" }`. Stable
  error codes include `transition-illegal`, `duplicate-source-ref`,
  `protected-field`.

**Never file a reception item with plain `new`** — `new` fixes the creation
status at `open`. Reception filing goes through `issuectl intake file`.

The `/issue-intake` skill drives the developer/PM side (queue → drive
`/worktree-bug-analysis` on unclear items → PO briefing → stop; the disposition
is the user's).

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
- The slug is derived from the title by default (see Create → step 2); pass `--slug` to override, or `--slug-random` when the title would leak sensitive data
- Default priority is `normal`; default status is `open` (except the intake
  flow, which files into `untriaged` via `issuectl intake file` — see "Action:
  Intake")
- **Intake flow**: incoming bug reports / feature requests are filed and triaged
  through the `issuectl intake` command group (statuses `untriaged` / `needs-info`
  / `deferred`; fields `provenance` / `source_ref` / `disposition_reason` /
  `duplicate_of` / `deferred_until`). See "Action: Intake".
- There is no default type — always pass `--type`
- All images must be AVIF — convert PNG/JPG/WebP first
- **Epic linkage**: prefer the `epic:` frontmatter field, value is the parent epic's slug
- **Closing statuses** also move the directory to `closed/`. Use `issuectl
  --json close` (or `update --status`) — never `git mv` by hand
- For raw filesystem operations, `issues/open/<slug>/item.md` is the format;
  but prefer the CLI for anything it supports
- **Always `--json`** when invoking `issuectl` from this skill
