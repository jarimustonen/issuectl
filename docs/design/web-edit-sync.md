# Web ↔ File bidirectional sync — design

Status: **revision 2** after multi-LLM panel review. Implementation lives in a
separate worktree; this doc is a contract for that work, not code.

Round-1 + round-2 reviewer findings are synthesized in
`history/review-web-edit-sync.md`. Decisions on disputed/discussion items are
recorded in §13 below.

## 1. Goals & non-goals

**Goals**

- Edit issues in the browser; changes land in `issues/<folder>/<slug>/item.md`.
- External edits (`$EDITOR`, `git pull`, `issuectl update`, agents) propagate
  live to every connected board.
- Filesystem stays the source of truth. The server is a renderer + a thin
  write proxy, not a database.
- Every web action is reachable from the CLI. Anything the UI does must be
  expressible as `issuectl --json …`. AI agents and humans share the same
  surface.
- Concurrent writers — server, CLI, agents, `$EDITOR`, `git pull` — cannot
  silently lose each other's changes.

**Non-goals (this round)**

- Multi-user authentication, RBAC, audit logs.
- Operational-transform / CRDT collaborative editing.
- Cross-repo or remote (non-loopback) write access.
- Persistent server-side undo stack; the filesystem + git is the undo log.

## 2. Component diagram

```
┌──────────┐   HTTP+SSE     ┌──────────────────┐
│ browser  │ ◀────────────▶│ axum Router      │
└──────────┘                │  ├ GET html/api  │
   ▲                        │  ├ PATCH/PUT/POST│
   │                        │  └ GET /events   │
   │                        └────────┬─────────┘
   │                                 │
   │                                 ▼
   │                        ┌──────────────────┐
   │                        │  AppState        │
   │                        │   ├ root         │
   │                        │   ├ event_hub    │  seq + ring + broadcast
   │                        │   ├ slug_locks   │  DashMap<Slug, Mutex>
   │                        │   └ csrf_token   │  per-process
   │                        └────────┬─────────┘
   │                                 │
   │                                 ▼
   │                        ┌──────────────────┐
   │                        │  mutate.rs       │  shared by CLI + server
   │                        │  flock(.issuectl/write.lock)
   │                        └────────┬─────────┘
   │                                 │
   │                                 ▼
   │                        ┌──────────────────┐    ┌────────────────┐
   │                        │  issues/**       │ ◀──│ notify watcher │
   │                        └──────────────────┘    └────────┬───────┘
   │                                                         │
   │                                                         ▼
   │                                                ┌────────────────┐
   │                                                │ debouncer-full │
   │                                                │ + spawn_blocking
   │                                                │ for parse work │
   │                                                └────────┬───────┘
   │                                                         │
   └─── re-render ◀─────────── /events SSE ◀── EventHub ◀────┘
```

`AppState` (today: `{ root: Arc<PathBuf> }`) grows three things:

```rust
// illustrative, not final
pub struct AppState {
    pub root: Arc<PathBuf>,
    pub event_hub: Arc<EventHub>,                       // §5
    pub slug_locks: Arc<DashMap<String, Arc<Mutex<()>>>>, // per-issue serialisation
    pub csrf_token: Arc<str>,                            // generated at startup; §9
}
```

Two new long-running tokio tasks alongside `axum::serve`:

1. `watcher_task` — owns a `notify::RecommendedWatcher` rooted at
   `<root>/issues/`, debounces with `notify-debouncer-full`, filters
   `.issuectl-tmp-*`, dispatches parse work via `spawn_blocking`,
   broadcasts `BoardEvent`.
2. `serve` (existing) — adds `/events`, write endpoints, and the CSRF token
   bootstrap.

## 3. Mutation protocol

This is the **single contract** every writer must follow. The CLI
(`issuectl update`, `close`, `new`) and the web server both go through it.

### 3.1 Sequence

For any mutation:

```
1. acquire flock(LOCK_EX) on <root>/.issuectl/write.lock
2. locate_issue(slug)              ← canonical-path symlink check
3. read item.md, compute canonical_hash (§3.2)
4. if request supplied expected_version: compare; mismatch → 409
5. apply mutation in memory (mutate.rs)
6. atomic write (§3.3)
7. if status change crossed open↔closed: rename dir (§3.4)
8. compute new canonical_hash
9. release flock
10. return { version: new_canonical_hash, ... }
```

The `flock` is the cross-process barrier shared between CLI and server. The
**per-slug `tokio::sync::Mutex`** inside the server handles intra-process
serialisation without serialising unrelated issues.

