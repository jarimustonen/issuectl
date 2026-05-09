# Kanban / web board (`issuectl serve`)

A local, read-only web UI for the current repo's `issues/` directory.
Renders open and closed issues as a Trello-style board, plus a detail
view per issue with the rendered markdown body and any sibling `*.md`
docs in the issue directory.

## Run it

```
issuectl serve                       # http://127.0.0.1:7878
issuectl serve --port 8080
issuectl serve --host 0.0.0.0        # bind to all interfaces (see warning)
```

The server reads the filesystem on every request — there is no cache,
so edits made via `issuectl new/update/close` (or by hand) show up on
the next page load.

## Scope and limitations

- **Read-only.** No POST/PATCH endpoints; the board cannot create,
  update, or close issues. Use the CLI / `/issue` skill for writes.
- **No authentication.** Defaults to `127.0.0.1` (loopback only). If
  you bind to a non-loopback address, the contents are reachable by
  anyone on that network — the server prints a warning when this
  happens. Use only on trusted networks.
- **Static asset bundle.** CSS/JS are served from `/assets/*` and are
  baked into the binary; no external CDNs.
- **Security headers** (CSP, X-Content-Type-Options, Referrer-Policy,
  X-Frame-Options) are set on every response.

## Routes

| Path | Purpose |
| ---- | ------- |
| `GET /` | Board HTML shell |
| `GET /issue/{slug}` | Issue detail HTML |
| `GET /api/issues` | All issues as JSON (with parse warnings) |
| `GET /api/issues/{slug}` | One issue with rendered `body_html` and `docs[]` |
| `GET /api/issues/{slug}/docs/{name}` | Sibling `*.md` doc, rendered |
| `GET /assets/{board.css,board.js,theme-*.js}` | Bundled static assets |

Slugs are validated against the canonical shape before any filesystem
access; symlinks that escape an issue directory are rejected.

## When to suggest it

If the user wants to **see** issues — "show me the board",
"open the kanban", "let me browse visually" — start the server and
hand them the URL. For everything else (creating, updating, closing,
querying) keep using the CLI.
