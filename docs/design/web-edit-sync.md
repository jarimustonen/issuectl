# Web ↔ File bidirectional sync — design

Status: **draft for review**. Implementation lives in a separate worktree; this
doc proposes ideas, not code.

## 1. Goals & non-goals

**Goals**

- Edit issues in the browser; changes land in `issues/<folder>/<slug>/item.md`.
- External edits (`$EDITOR`, `git pull`, `issuectl update`, agents) propagate
  live to every connected board.
- Filesystem stays the source of truth. The server is a renderer + a thin
  write proxy, not a database.
- Web actions remain expressible from the CLI. Anything the UI does must be
  reachable from `issuectl --json …` so AI agents and humans share the same
  surface.

**Non-goals (this round)**

- Multi-user authentication, RBAC, or audit logs.
- Operational-transform / CRDT collaborative editing.
- Cross-repo or remote (non-loopback) write access — explicitly out of scope.
- Persistent server-side undo stack; the filesystem + git is the undo log.

## 2. Component diagram

Textual sketch of the runtime (boxes are types/modules, arrows are messages):

```
┌──────────┐  HTTP+SSE/WS   ┌────────────────┐
│ browser  │ ◀────────────▶│ axum Router    │
└──────────┘                │  ├ GET html/api│
   ▲                        │  ├ PATCH issue│   in-memory
   │ DOM events             │  └ GET /events│   broadcast::Sender<Event>
   │                        └──────┬─────────┘ ─────────────┐
   │                               │ writes                  │ events
   │                               ▼                         ▼
   │                        ┌─────────────┐         ┌────────────────┐
   │                        │ write.rs +  │         │ watcher task   │
   │                        │ repo.rs     │         │  (notify crate)│
   │                        └──────┬──────┘         └──────┬─────────┘
   │                               │                       │
   │                               ▼                       │
   │                        ┌─────────────────────────────┐│
   └─────── re-render ◀───── │  filesystem: issues/**     ││
                            │  (single source of truth)   ││
                            └──────────────┬──────────────┘│
                                           │ inotify/FSEvents
                                           └────────────────┘
```

`AppState` (today: `{ root: Arc<PathBuf> }`) grows two things:

```rust
// illustrative, not final
struct AppState {
    root: Arc<PathBuf>,
    events: broadcast::Sender<BoardEvent>,    // fan-out to SSE clients
    own_writes: Arc<DashSet<WriteToken>>,     // echo suppression (§4)
}
```

Two new long-running tokio tasks alongside `axum::serve`:

1. `watcher_task` — owns a `notify::RecommendedWatcher` rooted at
   `<root>/issues/`, debounces events, looks up affected slugs, re-parses
   them, broadcasts `BoardEvent`.
2. `serve` (existing) — adds an `/events` endpoint that subscribes to the
   broadcast and streams to the client.

## 3. Web → files (write path)

### 3.1 API surface

Two viable shapes; I recommend **(A) per-field PATCH that maps 1:1 to the CLI**.

```
PATCH  /api/issues/{slug}      # field-level mutation, structured JSON body
PUT    /api/issues/{slug}/body # body markdown only (covered separately)
POST   /api/issues             # new issue (mirrors `issuectl new --json`)
DELETE /api/issues/{slug}      # not in v1 — closing is `update --status`
```

`PATCH` body mirrors the `Update` clap subcommand fields, so the server can
reuse existing validators:

```json
{
  "expected_version": "sha256:…",        // §4 concurrency token
  "status":   "in-progress",
  "assignee": "alice",
  "priority": "high",
  "epic":     null,                      // null ≡ --no-epic
  "add_labels":    ["frontend"],
  "remove_labels": ["legacy"],
  "add_related":    ["@amber-loud-fox"],
  "remove_related": []
}
```

Why per-field, not whole-document?

- The CLI is already field-shaped (`do_update` in `src/main.rs:828`).
- Whole-document PUT means the client has to reconstruct YAML, which puts
  YAML serialization in JS — a footgun for `'#7'`-style quoting and the
  flow-list normaliser in `write::flowify_string_arrays`.
- Smaller payloads, less to merge during a 409 conflict (§4).

