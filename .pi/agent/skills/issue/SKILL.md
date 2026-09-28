---
name: issue
description: Manage issues and epics in issues/ with the issuectl CLI. Use when creating, searching, updating, closing, or triaging issues and epics, or when an agent needs to read or write issue data.
---

# Issue Management

This repository tracks its work as markdown issues under `issues/`. Each issue
is a directory holding `item.md`: YAML frontmatter plus a markdown body. The
`issuectl` CLI is the interface to that tree. It knows the schema in
`issues/.schema.yaml`, holds the repo-wide write lock, validates every write,
and emits a version token you can use for optimistic concurrency. Hand edits
to frontmatter get none of that, so frontmatter goes through the CLI. Body
markdown is yours to edit when the CLI has no verb for what you need; the
CLI writes the frontmatter and a few structured body blocks, nothing more.

Argument: $ARGUMENTS

Read the request and decide what it calls for: filing something new, finding
or showing issues, changing fields, closing, or working an intake queue. Most
issue writes are cheap to correct (reopen, retitle, close as duplicate), so
when a reading is reasonable, take it, say what you chose, and move on. Ask
when the outcomes differ in a way the user would care about and you cannot
tell which they want: which of several similar issues they mean, or whether
"drop it" means `wontfix` or `obsolete`.

## Version pin

This skill was installed for `issuectl 0.18.6`. It describes
the CLI surface and the JSON shapes of that version. Once per session, compare
against `issuectl --version`:

- **Missing.** Install with Homebrew (`brew install
  jarimustonen/issuectl/issuectl`), Cargo (`cargo install issuectl`), or the
  shell installer for machines without a toolchain:
  `curl -LsSf https://github.com/jarimustonen/issuectl/releases/latest/download/issuectl-installer.sh | sh`
- **Newer than the pin.** The binary has moved on; this text may describe
  flags or shapes that changed. Refresh the skill with
  `issuectl skill install --force` (all bundled skills for Claude, pi, and
  Codex; narrow with `--agent`), then run `issuectl doctor` (or `doctor
  --fix`) because a newer binary often ships schema rules or migrations the
  repo has not picked up. Continue once both are done.
- **Older than the pin.** Commands described here may not exist and the
  schema may differ. Tell the user which version the skill expects and
  suggest upgrading through the channel they used (`brew upgrade
  jarimustonen/issuectl/issuectl`, `cargo install issuectl --force`, or the
  installer again). Proceeding against a mismatched binary is how agents
  produce confidently wrong commands, so wait for the upgrade.

## Talking to the CLI

Pass `--json` on every call. The human-readable mode is for terminals; the
JSON mode is the stable contract. The global flag goes anywhere on the
command line (`issuectl --json ls` and `issuectl ls --json` are the same).

The CLI's own help is the source of truth for flags and accepted values, and
it is machine-readable: `issuectl <command> --help --json` returns `data` with
`subcommands`, `flags` (each with `long`, `short`, `possible_values`, and a
`description`), `args`, and `examples`. Use it for any command whose exact
surface you are unsure of instead of relying on this file's examples.

Input validation is strict: an unknown `--type`, `--priority`, or `--status`
value, a missing required flag, or a bad combination is rejected before
anything is written, and the error names the offending value and the valid
alternatives.

### The `--json` contract

- **Success**, including partial success, is one object on stdout:
  `{"schema_version":1,"data":…,"warnings":[]}`. Domain fields live only in
  `data`. Non-fatal advisories live only in the top-level `warnings` array,
  and they carry things you would otherwise miss: the derived-slug notice, a
  Definition-of-Done shortfall, a preserved title. Read them.
- **Error**, with no work landed, is one object on stderr and an empty stdout:
  `{"schema_version":1,"error":{"code":"<stable-kebab-code>","message":"…"}}`,
  with any extra context inside `error`. Clap usage errors use the code
  `usage-error`; not-found is `not-found`; illegal status transitions are
  `transition-illegal`.
- **Exit codes.** `0` success. `2` refused-but-actionable: a `--check-duplicates`
  strong match or an illegal status transition (error envelope on stderr) or a
  partial `import` where some records landed (success envelope on stdout). `1`
  everything else. Branch on the exit code, then decide which stream to read.
