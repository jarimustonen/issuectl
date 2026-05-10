# Custom boards — user-defined views with configurable column grouping

## Status

Accepted; landing in this branch.

## Motivation

The existing kanban (`server/`) groups on a fixed `status` axis. Triage
workflows want to bucket open issues by *something else* (target release
epic, label, priority, custom field) and to **drag** between buckets
instead of editing each `item.md` by hand.

The shape of the change is "add a configurable column axis". The build
already has every primitive needed:

- **Filter:** `crate::query` (used by `ls` / `search` / `?q=`).
- **Write path:** `mutate::update_issue` under flock + `expected_version`
  with full custom-field support.
- **Schema:** `.schema.yaml` declares which fields exist and which are
  enum-constrained.

The work is to plumb a board definition through the server and JS so
that columns and the drag PATCH target a configurable group_by field.

## Non-goals

- Cross-cutting board widgets (swim lanes, WIP limits, multi-axis).
- A new query language. Filters reuse `crate::query` syntax verbatim.
- A board editor in the UI; v1 boards are YAML files committed to
  `.issuectl/boards/`.
- Migrating existing repos. Repos without `.issuectl/boards/` see only
  the default status board at `/`.

## Definition file

```yaml
# .issuectl/boards/triage.yaml
name: triage
group_by: epic
columns:
  - value: ""                # empty bucket — issues with no epic
    label: "Unscoped"
  - value: hugely-exciting-spiders
    label: "v0.6 candidates"
  - value: future
    label: "Future"
filter: "type:bug status:open"   # optional; query-engine syntax
```

Fields:

- `name` — must equal the file's basename (sanity check; otherwise
  rejected at load).
- `group_by` — built-in scalar field (`epic`, `assignee`, `owner`,
  `priority`, `type`) or a custom **scalar** field declared in
  `.schema.yaml`. List-typed fields (`labels`, `related`) are
  intentionally rejected in v1; see *Open questions* below.
- `columns` — explicit ordered list. Values not listed do not appear on
  the board (no implicit "Other" bucket in v1; the rationale matches
  the brief — keeps unbounded value spaces like `assignee` from filling
  the screen with one column per name). The empty-string value is the
  unassigned bucket (issue's frontmatter is missing or null).
- `filter` — optional `crate::query` string. Reused, no DSL.

The loader rejects:

- Unknown `group_by` (not built-in, not in `.schema.yaml`).
- List-typed `group_by` (v1 scope).
- Empty / missing `columns`.
- Duplicate `value` entries within `columns`.
- Filter that doesn't parse.

Validation failures of `group_by` and `filter` are reported separately so
the runtime can choose between "404 / refuse to render" vs. "render
read-only with banner" — see *Read-only fallbacks*.

## URL & routing

- `/` — default status board (unchanged).
- `/board/<name>` — custom board.
- `GET /api/boards` — `[ "triage", "release", ... ]` (sorted).
- `GET /api/boards/<name>` — one envelope:

  ```jsonc
  {
    "name": "triage",
    "group_by": "epic",
    "columns": [ { "value": "", "label": "Unscoped" }, ... ],
    "filter": "type:bug status:open",
    "issues": [
      { ...IssueSummary fields...,
        "group_value": "hugely-exciting-spiders" }
    ],
    "warnings": [],
    "snapshot_seq": 42,
    "instance_id": "...",
    "read_only": false,
    "read_only_reason": null
  }
  ```

Resolving `group_value` server-side keeps the JS dumb: it does not need
to know which fields are scalar built-ins vs. custom frontmatter, and
the API response shape doesn't depend on the group_by axis. Loaded
issues use the existing summary path; for custom group_by fields the
loader reads `extra` from the parsed `Issue` and stringifies it.

## Drag semantics

Drop a card into column X writes `group_by = X.value` via the existing
`PATCH /api/issues/<slug>` endpoint:

| group_by              | PATCH body                                       |
|-----------------------|--------------------------------------------------|
| `epic`/`assignee`/`owner`/`priority`/`type` | `{ "<field>": "X.value" }` (dedicated slot) |
| custom scalar         | `{ "custom_fields": { "<field>": "X.value" } }` |
| empty-bucket drop     | the same field set to `null` (clear semantics)  |

In all cases the body carries `expected_version`, just like the status
drag path; the 409 / 429 / generic-error toasts reuse the same JS
helpers.

The closing-status picker modal does **not** apply to custom boards —
no column collapses multiple distinct values. A single drop maps
unambiguously to one PATCH.

## Read-only fallbacks

The board renders read-only with a banner — the cards still show, drag
is disabled — when:

- `group_by` field is missing from `.schema.yaml` and is not a built-in
  scalar (typo, schema regression).
- `filter:` is set but does not parse.

Hard errors (file unreadable, malformed YAML, duplicate column values,
list-typed group_by) return 404 from `/api/boards/<name>` rather than
rendering, because the user has no chance of forming a coherent mental
model from a half-broken board. The loader returns a typed
`BoardError`; the route maps `Validation` → 404, `Soft` (missing field
or bad filter) → 200 with `read_only=true`.

## Enumeration vs. open-set columns

For `epic`/custom-scalar the user explicitly enumerates columns —
unlisted values vanish (intentional, matches brief). v2 may add
`include_unlisted: true` for assignee-style boards.

## Backwards compatibility

No migration. Repos without `.issuectl/boards/` see no `/board/<name>`
routes (404), and the existing `/` board is untouched.

## File layout & loader sketch

New module `crate::boards`:

```rust
pub struct Board {
    pub name: String,
    pub group_by: String,
    pub columns: Vec<BoardColumn>,
    pub filter: Option<String>,
}

pub struct BoardColumn { pub value: String, pub label: String }

pub enum BoardError {
    NotFound,
    Io(anyhow::Error),
    Validation(String),  // → 404
    Soft(String),        // → 200 read_only=true
}

pub fn load(root: &Path, name: &str) -> Result<Board, BoardError>;
pub fn list(root: &Path) -> Vec<String>;
```

The loader is **stateless** (re-reads on every request). Schema lookups
go through the existing `RepoConfigCache` so the per-process snapshot is
shared with the mutation layer. Cache invalidation is the cache's
problem, not boards'.

## Open questions / deferred

- **List-typed group_by (`labels`).** Drag onto a label column would
  need add/remove semantics across columns, and a card could be in
  multiple columns simultaneously. Defer until there's a concrete
  workflow ask; the brief explicitly defers an "other" bucket too,
  which sits in the same shape.
- **Per-column WIP limits.** Trivial to add later; not needed for the
  triage use case.
- **In-UI board editor.** Out of scope; v1 is YAML.