The body is special-cased because typing into a textarea is a stream of
small edits that benefits from a different rate-limit and conflict policy
than metadata flips.

### 3.2 Server-internal call vs CLI shell-out

Two options:

| Option | Pros | Cons |
| --- | --- | --- |
| **A. Library call** — server invokes `do_update(root, args)` directly | Zero process overhead; shared validation; rich `Result` types | Small refactor: `do_*` lives in `main.rs`; promote to `lib.rs` or `mutate.rs` |
| **B. Shell out to `issuectl --json update`** | Maximum dogfooding; the web is provably "just a CLI client" | 30–80 ms fork+exec per keystroke-debounced write; locking races between the CLI and the server's own watcher; harder to thread through the echo-suppression token |

**Recommendation: A**, with a hard test that the JSON request body and the
clap `UpdateArgs` stay structurally equivalent. Dogfooding is preserved
because the server uses *the same Rust functions* that `cmd_update` uses —
the value of "shells out" is symbolic, the cost is real.

Refactor the four mutation entry points (`do_new`, `do_update`, `do_close`,
plus a future `do_set_body`) into `src/mutate.rs` exporting structured
results. The CLI continues to call them; the server starts to.

### 3.3 Atomicity

`write::write_item` today calls `fs::write(path, …)` (`src/write.rs:74`),
which is **not** atomic — a crash mid-write leaves a half-written `item.md`
that `parse_item_md` will warn on. For a future where the server writes
on every textarea blur, that's not acceptable.

Proposed write sequence (move into `write_item_atomic`):

1. Render full file content into a `String`.
2. `tempfile::NamedTempFile::new_in(item.parent())`.
3. Write bytes, `tempfile.as_file().sync_all()` (fsync).
4. `tempfile.persist(item_path)` (atomic rename within same fs).
5. Best-effort `fsync` of the parent directory on Unix to flush the rename
   to the directory inode.

The same-filesystem requirement is satisfied because the temp lives next to
the target. We add `tempfile` (already a dev-dep) as a runtime dep.

For the folder move on closing-status changes (`do_update` ≈ line 904), the
existing `fs::rename` is already atomic on the same filesystem. No change
needed beyond ordering: write the new content first, *then* rename the dir.

### 3.4 Validation

Validate at three rings, outermost first:

1. **HTTP layer**: clap-equivalent value parsers. Reject malformed slugs,
   bad enums (`status`, `priority`, `type`), bad `@slug` refs, with
   structured 400 errors. Reuses `slug::is_valid`, the closing-status
   table, etc.
2. **Mutate layer**: re-validate inside `do_update` because the CLI
   contract requires it (`AGENTS-AI-FIRST-CLI.md §1`). Web layer is *one*
   caller; the CLI must keep its own guarantees.
3. **Serialize layer**: `serde_yaml::to_string(&map)` round-trips a typed
   `Mapping`, so malformed YAML cannot leave the server. The body, treated
   as opaque markdown, is *not* HTML-sanitized at write-time — sanitation
   is render-time only (`render::sanitize_markdown` in `src/server/render.rs:167`).

Reject a write before the disk is touched if any ring fails. Return a
problem-detail JSON body (RFC 7807-shaped) with field-level errors so the
UI can highlight which field exploded.

## 4. Files → web (read path / push)

### 4.1 Watcher

Use the **`notify`** crate (well-trodden, abstracts inotify / FSEvents /
ReadDirectoryChangesW). Wrap with `notify-debouncer-mini` (or `-full`) so
we don't fan out 50 events for one editor save. Rationale:

- Editors (vim, VS Code) write via temp+rename; we'd see Create + Remove
  + Modify in milliseconds. Debouncing collapses these into one
  "something changed in `issues/open/foo/`" event.
- macOS FSEvents is coarser-grained than Linux inotify; debouncing makes
  the platform difference irrelevant.

Coalescing strategy: 100–200 ms debounce window keyed by issue *slug*
(directory two levels above the changed file). After the window closes:

1. Re-locate the issue (it may have moved open ↔ closed).
2. Re-parse with `parse_item_md_with_warnings`.
3. Emit `BoardEvent::IssueUpserted` or `IssueRemoved`.