- **Shared vocabulary.** The same key means the same thing everywhere: `slug`,
  `title`, `version` (the concurrency token), `dir` (the issue directory),
  `path` (a single file), `dry_run`, `diff` (unified diff), `warnings`.
  `open` reports `is_dir` (was `--dir` requested) so it never collides with
  the `dir` directory field.
- `schema_version` is the version of this JSON contract. It bumps only for
  breaking changes; new fields do not bump it. `issuectl version --json`
  reports `supported_schemas` and each bundled skill's version pin.

### Concurrency

Every mutation returns `version`, and `show`, `context`, and `intake show`
carry the current one. `--expected-version` is optional on the single-field
verbs under `--json` and is enforced whenever passed. Pass it back unchanged
when your flow is read-then-write and another writer could interleave
(several agents, or a human alongside you). The flock prevents a torn file;
only the token detects a stale read. A patch file given to `apply` or `update
--patch-file` must declare a non-empty `expected_version:` under `--json`,
because a multi-field patch assembled from an earlier `show` is exactly the
shape a stale token protects.

## Identifiers and layout

Issues are keyed by short kebab-case slugs. `create` derives one from the
title (two or three significant words, stop-words dropped, e.g. `login-redirect-loops`),
and disambiguates a collision with `-2`, `-3` and a warning. The slug appears
in directory names, branch names, and every later command, so a recognizable
one pays off. `--slug <kebab>` overrides the derivation; an explicit slug that
collides is an error. `--slug-random` forces a random
`intensifier-adjective-noun` slug (`extremely-quiet-otter`) and is the right
choice when the title would put a customer name, email, or secret into the
directory name and git history, where it cannot be taken back. Titles that
yield nothing usable fall back to random on their own.

Body cross-references use the `@<slug>` form. The `epic:` and `related:`
frontmatter fields hold bare slugs or `@<slug>` strings.

Active and closed issues live at `issues/<slug>/item.md`. Closing changes
status and metadata only; nothing moves. Cold storage is a separate
`issuectl archive` step that relocates old closed issues to
`issues/archive/YYYY/MM/<slug>/item.md`; discovery is archive-aware, so
`show`, `ls`, and queries find them anyway, and reopening an archived issue
moves it back. Never `git mv` an issue directory: the layout is the CLI's,
and doctor will flag what a manual move leaves dangling. When you need a
filesystem path, take `.data.path` or `.data.dir` from the command that
created or showed the issue rather than reconstructing it.

## Finding issues

Use the CLI rather than grepping the directory; it knows the schema and the
archive.