The lock file `<root>/.issuectl/write.lock` is created on demand (mode
0600). The `.issuectl/` directory is excluded from the issues tree by
construction; if the user adds it to `.gitignore` is their call (default:
yes — issue `init-project` / `doctor` should add it).

### 3.2 Canonical hash

The version token is the SHA-256 of a **canonical** representation, not raw
file bytes. Empirically verified: a no-op `issuectl update foo --priority high`
rewrites both `updated:` (today's date) and `labels:` (block→flow style), so
raw-byte hashes diverge between read and write — they would 409 on every
PATCH.

Canonical form:

```rust
fn canonical_hash(item: &ItemFile) -> String {
    let mut h = Sha256::new();
    h.update(canonical_frontmatter_json(&item.frontmatter)); // sorted keys, normalised types
    h.update(b"\n---\n");
    h.update(item.body.trim_end().as_bytes());               // strip trailing whitespace
    format!("sha256:{}", hex::encode(h.finalize()))           // full 64-char hex
}
```

`canonical_frontmatter_json` is a deterministic projection: keys sorted,
strings unquoted-where-safe, sequences as JSON arrays. Independent of YAML
formatting choices made by `serde_yaml` round-trip.

`updated:` is **excluded** from the canonical hash. `do_update` bumps it on
every save, but that's metadata about the write, not user-meaningful content.
Including it would re-introduce the false-409 problem.

### 3.3 Atomic write

```rust
fn write_item_atomic(target: &Path, content: &str) -> Result<()> {
    let dir = target.parent().expect("must have parent");
    let mut tf = tempfile::Builder::new()
        .prefix(".issuectl-tmp-")
        .tempfile_in(dir)?;
    tf.write_all(content.as_bytes())?;
    tf.as_file().sync_all()?;          // fsync(file)
    tf.persist(target)?;                // atomic rename within same fs
    #[cfg(unix)] fsync_dir(dir)?;       // best-effort, swallow errors
    Ok(())
}
```

- `.issuectl-tmp-` prefix is the signal that lets the watcher filter our
  temp files before debouncing (§5.1).
- On SIGKILL, `Drop` doesn't run; orphan tempfiles are swept on next
  startup (`doctor` extension, §11).
- On Windows, `fsync_dir` is a no-op — `std::fs::File::open(dir)` returns
  `Err`, so the call is gated `#[cfg(unix)]`.

### 3.4 Status change = directory rename

Closing/reopening crosses the `open/` ↔ `closed/` boundary. Authoritative
rule: **directory wins.** Frontmatter `status:` follows folder.

Sequence inside the lock:

```
1. preflight: target dir <other_folder>/<slug> must not exist; else error
2. update frontmatter in memory (status, closed: date, etc.)
3. fs::rename(<this_folder>/<slug>, <other_folder>/<slug>)
4. write_item_atomic(<other_folder>/<slug>/item.md, new_content)
```

Crash gap between (3) and (4) leaves a renamed directory with stale
frontmatter content. The startup reconciler (spin-off, §11) detects
this on next `serve` and silently corrects the frontmatter to match the
folder, emitting a `LoadWarning`. Web UI surfaces the warning in the
existing `#warnings` strip.

### 3.5 Shared mutation DTO

`mutate.rs` exports one request type. Both clap and serde derive into it:

```rust
pub struct UpdateIssueRequest {
    pub slug: Slug,
    pub expected_version: Option<String>,
    pub status: Patch<String>,
    pub priority: Patch<String>,
    pub assignee: Patch<String>,
    pub owner: Patch<String>,
    pub epic: Patch<String>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
    pub add_related: Vec<String>,
    pub remove_related: Vec<String>,
    pub add_commits: Vec<CommitSpec>,
}

#[derive(Debug, Default)]
pub enum Patch<T> {
    #[default] Unspecified,
    Clear,
    Set(T),
}
```

`Patch<T>` distinguishes "field omitted" (`Unspecified` — leave alone),
"field present as null" (`Clear` — delete it), and "field present with
value" (`Set(T)`). `Option<T>` cannot represent this; ad-hoc handling
silently confuses callers. Custom serde for JSON (`null` ↔ `Clear`,
absent ↔ `Unspecified`); clap maps `--no-epic` to `Clear`, `--epic foo`
to `Set("foo")`, omission to `Unspecified`.

The current `do_new`, `do_update`, `do_close` move into `mutate.rs`
exporting structured `Result` types; both `cmd_*` and the axum handlers
consume them.

## 4. Web → files (HTTP API)

### 4.1 Routes

```
GET    /api/issues               list (with replay_from_seq cursor)
GET    /api/issues/{slug}        detail (frontmatter + body_markdown + body_html + version)
GET    /api/issues/{slug}/docs/{name}   side docs (existing)

POST   /api/issues               create (mirror `issuectl new`)
PATCH  /api/issues/{slug}        per-field mutation (mirror `issuectl update`)
PUT    /api/issues/{slug}/body   body-only (separate rate limit + size)
POST   /api/preview              markdown → sanitised HTML, no disk
GET    /events                   SSE stream
GET    /api/session              CSRF token bootstrap (§9)
```

No `DELETE` — closing is `update --status fixed` per the existing CLI;
there is no destructive delete in the design.

### 4.2 Request shapes

`PATCH /api/issues/{slug}`:

```json
{
  "expected_version": "sha256:4f8e7e2c…",
  "status":   "in-progress",
  "assignee": "alice",
  "priority": "high",
  "epic":     null,
  "add_labels":     ["frontend"],
  "remove_labels":  ["legacy"],
  "add_related":    ["@amber-loud-fox"],
  "remove_related": []
}
```

- `expected_version` required when caller cares about safety (always for
  AI agents); optional for naive callers.
- `epic: null` ≡ `Clear`; field absence ≡ `Unspecified`. Same pattern for
  `assignee`, `owner`, `priority`, `status`.
- `add_X: ["x"] + remove_X: ["x"]` (same value in both) → 400 `conflicting_intent`.
- Removing a value that isn't present → 200 (no-op, idempotent).
- Label/related casing preserved as-given; equality is byte-exact.

`PUT /api/issues/{slug}/body`:

```json
{
  "expected_version": "sha256:4f8e7e2c…",
  "body": "# Title\n\nMarkdown text…"
}
```

Body endpoint is separate from PATCH so the rate-limit profile (§9) and
size limit (1 MiB body vs 64 KiB metadata) can differ per route.

`POST /api/issues` mirrors `cmd_new`:

```json
{
  "type": "bug",
  "title": "...",
  "slug": "optional-override",
  "reporter": "alice",
  "assignee": "bob",
  "priority": "high",
  "labels":  ["frontend"],
  "related": ["@amber-loud-fox"],
  "description": "..."
}
```

`POST /api/preview` accepts `{ "body": "markdown" }` and returns
`{ "body_html": "..." }` using the same `sanitize_markdown` as
`render.rs`. No disk access. 1 MiB request limit. Per-session rate limit
shared with body PUT. CSRF-protected like other state-changing routes
(even though it's read-shaped, it accepts attacker-controlled markdown).

### 4.3 Error shape (RFC 7807-shaped)

```json
{
  "type":   "https://issuectl/errors/version_mismatch",
  "title":  "Version mismatch",
  "status": 409,
  "code":   "version_mismatch",
  "detail": "expected sha256:abc…, got sha256:xyz…",
  "issue":  { /* current full issue */ }
}
```

`code` is stable; `title`/`detail` are human strings. Concrete codes:
`version_mismatch` (409), `validation` (400), `conflicting_intent` (400),
`not_found` (404), `forbidden` (403), `rate_limited` (429),
`storage_full` (507), `internal` (500).

### 4.4 Server-internal call vs CLI shell-out

The server calls `mutate::update_issue(...)` directly. Reviewer GPT-5.5
correctly noted that the original "structural-equivalence test" was
hand-wavy; replaced with a real mechanism: one `UpdateIssueRequest` type,
clap and serde both derive into it, no test needed. Dogfooding remains
because the CLI uses the same Rust functions — the symbolic value of
"shells out" doesn't justify its 30–80 ms fork+exec cost or its locking
races with the watcher.

### 4.5 CLI parity additions

To uphold "anything the web does is reachable from CLI":

- `issuectl show <slug> --json` and `issuectl list --json` add a
  `version: "sha256:…"` field per issue.
- `issuectl update --json …` **requires** `--expected-version sha256:…`
  (DISCUSS D4 = B). Human invocations without `--json` keep working
  without it; `flock` still prevents corruption.
- `issuectl close --json` similarly requires it.
- New: `issuectl body set <slug> --stdin --expected-version sha256:…
  --json` (or `--from-file`) for body editing parity. Without this,
  M2's web body editor would have no CLI mirror.

## 5. Files → web (push)

### 5.1 Watcher

- Crate: `notify` + `notify-debouncer-full`. Full debouncer (not mini)
  preserves rename pairs needed for slug-rename detection (§5.3).
- Watch root: canonicalized `<root>/issues/`. Symlinks not followed
  (`Config::default().with_follow_symlinks(false)`).
- **Pre-debounce filter** drops paths whose final component matches
  `.issuectl-tmp-*` so our own atomic-write tempfiles never reach the
  parser. Editor swap files (`.foo.swp`, `.foo~`) are not filtered —
  they don't match valid issue paths so the slug resolver rejects them.
- Debounce window: 100–200 ms.
- All parse / sanitise / hash work runs in `tokio::task::spawn_blocking`,
  so a `git checkout` of 200 issues doesn't stall the watcher's event
  loop.

### 5.2 Slug resolution from event paths

```rust
fn issue_slug_from_event(issues_root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(issues_root).ok()?;
    let mut comps = rel.components();
    let folder = comps.next()?.as_os_str().to_str()?;
    if folder != "open" && folder != "closed" { return None; }
    let slug = comps.next()?.as_os_str().to_str()?;
    if !slug::is_valid(slug) { return None; }
    Some(slug.to_owned())
}
```

For `issues/open/foo/item.md`, this returns `"foo"`. The original doc
said "two levels above the changed file" which arithmetically gave
`"open"`; corrected.

### 5.3 Slug renames and folder moves

`git mv issues/open/old-slug issues/open/new-slug` produces a rename event
with both paths (full debouncer preserves them). Watcher emits both:

```
IssueRemoved  { slug: "old-slug" }
IssueUpserted { slug: "new-slug", ... }
```

Same shape applies to `open/foo` → `closed/foo` (status change) when an
external writer does it. The server's own status-change move (§3.4)
synthesises one `IssueUpserted` server-side and tags it with the new
version, but the watcher will also fire — clients dedupe by version
(§5.6).

### 5.4 Transport: SSE

Decision unchanged: SSE wins for one-way push. axum's `Sse` plus
`EventSource` covers reconnect.

### 5.5 EventHub and replay cursor

The original §2 sketch used `events: broadcast::Sender<BoardEvent>` plus
a separate "ring buffer of 256 events" — two contradictory mechanisms.
Replaced with one explicit type:

```rust
pub struct EventHub {
    seq: AtomicU64,
    tx: broadcast::Sender<BoardEvent>,
    ring: Mutex<VecDeque<BoardEvent>>,   // bounded, e.g. 1024
}

impl EventHub {
    pub fn current_seq(&self) -> u64 { self.seq.load(Ordering::Acquire) }

    pub fn publish(&self, payload: EventPayload) -> BoardEvent {
        let seq = self.seq.fetch_add(1, Ordering::AcqRel) + 1;
        let evt = BoardEvent { seq, payload, ts: Instant::now() };
        {
            let mut r = self.ring.lock();
            if r.len() == r.capacity() { r.pop_front(); }
            r.push_back(evt.clone());
        }
        let _ = self.tx.send(evt.clone());
        evt
    }

    pub fn replay_since(&self, since: u64) -> Replay { /* … */ }
}

pub enum Replay {
    Events(Vec<BoardEvent>),
    TooOld,                  // gap; client should resync
}
```

Initial-load cursor handoff (closes the REST↔SSE race the original doc
introduced):

```
GET /api/issues
  → captures snapshot_seq = event_hub.current_seq() BEFORE scan
  → scans filesystem
  → returns { snapshot_seq, issues, warnings }

client opens /events?since=<snapshot_seq>
  → server returns Replay::Events(buffered) then live stream
  → if Replay::TooOld → server sends Resync immediately
```

For reconnects, the standard `Last-Event-ID` header works the same way
on top of the same `replay_since` mechanism. Browsers can't set
`Last-Event-ID` on the *initial* `EventSource` connection, so the
`?since=` query param is required for that one case.

`broadcast::error::RecvError::Lagged(_)` → server emits `Resync` to that
subscriber and resets its cursor.

### 5.6 Event types

```rust
#[derive(Serialize, Clone)]
#[serde(tag = "type")]
pub enum BoardEvent {
    IssueUpserted {
        seq: u64,
        slug: String,
        version: String,           // full canonical hash
        issue: IssueSummary,        // summary only — no body_html
    },
    IssueRemoved {
        seq: u64,
        slug: String,
    },
    IssueInvalid {
        seq: u64,
        slug: String,
        warnings: Vec<LoadWarning>,  // reuse existing shape from repo.rs
    },
    Resync {
        seq: u64,
        reason: String,              // "bulk_change" | "watcher_restart" | "lagged" | "gap"
    },
}
```

Notable changes from the original:

- **No `body_html` in events.** The doc previously had
  `body_html: Option<String> // present iff a detail subscriber asked`,
  which is incompatible with a single broadcast channel. Detail dialogs
  refetch `/api/issues/{slug}` on relevant events. Bodies are short;
  fetch is cheap.
- **No `origin` / `WriteToken`.** Echo suppression is client-side
  (§6.4) — server sends the same event to everyone, the originating
  tab recognises its own `version` from the PATCH 200 response.
- **`IssueInvalid`** (new) — for malformed YAML, missing `item.md`,
  `<<<<<<<` merge markers, partially-written files. Reuses
  `repo::LoadWarning` shape; web UI already renders these in the
  `#warnings` strip. Card stays visible with an error badge instead
  of vanishing.

### 5.7 Bulk-change coalescing

If more than ~50 distinct slugs change within a single debounce window
(e.g. `git checkout` switching feature branches), the watcher emits one
`Resync { reason: "bulk_change" }` instead of fanning out per-issue
events. Threshold tunable via `--watch-bulk-threshold`.

### 5.8 Watcher restart

If the watcher task panics, exponential backoff retries up to 3 times.
On each successful (re)start — including the first — the watcher emits
`Resync { reason: "watcher_restart" }`. After 3 failures, the hub emits
`Degraded { reason: "watcher_unavailable" }` (clients show a red dot;
manual refresh button works).

## 6. Concurrency & conflict UX

### 6.1 Lost-update protection

Three layers, in order of strength:

1. **`flock(LOCK_EX)`** on `<root>/.issuectl/write.lock` — cross-process,
   shared by CLI and server. The only mechanism that survives "user runs
   `issuectl update` from terminal while server is up." Note: does not
   protect against `$EDITOR` saves or `git pull` — those are outside the
   tool's control and documented as such.
2. **Per-slug `tokio::sync::Mutex`** inside the server — serialises
   concurrent PATCHes to the same issue. Held across the whole
   lock→read→hash→write→rename sequence.
3. **`expected_version` optimistic check** — surfaces conflicts to the
   browser tab that started its edit before another writer changed the
   file. Required on AI-agent (`--json`) calls.

Layers 1 and 2 prevent corruption; layer 3 surfaces *user-visible*
conflicts. Removing layer 3 would silently overwrite changes; removing
1 or 2 would corrupt the file.

### 6.2 Status/folder authority

DISCUSS #19 = A: directory wins. If reconciler or any read path detects
mismatch between folder (`closed/`) and frontmatter (`status: open`), the
folder is authoritative; frontmatter is rewritten to a sane default
(closed-folder issues with active `status:` get `status: done` and a
`LoadWarning` entry; open-folder issues with closing `status:` get
`status: open` and a warning).

The web UI surfaces these warnings in the existing `#warnings` strip.
No new UI needed.

### 6.3 Body conflict UX (M2)

When `PUT /body` returns 409:

1. The server's fresh `IssueDetailResponse` is shown alongside the user's
   current draft (split pane).
2. The textarea **is not overwritten**. The user's typed content stays
   exactly where they put it. Reviewer concern: silently wiping a body
   into an "in-memory clipboard" is data-loss UX.
3. `localStorage` already has a backup keyed by
   `(slug, started_editing_at)` — written every keystroke (free,
   synchronous, survives tab-close, kraashes, network failure).
4. User picks: "keep mine" (PATCH again with new `expected_version`),
   "keep theirs" (discard local, accept server), or manually merge in
   the textarea then save.

For **metadata** PATCHes (status, priority, etc.), the simpler
"toast + reload" UX is fine — there's no draft to lose.

### 6.4 Echo suppression

DISCUSS #6 + paneeli: drop server-side `WriteToken` machinery.

Mechanism:

1. PATCH 200 response includes the new canonical `version`.
2. Client stores `local_version = response.version`.
3. SSE delivers `IssueUpserted { version: V, ... }`.
4. If `V === local_version`: the originating tab silently reconciles
   in place ("Saved" indicator instead of full re-render flash).
5. If `V !== local_version`: full re-render, treat as external edit.

This works for the originating tab. Other tabs (same user, different
tab) treat the event as external — correct, because from their
perspective it *is* external. No server state required.

### 6.5 Body autosave (M2)

DISCUSS D3 = C:

- 5 s debounce after last keystroke.
- Save on `blur`.
- `localStorage` written on every keystroke (independent of network).
- Manual `Ctrl+S` / save button always available.
- No save on `tab-hide` / `pagehide` — `EventSource`-style loss is
  too unreliable; `localStorage` covers crash/close anyway.

### 6.6 Rate limiting

Per-`(slug, session)` token bucket — not per-IP (every loopback client
is `127.0.0.1`, so per-IP would collapse to global). Defaults:

- `PATCH /api/issues/{slug}`: 10 req/sec, burst 30.
- `PUT /api/issues/{slug}/body`: 4 req/sec, burst 10. Body autosave
  at 5 s debounce is well under this.
- `POST /api/preview`: shares the body bucket.

Excess returns 429 with `Retry-After`. Client backs off.

## 7. Edit granularity in the UI

### 7.1 Frontmatter / status

- **Status**: drag card between columns; also `<select>` in the detail
  dialog. Both call `PATCH { status: ... }`.
- **Priority**, **type**: `<select>` constrained to `PRIORITIES` /
  `ISSUE_TYPES` from `src/main.rs:19`.
- **Assignee / owner / reporter / epic**: free-text inputs, validated
  client-side against `slug::is_valid`-shaped patterns where applicable.
- **Labels**, **related**: chip inputs with add/remove.

Each interaction → one PATCH.

### 7.2 Body

`<textarea>` with monospace font + `POST /api/preview` for a side-by-side
preview pane. No CodeMirror, no Monaco — they would dwarf the binary and
contradict the project's no-build-step / no-JS-deps stance. CodeMirror 6
remains a drop-in upgrade if `tab` key handling or line-wrapping become
real pain points.

## 8. Failure modes

### 8.1 Watcher misses events

- **macOS FSEvents** is per-directory. The full debouncer + slug-based
  re-parse covers the missing per-file granularity.
- **Network filesystems**: `notify` falls back to polling. CLI flag
  `--watch-poll-ms <ms>` forces polling explicitly. `serve --no-watch`
  disables the watcher (read-only board, manual refresh button only).
- **Linux inotify queue overflow** under bulk operations: emits
  `Resync { reason: "bulk_change" }`.

### 8.2 Disk full / permission errors

Atomic write fails at `persist`. Returns 507 (storage_full) with the
RFC-7807-shaped body. On-disk state unchanged because the rename never
happened. `tempfile`'s `Drop` cleans up; if `Drop` skipped (SIGKILL),
startup sweep handles it.

### 8.3 Disconnected clients

`EventSource` auto-reconnects with `Last-Event-ID`. Server uses
`replay_since` to fill the gap, or emits `Resync` if the gap is too
large. See §5.5.

### 8.4 Crash mid-write

Atomic write guarantees readers see either the pre- or post-image,
never half. Crash between rename and content write (status change
sequence §3.4) leaves a renamed dir with stale content; reconciler
fixes on next startup.

### 8.5 Watcher itself crashes

Exponential backoff up to 3 retries; emit `Resync` on each successful
restart; demote to `Degraded` after 3 failures.

### 8.6 Malformed external writes

Vim-in-place writes that fire events mid-save, `git merge` leaving
conflict markers, partial saves: parser returns `Err`, watcher emits
`IssueInvalid` with the warning message. Card stays visible. Next
PATCH attempt against an invalid issue: server returns 409 with the
fresh (still-invalid) issue and `code: "version_mismatch"`; client
surfaces "this issue has parse errors — fix it on disk first" rather
than overwriting the invalid file.

## 9. Security

### 9.1 Threat model

`issuectl serve` is bound to `127.0.0.1` by default. Adding writes
raises the stakes:

- **Local CSRF**: malicious browser tab from another origin, or a
  malicious local process (npm postinstall, browser extension, IDE
  plugin) issuing PATCH/PUT to loopback.
- **DNS rebinding**: an attacker website resolves a subdomain to
  `127.0.0.1` to bypass same-origin policy.
- **Network exposure**: `--host 0.0.0.0` for read-only is documented
  as "trusted networks only"; with writes it must be opt-in.

### 9.2 Mechanism

DISCUSS #17 = A: per-process CSRF token.

1. Server generates a random 256-bit token at startup, kept in
   `AppState.csrf_token`. Not persisted; restart → new token.
2. `GET /api/session` (same-origin) returns
   `{ "csrf_token": "<token>" }`. The HTML shell loads it once on
   page load; `board.js` stashes it in memory.
3. All state-changing routes (PATCH, PUT, POST writes, POST preview)
   require header `X-Issuectl-CSRF: <token>`. Missing/wrong → 403.
4. **Host validation** on every request: header must match the
   server's actual bind host:port (`127.0.0.1:<port>` /
   `localhost:<port>` / `[::1]:<port>`). Defeats DNS rebinding.
5. `Origin` and `Sec-Fetch-Site` checks remain as defense-in-depth
   for browser-origin requests.

### 9.3 SSE auth

`EventSource` cannot set custom headers, so `/events` cannot use
`X-Issuectl-CSRF`. Instead:

- A `SameSite=Strict; HttpOnly` cookie is set on `GET /api/session`,
  carrying a session ID.
- `/events` requires the session cookie; rejects without it.
- `Host` validation applies.

The cookie and the `X-Issuectl-CSRF` header serve different purposes
(SSE vs state-change) and must not be conflated.

### 9.4 Non-loopback bind

When `--host` is non-loopback:

- All write routes return 403 unless `--allow-remote-writes` is also
  passed.
- `--allow-remote-writes` requires `--auth-token-file <path>` (this
  *is* a persisted token for remote use; loopback per-process model
  doesn't fit). If neither is set, `serve` refuses to start.

This is a future feature; M0–M3 stays loopback-only for writes.

### 9.5 Other hardening

- Request size limits per route: PATCH metadata 64 KiB, PUT body
  1 MiB, POST preview 1 MiB.
- Atomic-write target re-canonicalised inside `locate_issue`-style
  guard before persist; symlink swap between validation and write
  is prevented by holding `flock` across the whole sequence.
- Watcher does not follow symlinks (`with_follow_symlinks(false)`).
- `NamedTempFile` placement inside the watched dir is fine because
  the tempfile prefix is filtered (§5.1).

## 10. Phasing

| Phase | Scope | Ship value |
| --- | --- | --- |
| **M0** | `EventHub` + `notify-debouncer-full` watcher + `/events` SSE + `replay_from_seq` cursor in `/api/issues`. No writes. | Live read-side updates: `$EDITOR` saves, `git pull`, agent edits all show up in the board immediately. |
| **M1** | `mutate.rs` refactor with `Patch<T>` + `UpdateIssueRequest` + `flock` + per-slug mutex + canonical hash. CSRF token + `Host` validation. PATCH metadata routes. CLI: `--expected-version`, `version` field in `--json` output. Drag-to-move in UI. | Status drag, label/assignee edits — most-requested ergonomic gap. |
| **M2** | `PUT /body` + textarea + `POST /api/preview` + `localStorage` draft + body conflict UX. `IssueInvalid` event surfacing. CLI: `issuectl body set`. | Full edit-in-place. |
| **M3** | `--watch-poll-ms`, `--no-watch`, `Degraded` banner, three-way merge UI for body conflicts. | Robustness for real multi-client use. |

**Spin-offs** (own issues, off the M0–M3 path):

- Startup reconciliation (extends `issuectl doctor`) — see issue
  created in this revision.
- Field-level merge for commuting metadata PATCHes — only build if
  M2 user reports show 409 friction is real.

**Not building**: idempotency keys (cosmic ray on loopback), SSE event
schema versioning ("unknown type → Resync" client fallback is enough),
watcher heartbeat marker files (channel-based liveness instead),
explicit mode-bit preservation on atomic write (issue files don't have
exotic perms).

## 11. Trade-off summary

| Decision | Options | Pick | Why |
| --- | --- | --- | --- |
| Transport | SSE / WS / long-poll | **SSE** | One-way fits; built into axum; standard reconnect |
| Write source | Library call / shell-out | **Library** | Fork-exec cost; `mutate.rs` shared DTO does dogfooding right |
| Concurrency token | mtime / counter / raw hash / canonical hash | **Canonical hash** | Empirically verified raw bytes false-409 every PATCH |
| Cross-process locking | None / flock / OS mutex | **flock** | Simple, shared with CLI, single-user tool |
| Conflict UX | Toast+reload / 3-way merge / OT | **Toast+reload for metadata, preserve-dirty + localStorage for body** | Body draft loss is the worst failure mode |
| Echo suppression | Server tokens / client hash compare / always rebroadcast | **Client hash compare** | Server state buys nothing real |
| Body editor | textarea / CodeMirror / Monaco | **textarea + preview** | Zero new deps |
| PATCH shape | Per-field add/remove / whole-doc PUT / RFC 6902 | **Per-field add/remove** | CLI parity, intent-preserving, AI-friendly |
| Replay mechanism | broadcast::Sender alone / explicit EventHub | **EventHub** | broadcast can't replay by `Last-Event-ID` |
| Initial state ↔ stream | Snapshot-then-stream / cursor handoff | **`replay_from_seq` cursor in REST** | Closes the lost-event window |
| CSRF | Punt to v2 / per-process token / persistent token | **Per-process token from M1** | Loopback ≠ trusted; npm postinstall is realistic |
| Status/folder authority | Directory / frontmatter / diagnose-only | **Directory** | `git mv` works; reconciler rule is one line |
| Body autosave | None / 750 ms / 5 s + localStorage | **5 s + localStorage + manual** | Covers tab-close + avoids 409 storms |

## 12. Decisions record

DISCUSS items resolved during review:

- **D3 (body autosave)** = **C**: 5 s debounce + `localStorage` + manual save.
- **D4 (CLI `--expected-version`)** = **B**: required when `--json`,
  optional otherwise. `flock` covers corruption either way.
- **D5 (PATCH array semantics)** = **A**: keep `add_*`/`remove_*` for
  CLI parity; specify edge cases (duplicate add+remove → 400; absent
  remove → no-op).
- **#17 (CSRF token persistence)** = **A**: per-process. Non-loopback
  uses a separate persistent `--auth-token-file`.
- **#19 (status/folder authority)** = **A**: directory wins; reconciler
  rewrites frontmatter to match; web UI surfaces a `LoadWarning`.

Spin-offs filed (won't block M0–M3):

- **#23**: Startup reconciliation — extends `issuectl doctor`.
- **#24**: Field-level merge for commuting metadata PATCHes — only
  build if real-world 409 friction shows up post-M2.

Dropped:

- **#25** Idempotency keys: cosmic ray on loopback.
- **#26** SSE schema versioning: same-binary deployment + client
  "unknown type → Resync" fallback is enough.
- **#27** Watcher heartbeat marker file: writes to repo to test itself.
- **#28** Mode-bit preservation: no real users with exotic perms on
  issue files.
- **#29** Windows fsync edge: gated `#[cfg(unix)]`; Windows path is
  documented as undefined.

## 13. Out of scope / open questions

These are genuine open questions, not deferred decisions:

1. **`flock` lock granularity**: repo-wide vs per-issue. Repo-wide is
   simpler and what M1 ships. Revisit if profiling shows contention
   under bursty server-side body autosaves.
2. **`new` endpoint UI**: separate "Copy CLI command" button vs full
   form. Lean: defer to M3+; CLI is the canonical creation path.
3. **`watch-poll-ms` default for NFS**: ship in M0 or wait for an
   actual NFS user? Lean: defer; nobody runs `serve` over NFS yet.
4. **Wire format for `/events`**: JSON only. MessagePack is a
   non-improvement at this scale.

## Appendix A — illustrative event types

```rust
// pseudocode — final shapes belong in events.rs

#[derive(Serialize, Clone)]
#[serde(tag = "type")]
pub enum BoardEvent {
    IssueUpserted {
        seq: u64,
        slug: String,
        version: String,
        issue: IssueSummary,    // no body_html
    },
    IssueRemoved {
        seq: u64,
        slug: String,
    },
    IssueInvalid {
        seq: u64,
        slug: String,
        warnings: Vec<LoadWarning>,
    },
    Resync {
        seq: u64,
        reason: String,
    },
    Degraded {
        seq: u64,
        reason: String,
    },
}
```

## Appendix B — illustrative PATCH flow

```
client                              server
  |  GET /api/session                |
  |  -------------------------->     |
  |  set-cookie: sid=abc; SameSite   |
  |  { "csrf_token": "<tok>" }       |
  |  <--------------------------     |
  |                                  |
  |  GET /api/issues                 |
  |  -------------------------->     |
  |                                  |  snapshot_seq = hub.current_seq()
  |                                  |  scan filesystem
  |  { snapshot_seq: 42, issues:[] } |
  |  <--------------------------     |
  |                                  |
  |  GET /events?since=42            |
  |  Cookie: sid=abc                 |
  |  -------------------------->     |  open SSE; replay_since(42) → live
  |                                  |
  |  PATCH /api/issues/foo           |
  |  X-Issuectl-CSRF: <tok>          |
  |  Cookie: sid=abc                 |
  |  { expected_version: "sha256:…", |
  |    status: "in-progress" }       |
  |  -------------------------->     |
  |                                  |  flock(LOCK_EX)
  |                                  |  per-slug mutex
  |                                  |  read item.md, hash
  |                                  |  check expected_version
  |                                  |  apply mutation
  |                                  |  write_item_atomic
  |                                  |  recompute hash → V_new
  |                                  |  release locks
  |                                  |  hub.publish(IssueUpserted{V_new})
  |  200 { version: V_new, issue }   |
  |  <--------------------------     |
  |  client stores local_version = V_new
  |                                  |
  |  (notify fires; watcher debounces)
  |                                  |  re-parse, recompute hash → V_new
  |                                  |  hub.publish(IssueUpserted{V_new})
  |                                  |  (de-duped against the synthetic one
  |                                  |   above by version equality)
  |  SSE event { version: V_new }    |
  |  <--------------------------     |
  |  V === local_version → silent reconcile,
  |  show "saved" indicator instead of re-render.
```

---

*Last revised: 2026-05-06 after multi-LLM panel review. Implementation
starts in a separate worktree once spin-off issues for reconciliation
and field-level merge are filed.*