Symlink hardening: configure the watcher with `Config::default()
.with_follow_symlinks(false)`. The watch root is the *canonicalized*
`<root>/issues/` so a path-replacement attack on `issues/` itself doesn't
silently start watching another tree.

### 4.2 Transport: SSE vs WebSocket vs long-poll

| Transport | Pros | Cons |
| --- | --- | --- |
| **Server-Sent Events** | One-way, fits the workload; built into axum (`axum::response::sse`); auto-reconnect with `Last-Event-ID`; works through every proxy that handles HTTP/1.1 keep-alive; no framing protocol to maintain | Browsers cap SSE at ~6 connections per origin (irrelevant for a single tab board) |
| **WebSocket** | Bidirectional, lower overhead at scale | We don't need bidirectional — writes are HTTP PATCH; adds `tokio-tungstenite` and a framing protocol; harder to debug in DevTools |
| **Long-poll** | Works everywhere | Higher latency, more wakeups, not nice to a watcher fan-out |

**Recommendation: SSE.** It's literally the shape of our data: the server
publishes timestamped events, the client consumes them in order, and
reconnect-with-resume is built into the standard.

### 4.3 Payload shape

Three viable shapes; combine them.

| Shape | When |
| --- | --- |
| `IssueUpserted { issue: IssueSummary, body_html: Option<String> }` | Default for one-issue changes — small, sufficient for board cards |
| `IssueRemoved { slug }` | Folder rename → other folder, or directory deleted |
| `Resync` (i.e. "re-fetch `/api/issues`") | Bulk events (git checkout, mass rename), watcher overflow, or after a missed-event detection |

Use full-issue payloads (not "invalidated, re-fetch a single id") because:

- The detail dialog already needs the issue body; if we already parsed
  it server-side we may as well ship it.
- Re-fetching one issue takes a round trip per change — irritating during
  a `git pull` that touches 20 files.
- Wire size is small (markdown bodies are short).

When the watcher reports overflow / drop (Linux inotify has a queue),
bypass the per-issue path and emit a `Resync` event; clients respond by
re-fetching `/api/issues`.

### 4.4 Echo suppression

The server is itself a writer. After a PATCH, the watcher will fire and
broadcast — without suppression, every connected client (including the
one that issued the PATCH) gets a redundant snapshot, causing UI flicker
and a brief "your draft just got overwritten" appearance.

Three options:

1. **Always re-broadcast.** Simple. Cost: every PATCH causes a round-trip
   echo on the same connection. UI must be idempotent against its own
   writes — but it has to be anyway, because two browser tabs editing
   the same issue will see each other's writes via the watcher.
2. **Write tokens.** Each PATCH generates a `WriteToken` (UUID) before
   touching disk; the server stashes `(token → slug, content_hash)` in
   `own_writes` for ~2 s; the watcher consults this set and tags
   broadcasts as `origin: "self"`. Clients that issued the PATCH see
   a 200 with the token and ignore the matching SSE.
3. **Hash-based suppression.** Compare the post-write hash to the
   broadcast hash and drop "self-equal" events. Brittle: a near-
   simultaneous external edit that happens to produce the same hash
   gets dropped (vanishingly unlikely but possible).

**Recommendation: 1 + 2 mixed.** Broadcast everything (option 1 is the
correctness baseline), and tag self-originated events with the write
token so the originating tab can show "saved" instead of a re-render
flash. Echo suppression is a UX nicety, not a correctness requirement.

### 4.5 Reconnect & catch-up

Each `BoardEvent` carries a monotonically increasing `seq: u64`. Server
keeps a ring buffer of, say, 256 recent events. When the client
reconnects with `Last-Event-ID: 42`, the server replays everything
`> 42`; if the gap is too large, it sends `Resync` and the client
re-fetches the listing.

On a brand-new connection (no `Last-Event-ID`), the server *only* opens
the stream — it does **not** push a startup snapshot. The page already
fetched `/api/issues` on load; mixing snapshot+stream creates ordering
ambiguity. Initial state via REST, deltas via SSE.

## 5. Concurrency & conflicts

### 5.1 Optimistic concurrency token

Each `IssueSummary` / detail response gains a `version: String` field.
Three viable sources:

| Source | Pros | Cons |
| --- | --- | --- |
| **`mtime`** | Free | 1 s precision on some FSes; not stable across `git checkout` (mtime resets); `cp -p` lies |
| **Frontmatter `version: <n>`** counter | Stable across copies | Pollutes the frontmatter with a synthetic field; conflicts with hand-edits that bump it manually; another field for `parser.rs` and migration to handle |
| **Content hash (sha256 of raw file bytes)** | Stable, deterministic, no extra disk state, survives `git checkout`, easy to compute | Constant cost per write/read (negligible at issue file sizes) |

**Recommendation: content hash.** Compute on read, return as
`version: "sha256:abcdef…"` (truncated to 16 hex chars in the wire
payload — the prefix is plenty for collision resistance against the
2–3 versions an issue sees per minute). A PATCH whose
`expected_version` doesn't match the on-disk hash returns 409 with the
fresh issue payload in the body so the client can three-way merge.

### 5.2 Client behaviour on 409

UI options, easiest first:

1. Toast `"Someone else changed this issue — reloaded"`, replace the
   form state with the server's fresh copy, ask the user to retry.
2. Show a diff of "yours / theirs / current" and a "keep mine /
   keep theirs" picker — only worth it for the body, not for
   single-field flips.
3. Field-level OT/CRDT — out of scope.

For v1: option 1 everywhere, plus an in-memory clipboard of the user's
last attempted edit so they can paste it back if surprised.

### 5.3 Bursts (typing in the body)

Rate-limit at three places:

- **Client** debounces textarea input ≈ 750 ms after last keystroke,
  *and* on blur, *and* on tab-hide.
- **Server** rate-limits PATCH per-slug per-IP via a token bucket
  (e.g. `tower_governor` or a hand-rolled `Mutex<HashMap<…>>`) — say
  4 writes/sec sustained, burst 10. Exceeding returns 429; client
  backs off.
- **Watcher** debounce already absorbs server-internal writes from
  feeding back into a notification storm.

The hard rule: every single PATCH still produces an atomic file
write. We don't batch on the server — that would mean dropping a write
on shutdown.

## 6. Edit granularity in the UI

Two distinct interactions; pick different controls.

### 6.1 Frontmatter / status

Inline controls per field:

- **Status**: drag card between columns (already a column-laid-out
  board, this is the cheapest interaction); also a `<select>` in the
  detail dialog.
- **Priority**, **type**: `<select>` with the same allowed values as
  the CLI (`PRIORITIES`, `ISSUE_TYPES` in `src/main.rs:19`).
- **Assignee / owner / reporter / epic**: free-text inputs with
  client-side validation against `slug::is_valid` patterns.
- **Labels**, **related**: chip inputs with add/remove.

Each interaction → one PATCH.

### 6.2 Body

Plain-text-first. Recommendation: **`<textarea>` with monospace font,
maybe a tiny preview pane**. Not CodeMirror, not Monaco. Reasons:

- The project ships zero JS dependencies (board.js is hand-written,
  CSP forbids inline scripts). A 4-MB Monaco bundle would dwarf the
  binary.
- Issue bodies are short (sub-1 kB). Syntax highlighting buys little.
- The reader experience already uses `pulldown-cmark` + `ammonia`;
  we can render a side-by-side preview by re-using the existing
  endpoint `POST /api/preview` (new) that returns sanitized HTML
  without writing.

If we later want CodeMirror 6, it's drop-in: replace the `<textarea>`
target. Defer.

## 7. Failure modes

### 7.1 Watcher misses events

Realistic risks:

- macOS FSEvents coalesces to per-directory granularity. A rapid
  rename → write within the debounce window can hide one event;
  re-parsing the directory fixes it (we do that anyway).
- Network filesystems (NFS, SMB): `notify` falls back to polling. The
  CLI will document `--watch-poll-ms` to force polling explicitly;
  `serve --no-watch` disables the watcher (read-only, manual refresh).
- Linux inotify queue overflow under bulk operations (`git checkout`
  switching branch with hundreds of issues): `notify` reports
  `Event::Other(EventKind::Other)` / overflow; the watcher task
  responds with a `Resync` broadcast.

### 7.2 Disk full / permission errors