- `issuectl --json ls` lists open issues. Filter with flags (`-t/--type`,
  `-p/--priority`, `-s/--status`, `-a/--assignee` which matches `owner` for
  epics, `-l/--label`, `-e/--epic <slug>` for an epic's children) or with a
  positional query string.
- Vocabulary: types are `bug`, `task`, `feature`, `improvement`, `chore`,
  `epic` (a repo schema may add more). Priorities are `low`, `normal`, `high`.
  Active statuses are `open`, `in-progress`, `testing`, plus the intake states
  `untriaged`, `needs-info`, `deferred`. Closing statuses are `done`, `fixed`,
  `wontfix`, `duplicate`, `cannot-reproduce`, `obsolete`.
- Query syntax is shared by `ls`, `search`, and the web `?q=` filter. Fields:
  `status`, `type`, `priority`, `assignee`, `owner`, `reviewer`,
  `review_status`, `epic`, `label`, `slug`, `folder`, `blocked_by`, `blocks`,
  `updated`, `created`, `closed`, `text`; a bareword is `text:`.
  `-label:wontfix` negates; `assignee:none` / `assignee:any` test absence and
  presence; dates take relative offsets anchored on today in local time
  (`updated:<-14d`, `<=`, `>`, `>=`; `<=0d` means today or earlier, and avoid
  `+0d` in URLs where `+` decodes to a space). Terms AND together; there is no
  OR or grouping. Quote multi-word values (`text:"phrase to match"`); inside
  quotes only `\\` and `\"` are escapes, so paths and regex fragments survive.
  Outside quotes, `\:`, `\\`, `\ `, `\"`, and a leading `\-` escape the
  respective characters. Pass a leading-hyphen negation as one quoted argument.
- **Scope.** Bare `ls` and `search` are open-only. Any positional query to `ls`,
  or a positive `status:`/`folder:` term, or a pinned `-s/--status`, lifts that
  default so `ls -s fixed` finds closed and archived issues. `--all`
  (everything) and `--closed` (closed only) stay authoritative when given, so
  `ls --closed -s done` stays in the closed set. `search` widens only for
  `--all` or a positive `status:`/`folder:` term; a bareword or a negated
  status alone leaves it open-only. When the user says "all issues", "closed
  issues", or asks for the history of something, they want the wider scope.
- `issuectl --json show <slug>` for one issue. `issuectl --json epic tree
  [<slug>]` renders an epic with its children as nested
  `{slug,title,status,priority,type,children}` nodes, or every top-level epic
  when the slug is omitted. `issuectl --json stats` for counts.
- `issuectl --json duplicates [<slug>] [--threshold 0.30] [--all]` (alias
  `dups`) scores likely duplicates by title, label, and body-token overlap,
  locally, with no remote AI. All-pairs rows are
  `{a_slug,a_title,b_slug,b_title,score,…}`; single-slug rows are
  `{slug,title,score,…}`. Highest score first.

Process the JSON with `jq` and show the user a compact list
(`@<slug> — Title (type, status, assignee)`), not raw JSON.

## Creating

Gather what the issue needs. Type is required and has no default; infer it
("X is broken" is a bug, "we need Y" is a feature or task, "set up Z" is a
chore) and ask only when inference fails. Bugs want reproduction steps.
Reporter is usually `whoami` or the user. Check `issuectl --json ls -t epic`
when the work plausibly belongs to an existing epic, and suggest an epic
instead when the user describes a multi-week, several-task initiative.
Epics take `--owner` rather than `--reporter`/`--assignee`.

```
issuectl --json create \
    --type bug \
    --title "Login redirect loops on safari" \
    --reporter alice \
    --assignee bob \
    --priority normal \
    --source "frontend/login" \
    --description "Users get stuck in a 302 loop after SSO redirect."
```

The title may be positional instead (`create "Login redirect loops" --type bug`);
exactly one of the two forms. `new` is an alias for `create` and `--body` for
`--description`. `--body-file <path>` supplies the body from a file (`-` for
stdin, `./-` for a file literally named `-`) and excludes `--description`.
Other flags: `--epic <slug>`, `--label` and `--related "@<slug>"`
(repeatable), `--field key=value` for custom frontmatter declared in the
schema, the scheduling flags `--lane`, `--lane-seq`, `--add-collision`
(see Updating), and `--check-duplicates`, which refuses with exit 2 and a
`duplicate-precheck` envelope listing `matches` when a strong duplicate
exists; re-run without it to create anyway.

Relevant `.data` for a plain create; parse `.data.slug` and `.data.path`, and
do not reconstruct the path from the slug:

```json
{ "slug": "login-redirect-loops",
  "title": "Login redirect loops on safari",
  "path": "/abs/path/issues/login-redirect-loops/item.md",
  "dir": "/abs/path/issues/login-redirect-loops" }
```

**The body.** Without `--body-file` the CLI writes `# Title`, an optional
`_Source: …_` line, and `## Description`. With `--body-file` it places your
structured markdown under that preamble with no wrapper. Either way the repo
schema may append stubs for required H2 sections. The CLI does not otherwise
manage body sections, so add what the type needs by editing the file (use
`.data.path` for the file or `.data.dir` for its directory): `## Reproduction`
and `## Quick Test` for bugs; `## Goal`, `## Issues`, `## Phases`, and
`## Comments` for epics. The `## Issues` list in an epic is body text that
humans and `issuectl context` read; when you add a child to an epic or close
one, bring that list up to date, because nothing else will.

**Images** belong in the issue's `attachments/` directory as AVIF; doctor
warns about non-AVIF rasters and files over 1 MiB. `issuectl attach <slug>
<files…>` copies them in (colliding names get a numeric suffix); reference
them from the body with relative paths.

**Reception items are not `create`.** A bug report or feature request arriving
from a user or channel goes through `issuectl intake file`, which lands it in
`untriaged` with provenance and an idempotency key. `create` fixes the status
at `open`, which skips triage. See Intake below.

## Updating

`issuectl --json update <slug> …` changes frontmatter and bumps `updated:`.
The flags mirror the fields (`--status`, `-t/--type`, `--assignee` /
`--no-assignee`, `--owner` / `--no-owner`, `--no-reporter`, `--priority`,
`--epic` / `--no-epic`, `--add-label` / `--remove-label`, `--add-related` /
`--remove-related`, `--add-blocked-by` / `--remove-blocked-by`,
`--add-commit HASH:summary`, `--field` / `--clear-field`, `--title`,
`--body-file`); `--help --json` has the full list. Things the flag names do
not tell you:

- A closing status via `update --status` records the same close metadata as
  `close`. Moving from a closing status back to an active one is the reopen
  path: it clears `closed:` and `closed_by:`, appends a `## Reopen Notes —
  <today>` section in the same write, and reports `moved_to_open`.
- Converting to `epic` migrates a lone `reporter:` to `owner:` with a warning,
  but an assignee or conflicting owner is an error that tells you which
  `--no-…` flag to add. A type change that leaves required body sections
  missing is a schema violation listing the `## <Section>` headings to add
  first.
- `--title` rewrites the body's `# <title>` H1 and echoes the persisted
  `title`. `update <slug> --body-file <path>` and `issuectl body set <slug>
  --from-file <path>` replace the whole body; when the incoming text has no
  H1 the existing title is preserved and a warning says so, and a different
  incoming H1 is accepted with a warning. Use `--title` when a retitle is
  intended.
- `--add-blocked-by "@<slug>"` (bare slug accepted) is the same edge as
  `issuectl depend add <slug> --blocked-by <other>`: this issue is blocked
  by the other.
- Scheduling fields `--lane NAME` / `--no-lane`, `--lane-seq <int>` /
  `--no-lane-seq`, `--add-collision TOKEN` / `--remove-collision TOKEN` drive
  `issuectl dag`. A lane is a serial queue and only its head can spawn, so
  draw lanes at independently mergeable conflict boundaries rather than
  themes; the number of lanes is the parallelism budget. Shared hot files
  across lanes go in `collision:` tokens rather than merging lanes.
  `lane_seq` orders within a lane after `blocked_by` and priority.
  `--lane unlaned` means confirmed parallel-safe (every member spawnable),
  which differs from no lane (unclassified). `issuectl --json dag
  [--reservations <file|-|json>]` renders lanes, depth, heads, and
  `spawnable_heads`. Full reasoning: `docs/design/lane-design.md`.

Among scheduling and dependency fields, `update` conditionally echoes only
`lane`, `lane_seq`, and `collision`: when the call requests one of them,
`.data` carries its persisted post-update value even for a no-op, with
cleared `lane`/`lane_seq` as `null` and `collision` as the resulting list or
`null` when empty. An absent key means this call did not touch that field,
so test with `has("lane")` before reading. `blocked_by` is not an update
echo: read the canonical `@`-prefixed list from `issuectl --json show <slug>`
at `.data.blocked_by`, or use `issuectl --json dag`, where each row carries a
canonical bare-slug `blocked_by` list.

Bulk and transactional forms take the place of the positional slug; supply
exactly one target:

- `update --query "status:open label:stale" --priority high --add-label triaged`
  applies the same flags to every match under one lock; `--dry-run` gives
  per-issue diffs without writing. Results use `{dry_run, count, results[]}`.
- `update --patch-file <patch.yaml|->` applies a YAML/JSON patch in one
  transaction: `slug:`, any built-in fields, `custom_fields:`, label/related
  list ops, commits, and ordered `body_ops:` (`set_checkbox` with `match` and
  `checked`, idempotent; `append_note` with `author`, `message`, and
  `section` of `comments`, `decisions`, or `agent_runs`). It rolls back
  cleanly on a schema violation or a failed body op. Patch inputs cannot be
  combined with field flags, and inline JSON on argv is not accepted; pipe to
  `-` instead. `issuectl apply <patch>` is the older spelling of the same
  thing.

Mutation results share one `.data` shape: `slug`, `dir`, `version`, `status`,
`priority`, `labels`, `moved_to_closed`, `moved_to_open`, plus whatever the
call requested. With `--dry-run` they add `dry_run: true` and `diff` and
touch nothing.

### Focused verbs

- `issuectl set <slug> <field> <value>` sets one frontmatter field:
  `status`, `priority`, `assignee`, `owner`, `epic` typed; schema-declared
  custom keys through the validated custom-fields slot; `--clear` removes a
  non-status field. Reserved keys (`labels`, `related`, `type`, `title`, the
  scheduling fields) point you to their dedicated flags.
- `issuectl assign <slug> <user>` is `set … assignee`; `--clear` unassigns.
- `issuectl label <slug> add|remove <label>` (or `--add`/`--remove`) is
  idempotent on the label list, though `updated:` still bumps.
- `issuectl check <slug> "<task substring>"` toggles a `- [ ]`/`- [x]` line
  and errors unless exactly one checkbox line matches.
- `issuectl note <slug> --as <user> "<message>"` appends a timestamped block
  to `## Comments` (`--decision` for `## Decisions`, `--agent-run` for
  `## Agent Runs`), creating the section if needed. `comment` is an alias.
  The text comes from exactly one source: the positional argument,
  `--message`/`--body`/`--comment`, `--body-file`/`--from-file PATH` (`-`
  reads stdin), or `--stdin`; two is a usage error, none is an error. The
  author is a single token without whitespace; a leading `@` is stripped (the
  heading adds the sigil). Transition-rule mismatches found by `note` and
  `check` are warnings and the write goes through; `apply` and `update` keep
  them as errors so they can be fixed in the same transaction. The generated
  block:

  ```
  ### 2026-05-07T12:00:00Z · @alice

  <message>
  ```

All of these take `--dry-run` and `--expected-version`.

## Commits

Prefer trailers to manual `--add-commit`. End the commit message with
`Refs-Issue: @<slug>`, or `Fixes-Issue: @<slug>` when the commit is the fix,
and run `issuectl sync-commits` to append matching commits to each issue's
`commits[]`. It is idempotent; `--dry-run` previews and `--no-branch-fallback`
disables the "branch named after a slug" attribution. Its default range is
`<merge-base(HEAD, main/master)>..HEAD`, which on `main` itself is empty, so a
bare run right after committing on `main` records nothing and says so in
`warnings`. Pass `--range HEAD~1..HEAD`, or `--range origin/main..HEAD`
before pushing, to record the commits you mean.

`issuectl changelog` builds release notes from those trailers. `close --stamp`
(below) adds the `Fixes-Issue:` trailer for you.

## Closing

`issuectl --json close <slug>` sets a closing status and records
`closed:`. The default is `fixed` for bugs and `done` for everything else,
and the transition rules hold that line: a bug refuses `done`. Choose from
the user's words otherwise, with `--status`:

- `done` for finished tasks, features, chores, epics; `fixed` for a verified
  bug fix.
- `wontfix` for a decision not to do it; `duplicate` when another issue
  covers it (link it first with `update --add-related "@<slug>"`);
  `cannot-reproduce`; `obsolete` when it no longer matters.

Options: `--as <user>` records `closed_by:` (same author grammar as `note`);
`--comment "resolution"` (aliases `--note`, `--message`) appends the
rationale; `--commit HASH:summary` records commits, repeatable.

`--stamp` rewrites HEAD's commit message to append `Fixes-Issue: @<slug>`,
so the changelog picks up the landing commit without trailer discipline.
Ordering matters: run it after committing the fix (it stamps whatever HEAD
is) and before pushing or merging, since rewriting changes HEAD's sha. Only
the message changes; tree, author, dates, and index are untouched. It never
blocks the close: `.data.stamp` reports `{"status":"stamped","sha","previous_sha"}`,
`{"status":"already_present","sha"}`, or `{"status":"skipped","reason"}`
for a detached HEAD, a merge or signed commit, an in-progress
rebase/cherry-pick/merge/revert, or an empty repo. It cannot be combined with
a `--commit` that names HEAD, because the rewrite would orphan that sha.

**Definition of Done.** A delivery close (`done` and `fixed` by default)
expects `## Acceptance Criteria` with at least one checked item. By default a
shortfall is a warning; with `dod.strict: true` in `issues/.schema.yaml` it
blocks. Non-delivery dispositions are never gated. A project can name its own
delivery statuses under `dod.delivery_statuses` (the list replaces the
default, so restate `done` and `fixed`). `issuectl ready <slug>` reports the
same checklist read-only.

The result carries `moved_to_closed: true` and, with `--as`, `closed_by`.
`moved_to_closed` is a legacy-named lifecycle-transition indicator: it means
the issue entered a closing status, not that a directory moved. Reopening
similarly reports `moved_to_open`; only reopening an archived issue moves
files.

If you close an epic, or an issue that belongs to one, update the epic's
`## Issues` list to show the final statuses. Confirm with slug, title, and
closing status. Several slugs means several `close` calls; confirm each.

## Intake

`issuectl intake` is the flow for filing and triaging incoming bug reports and
feature requests. Intake state lives in `status`, not labels. Design and
rationale: `docs/design/intake-flow.md`. Two roles share it: a reporting agent
files, a developer or product manager dispositions.

Three active statuses: `untriaged` (filed, awaiting a decision), `needs-info`
(un-actionable pending reporter input), `deferred` (worthwhile, intentionally
not scheduled). They show up in `ls` like any active issue. The fields:
`provenance` (chat, email, github, …; open-valued unless the schema declares
an enum), `provenance_detail`, `source_ref` (the external message id and the
idempotency key), `disposition_reason` (`by-design`, `out-of-scope`,
`wontfix`, `withdrawn`, `superseded`), `disposition_note`, `duplicate_of`,
`deferred_until`, `superseded_by`.

**Filing.** `issuectl --json intake file --type bug --title "…" --body-file
report.md --reporter alice --provenance chat --source-ref "chat:123/message:456"`
lands the item in `untriaged`; the filer never names the entry state. Any
non-epic type is accepted. Filing is idempotent on `(provenance, source_ref)`:
a retry returns the existing item with `"deduplicated": true` and exit 0
rather than a second issue, which is what makes retrying a webhook safe.
Protected keys (`status`, `type`, `reporter`, `provenance`, …) are rejected
via `--field`. Slugs are random by default here, since report titles are
untrusted; `--slug` still overrides.
`intake withdraw <slug> --reason "…"` retracts an untriaged report as
`wontfix` with `disposition_reason: withdrawn`; by convention the reporter
does this, though the CLI does not check identity. The `/issue-new` skill
wraps this path, with `issuectl attach` for screenshots.

**The queue.** `issuectl --json intake queue` returns `{items: […], state}`,
the actionable `untriaged` set oldest first, every type and provenance.
`--state deferred|needs-info` shows the parked sets; `--type` and
`--provenance` filter; `--needs-analysis` keeps only items without a
`## Triage analysis` body section (each row's `needs_analysis` is derived
from that section; there is no stored analysis state). `issuectl --json
intake show <slug>` adds `attachments` (names) and `analysis` (the section
text or `null`). Legacy rows with a single exact `via:<channel>` label are
projected as that provenance; ambiguous ones stay `null` and `intake migrate`
reports them.

**Dispositions**, each a checked transition returning
`{slug, status, dir, version}`:

```
issuectl --json intake accept    <slug> [--assignee <who>] [--priority low|normal|high]  # → open
issuectl --json intake defer     <slug> --reason "…" [--until <date>]                     # → deferred
issuectl --json intake need-info <slug> --reason "…"                                      # → needs-info
issuectl --json intake reject    <slug> --reason "…" [--kind by-design|wontfix|out-of-scope]  # → wontfix
issuectl --json intake cannot-reproduce <slug> --reason "…"                               # bug-only
issuectl --json intake duplicate <slug> --of <canonical-slug>                             # → duplicate
issuectl --json intake obsolete  <slug> --reason "…" [--superseded-by <slug>]             # → obsolete
issuectl --json intake retype    <slug> --to <type>
issuectl --json intake reopen    <slug> [--to untriaged|open] --reason "…"                # closing → active
```

`--reason` is required wherever it appears, so the why is captured in a
field rather than lost in prose. `reject --kind` defaults to `wontfix`;
`reopen --to` defaults to `untriaged`. The command is `need-info`; the status
is `needs-info`. Illegal source states (accepting a closed item, say) come
back as `transition-illegal`; other stable codes are `duplicate-source-ref`
and `protected-field`.

The `/issue-intake` skill drives the processing side: it reads the queue,
sends unclear bug items to the bug-only `/worktree-bug-analysis` worker, and
briefs the user, who owns the disposition. Unclear non-bugs are not sent to
that worker.

## Other read-only tools

- `issuectl --json context <slug>` renders a byte-deterministic bundle of the
  issue, its parent epic, blockers, related issues, body sections, commits,
  and schema rules; `--write` caches it under `.issuectl/cache/agent/<slug>/`.
  It never writes under `issues/`. `.data.issue.version` is the same token
  `show` returns, so it can feed `--expected-version` directly.
- `issuectl prompt <template> <slug>` renders `.issuectl/prompts/<template>.md`
  against that bundle with `{{key}}` substitution: `{{slug}}`, `{{title}}`,
  `{{body}}`, `{{version}}`, `{{epic_goal}}`, `{{related}}`, `{{commits}}`,
  `{{context}}`, and any body H2 by snake-cased name (`## Test Plan` →
  `{{test_plan}}`). Unknown keys stay in place so typos show. Template names
  are plain filenames.
- `issuectl --json scan-todos` classifies `TODO(issue: <slug>)` markers as
  `tracked`, `stale`, `unknown`, or `untracked`. `--file-intake` files each
  untracked marker through intake with provenance `scan-todos`, returning
  `{hits, filings}`; the source identity is content-derived, so a marker that
  merely moves lines deduplicates while a changed path or text files anew.
- `issuectl --json config show` reports every effective schema value with
  `source: "file"` or `"default"` under `.data.values`; `config path` gives
  the schema location at `.data.path`. Useful before writing from an
  unfamiliar repo.
- `issuectl --json skill list` describes the bundled `/issue`, `/issue-new`,
  and `/issue-intake` skills and the install contract (`supported_agents`,
  `install`, `skills`). `skill install` defaults to every skill for Claude,
  pi, and Codex; `--agent`, `--target`, `--dry-run`, and `--force` narrow or
  confirm it. `skill pi-status` reports only legacy global pi copies.

## Doctor

`issuectl --json doctor` is the read-only health check; `--fix` applies
migrations and repairs. It migrates legacy `<NN>-<slug>/` directories to the
flat layout, rewrites `number:` to `slug:` and `#NN` body refs to `@<slug>`,
promotes stranded `issues/inbox/` drafts, and flags invalid slugs,
duplicates, missing `item.md`, orphan epic refs, broken attachment refs,
self-dependencies in `blocked_by:`, and residual uses of the retired
`deferred` label (`--fix` removes the label without touching the valid
status of the same name; when the label still encodes a pending legacy
migration, `deferred_labels_require_intake_migrate` says so, and `issuectl
intake migrate --apply` comes first).

`--fix` is forward-progress only. On a clean run, `.data.apply_outcome`
has `stop_phase: "ok"` and empty `blockers`. When it stops, the command exits
1 with an error envelope whose code names the phase: `doctor-blocked`
(preflight refused, nothing written), `doctor-partial` (some phases wrote
before a later safety check found a new blocker), or `doctor-apply-error`;
the full report sits under `error.details`. In the partial case the writes
that landed are valid migrations, so do not try to roll them back; resolve
the listed blockers and run `--fix` again.

## Notes

- The CLI sets `created`, `updated`, and `closed` itself.
- Write issue content in English; Finnish text is fine in the body.
- Default priority is `normal`; default status is `open` except through
  `intake file`, which files into `untriaged`.
- Epic membership is the child's `epic:` field holding the parent's slug.
