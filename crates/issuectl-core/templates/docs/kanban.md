# Kanban / web board (`issuectl serve`)

Local web UI for the current repo's `issues/` directory. Renders
issues as a Trello-style board with drag-and-drop, plus a detail view
per issue with the rendered markdown body and any sibling `*.md` docs.

Two board flavors:

- **Default status board** at `/` — columns are fixed (Open / In
  progress / Testing / Closed). Drag updates `status:`.
- **Custom boards** at `/board/<name>` — columns and group_by axis
  defined per-board in `.issuectl/boards/<name>.yaml`. Drag updates
  whichever field the board groups on (epic, priority, custom scalar,
  …).

## Run it

```
issuectl serve                       # http://127.0.0.1:7878
issuectl serve --port 8080
issuectl serve --host 0.0.0.0        # bind to all interfaces (see warning)
issuectl serve --no-watch            # skip filesystem watcher
issuectl serve --watch-poll-ms 1000  # polling backend (network FS)
```

The server reads the filesystem on every request — there is no cache,
so external edits show up on the next page load. Live updates flow
via SSE (`/events`) when the watcher is enabled.

## Writes (drag-and-drop, body editor)

Writes are enabled on loopback by default and require:

- A valid `X-Issuectl-CSRF` token (bootstrap via `GET /api/session`).
- A `Host` header matching the bound address (DNS-rebinding defense).
- An `expected_version` token for optimistic concurrency.

Non-loopback binds are read-only unless you pass
`--allow-remote-writes`. Writes go through the same `mutate.rs` path
the CLI uses, under a repo-wide flock.

## Custom boards (`/board/<name>`)

A custom board lives at `.issuectl/boards/<name>.yaml`:

```yaml
name: triage
group_by: epic                       # built-in scalar OR custom scalar
                                     # from .schema.yaml
columns:
  - value: ""                        # empty bucket (clear semantics)
    label: Unscoped
  - value: hugely-exciting-spiders
    label: "v0.6 candidates"
  - value: exorbitantly-ill-apples
    label: "v0.5.0"
filter: "status:open"                # optional; query-engine syntax
filters: [search, type]              # optional; client filter-bar fields
```

- `name` must equal the basename.
- `group_by` accepts built-in scalars (`epic`, `assignee`, `owner`,
  `priority`, `type`, `reporter`, `status`) or any custom **scalar**
  field declared in `.schema.yaml`. List-typed fields (`labels`,
  `related`) are rejected.
- `columns` is an explicit ordered list. Values not listed do not
  appear on the board (no implicit "Other" bucket). The empty value
  is the unassigned bucket — clearing semantics on drop.
- For required built-ins (`priority`, `type`, `status`) the empty
  bucket is rejected — a drop there would 422 every time.
- Column values for enum-constrained fields (e.g. `priority` →
  `low/normal/high/critical`) must be in the schema's enum.
- `filter:` reuses `crate::query` syntax (same as `ls` / `search` /
  `?q=`). Unparseable filters are hard-rejected at load time.
- `filters:` opts in a subset of `[search, type, assignee, epic,
  label]` to render in the client filter bar. Default: hide the
  bar entirely (the board's own `filter:` already scopes the
  dataset).

Drag semantics: dropping a card into column X PATCHes `group_by = X.value`
through the existing `/api/issues/<slug>` endpoint. Built-in fields
ride dedicated `UpdateIssueRequest` slots; custom fields route through
`custom_fields`. The empty-string bucket clears the field (`null`).

### Read-only fallback

If the board's `group_by` field is missing from `.schema.yaml` (typo,
schema regression), the board renders read-only with a banner — drag
disabled, cards still visible. This is the only soft-error path; YAML
validation failures (filter parse, list-typed group_by, enum mismatch,
…) return 404 from `/board/<name>` and 422 from `/api/boards/<name>`
so the agent author can correct the file.

## Routes

| Path | Purpose |
| ---- | ------- |
| `GET /` | Status board HTML shell |
| `GET /board/{name}` | Custom board HTML shell |
| `GET /issue/{slug}` | Issue detail HTML |
| `GET /api/session` | CSRF token, instance id, watcher state |
| `GET /api/issues` | All issues + parse warnings + snapshot seq |
| `GET /api/issues/{slug}` | One issue with `body_html`, `version`, `docs[]` |
| `GET /api/issues/{slug}/docs/{name}` | Sibling `*.md` doc, rendered |
| `PATCH /api/issues/{slug}` | Frontmatter PATCH (drag drops, custom_fields) |
| `PUT /api/issues/{slug}/body` | Replace markdown body (`expected_version`) |
| `POST /api/issues` | Create new issue |
| `POST /api/preview` | Render body markdown to sanitized HTML |
| `GET /api/boards` | List configured custom boards (sorted) |
| `GET /api/boards/{name}` | Board metadata + issues with `group_value` |
| `GET /events` | Server-sent events stream (SSE) |
| `GET /assets/{board.css,board.js,theme-*.js}` | Bundled static assets |

## Security model

- Defaults to `127.0.0.1` (loopback only). Non-loopback binds are
  read-only unless explicitly opted in.
- CSP, X-Content-Type-Options, Referrer-Policy, X-Frame-Options on
  every response.
- Slugs and board names validated before any filesystem access;
  symlinks that escape an issue directory are rejected.
- Static asset bundle baked into the binary; no external CDNs.

## When to suggest it

If the user wants to **see** issues or move them around visually —
"show me the board", "open the kanban", "let me triage v0.6
candidates" — start the server and hand them the URL.

For triaging by something other than status (epic, target release,
custom field, …), write a `.issuectl/boards/<name>.yaml` and link
them to `/board/<name>`. The board YAML is committed alongside the
issues, so the configuration travels with the repo.