The atomic write fails at step (4) — `persist` returns `Err`. Map to
HTTP 507 Insufficient Storage / 500 with a structured error body. The
on-disk state is unchanged because `persist` is what swaps the file.
The temp file is auto-cleaned by `tempfile`'s `Drop`.

### 7.3 Disconnected clients

Per §4.5: monotonic `seq` + ring buffer + `Resync` fallback.
`EventSource` auto-reconnects; the `Last-Event-ID` header carries the
sequence number; the server keeps the buffer in `AppState`.

### 7.4 Crash mid-write

Already handled by atomic write. Reader sees either pre-image or
post-image, never a half-file.

### 7.5 Watcher itself crashes

The watcher task panics → its `JoinHandle` resolves → server logs and
either (a) restarts it once or (b) demotes to read-only. Recommend
(a) with exponential backoff capped at 3 retries; after that, broadcast
a structured `Degraded { reason: "watcher_unavailable" }` event so the
UI can show a red-dot indicator and offer manual refresh.

## 8. Security / safety

The server is bound to `127.0.0.1` by default; the read-only board
already warns when bound to a non-loopback address
(`src/server/mod.rs:97`). Adding writes raises the stakes.

Threat changes from adding writes:

- **Local CSRF**: another process on `localhost` (a malicious browser
  tab from another origin, a malicious VS Code extension) can POST to
  the loopback API. Mitigations:
  - **Origin / Sec-Fetch-Site enforcement**: PATCH requires
    `Sec-Fetch-Site: same-origin` and `Origin: http://127.0.0.1:<port>`.
    All modern browsers send these; non-browser clients bypass them
    but they're the realistic threat.
  - **Per-process token**: at startup, write a random token to
    `~/.cache/issuectl/serve.token` (mode 0600); the index page reads
    its own URL bar, fetches `/auth/handshake?token=…`, gets back a
    cookie. Local-only attacker without filesystem read on the user's
    cache directory can't proceed. Optional for v1; add when binding
    non-loopback.
- **Network exposure**: when bound to `0.0.0.0`, refuse all writes
  with 403 unless `--allow-remote-writes` is passed. (Reading is
  already documented as "trusted networks only".)
- **Symlink follow during write**: the existing `locate_issue` already
  refuses symlinked issue dirs. The atomic-write path must
  re-canonicalize the target inside the issue directory, just like the
  read path does (`src/server/api.rs:121`).
- **Path traversal in slug**: already blocked by `slug::is_valid` at
  the route extractor.

Operational hardening:

- **Request size limit**: `axum::extract::DefaultBodyLimit::max(64 KB)`
  on PATCH/PUT routes. Frontmatter is small; bodies are short.
- **Rate limit** (§5.3) doubles as DoS protection against a runaway
  client.
- **Watcher walks symlinks-not**: `Config::default().with_follow_symlinks(false)`,
  asserted with a test that drops an external symlink in
  `issues/open/` and verifies it's not followed.

## 9. Phasing

A four-stage plan that ships value at every checkpoint:

| Phase | Scope | Ship value |
| --- | --- | --- |
| **0. Read-side push (M1)** | Add `notify` watcher, broadcast channel, `/events` SSE; client subscribes and re-fetches `/api/issues` on `Resync`; no writes | Live updates while editing in `$EDITOR` or pulling git — the user immediately sees the board "feel" alive |
| **1. Status & assignment writes (M2)** | PATCH for status, priority, assignee, labels — i.e. the existing `cmd_update` fields; refactor `do_*` into `mutate.rs`; atomic write; optimistic-concurrency token; column drag-and-drop | Drag-to-move workflow; the most-requested CLI ergonomic gap closes |
| **2. Body editing (M3)** | PUT body with textarea + preview pane; rate-limited; Resync round-trips through SSE | Full edit-in-place, no `$EDITOR` round trip required |
| **3. Polish (M4)** | Three-way merge UI for body conflicts; Last-Event-ID resume; degraded-mode banner; `--watch-poll-ms` | Robustness for real multi-client use |

**Defer:** multi-user presence, OT/CRDT body merging, server-side undo,
new-issue creation form (POST), delete (just close → closed/), inline
attachments.

## 10. Trade-off summary

The hard sub-decisions, with my recommendation per row.

| Decision | Options | Pick | Why |
| --- | --- | --- | --- |
| Transport | SSE / WS / long-poll | **SSE** | One-way fits; built into axum; standard reconnect; debuggable |
| Write source | Library call / shell-out | **Library** | Fork-exec cost is real; dogfooding is via shared functions, not shared processes |
| Concurrency | mtime / frontmatter version / content hash | **Content hash** | Stable across `git checkout`, no extra fields, cheap |
| Conflict UX | Toast+reload / 3-way merge / OT | **Toast+reload** for v1 | Three-way merge = M4 polish; OT is years of work |
| Echo suppression | Always rebroadcast / write token / hash | **Rebroadcast + token tag** | Correctness from rebroadcast, UX polish from tagging |
| Body editor | textarea / CodeMirror / Monaco | **textarea + preview** | Zero new deps; bodies are short; CodeMirror is drop-in later |
| Patch shape | Per-field PATCH / whole-doc PUT | **Per-field PATCH** | Mirrors CLI; smaller wire; no JS YAML serialization |

## 11. Out of scope / open questions

Decisions the user should make before implementation starts:

1. **New-issue form in v1?** I propose deferring (`issuectl new` is the
   canonical path); the UI gets a "Copy CLI command" button instead.
2. **Body editor surface**: confirm textarea-first is acceptable, or
   pre-decide to ship CodeMirror in M3.
3. **Authentication for non-loopback bind**: add `--token` /
   `--allow-remote-writes` now or punt to its own follow-up? My
   inclination: punt; v1 stays loopback-only and refuses writes
   on non-loopback by default.
4. **`fsync` policy**: always, or `O_DSYNC`-style only, or skip?
   Recommendation: always fsync the file *and* the parent dir; the
   throughput cost is invisible at our write volume and the durability
   guarantee is worth it. Re-evaluate if profiling shows it.
5. **`mutate.rs` refactor as part of M0 or M1?** I'd do it in M1
   alongside the first PATCH endpoint, so M0 stays read-only and
   reviewable in isolation.
6. **Watcher polling fallback flag**: ship `--watch-poll-ms` from M0
   or wait for an actual NFS user? Marginal; defer.
7. **Session token storage**: `~/.cache/issuectl/` vs
   `$XDG_RUNTIME_DIR/`? `$XDG_RUNTIME_DIR` is correct but not
   universal; cache dir is portable. Defer with auth itself.
8. **Wire format**: JSON vs MessagePack on `/events`? JSON for now;
   payloads are tiny, debugging is easier.

## Appendix A — illustrative event types

```rust
// pseudocode — final shapes belong in mutate.rs / events.rs

#[derive(Serialize, Clone)]
#[serde(tag = "type")]
enum BoardEvent {
    IssueUpserted {
        seq: u64,
        slug: String,
        version: String,           // "sha256:…"
        origin: Origin,            // "self" | "external"
        issue: IssueSummary,
        body_html: Option<String>, // present iff a detail subscriber asked
    },
    IssueRemoved { seq: u64, slug: String, origin: Origin },
    Resync       { seq: u64, reason: String },
    Degraded     { seq: u64, reason: String },
}

enum Origin { SelfWrite { token: String }, External }
```

## Appendix B — illustrative PATCH flow

```
client                       server
  |    PATCH /api/issues/foo   |
  |  expected_version: abc…    |
  |--------------------------> |
  |                            |  read item.md, hash → check vs expected
  |                            |  apply field mutations (do_update logic)
  |                            |  serialize_frontmatter, write_item_atomic
  |                            |  insert WriteToken into AppState.own_writes
  |        200 + { version, token } |
  |<-------------------------- |
  |                            |
  |    (notify fires)          |
  |                            |  watcher debounces, re-parses
  |                            |  emits BoardEvent::IssueUpserted
  |                            |     origin: SelfWrite { token }
  |        SSE event           |
  |<-------------------------- |
  |  recognises own token,     |
  |  marks "saved" without     |
  |  flashing the form         |
```

---

*Last updated: 2026-05-06. This document precedes implementation; revise
in-place once code lands.*
