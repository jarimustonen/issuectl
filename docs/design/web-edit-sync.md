# Web ↔ File bidirectional sync — design

Status: current contract for the web edit/sync work. Implementation lives in
a separate worktree; this doc is a contract for that work, not code.

The on-disk layout is flat: every issue lives at `issues/<slug>/item.md`.
Status is read from frontmatter; the kanban-bucket label
(`open` / `closed`) seen in API payloads is derived from
`is_closing_status(fm.status)` — purely a presentation detail, not a
parallel state. (`is_closing_status` is the predicate defined in
`src/main.rs` that returns true for terminal-status values like
`fixed`, `closed`, `wontfix`; see `is_closing_status_classifies_correctly`
test for the canonical set.) Legacy `issues/{open,closed}/<slug>/` paths are still
accepted on read (compat layer in `repo::locate_issue_full` +
`mutate::locate_and_migrate`); writes always migrate the slug to the
flat path under `flock`. The one-shot bulk migration is `issuectl
doctor migrate-layout`.

Design history (review passes, superseded decisions, what shaped the
current shape) is in Appendix C.

## 1. Goals & non-goals

**Goals**

- Edit issues in the browser; changes land in `issues/<slug>/item.md`.
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

`AppState`:

```rust
// illustrative, not final
pub struct AppState {
    pub root: Arc<PathBuf>,
    pub event_hub: Arc<EventHub>,                  // §5
    pub csrf_token: Arc<str>,                      // generated at startup; §9
}
```

The repo-wide `flock` (§3.1) serialises all writes for a single user's
local tool. There is no in-process per-slug mutex on top of it: `flock`
already serialises everything, so a per-slug `tokio::sync::Mutex` would
add zero concurrency. If profiling later shows contention, switch the
`flock` to per-issue lock files instead.

Two long-running tokio tasks alongside `axum::serve`:

1. `watcher_task` — owns a `notify::RecommendedWatcher` rooted at
   `<root>/issues/`, debounces with `notify-debouncer-full`, filters
   `.issuectl-tmp-*`, dispatches parse work via `spawn_blocking`,
   broadcasts `BoardEvent`.
2. `serve` (existing) — adds `/events`, write endpoints, and the CSRF token
   bootstrap.

## 3. Mutation protocol

This is the **single contract** every issuectl-mediated writer must follow.
The CLI (`issuectl update`, `close`, `new`) and the web server both go
through it.

**Precondition / scope:** this protocol prevents silent loss only among
writers that take `<root>/.issuectl/write.lock`. External writers that do
not — `$EDITOR`, `git pull`, `git checkout`, hand-applied patches,
arbitrary scripts — can still race. A concurrent `$EDITOR` save between
this protocol's read and rename can be overwritten. Documented as
unavoidable for the local-FS-as-source-of-truth model; mitigation is
optimistic concurrency surfacing the conflict to the user (§6.3) and
startup reconciliation (spin-off).

### 3.1 Sequence

For any mutation, all blocking I/O runs inside `tokio::task::spawn_blocking`
(server side); CLI runs synchronously. The sequence:

```
1. acquire flock(LOCK_EX) on <root>/.issuectl/write.lock
2. locate_and_migrate(slug)        ← see 3.1.1
3. read item.md, compute canonical_hash (§3.2)
4. if request supplied expected_version: compare; mismatch → 409
5. apply mutation in memory (mutate.rs)
6. write_item_atomic (§3.3) to <root>/issues/<slug>/item.md
7. compute new canonical_hash from final on-disk content
8. event_hub.publish(IssueUpserted { version: V_new, ... })
   ← still holding flock, before release; closes the reorder window in §5.5
9. release flock
10. return { version: V_new, ... } to caller
```

Step 8 (publish-before-release) closes the reorder window described in
§5.5 — without it, two writers that release their locks before
publishing can land their events at clients in opposite order, breaking
the dedup-by-version invariant in §6.4.

The lock guard is RAII-bound: any panic or `tokio::task` cancellation
between 1 and 9 unwinds and drops the file handle, releasing the lock.
After-the-fact recovery (e.g. a partially-written tempfile) is the
startup reconciler's job.

**Lock acquisition order**: only one lock — the repo `flock`. There is
no second lock to deadlock against. CLI and server both acquire only
`flock`, so no inversion possible.

The lock file `<root>/.issuectl/write.lock` is created with mode `0o600`
on Unix:

```rust
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

let file = OpenOptions::new()
    .read(true).write(true).create(true)
    .mode(0o600)             // explicit; create(true) alone honours umask
    .open(lock_path)?;
fs2::FileExt::lock_exclusive(&file)?;
```

The `.issuectl/` directory is excluded from the issues tree by
construction; `doctor --fix` adds it to `.gitignore` if missing.

#### 3.1.1 `locate_and_migrate(slug)` semantics

The slug resolver runs inside the lock, before any read or write.
Outcomes:

- `Ok(folder)` — exactly one canonical or legacy location resolves to a
  directory (verified non-symlink via `symlink_metadata` +
  canonical-path-prefix check) containing `item.md`. If the hit was at
  a legacy `issues/{open,closed}/<slug>/` path, the directory is moved
  to the canonical `issues/<slug>/` path under the same `flock` before
  any subsequent read or write. After the rename the parent `issues/`
  directory is `fsync`ed so the new location is durable on crash.
- `Err(NotFound)` — neither canonical nor legacy paths exist. Mutation
  handlers map to 404.
- `Err(AmbiguousSlug)` — multiple locations exist (canonical + legacy,
  or both legacy folders simultaneously). Mutation handlers map to 409
  with `code: "ambiguous_slug"` and a `detail` pointing the user at
  `issuectl doctor migrate-layout`. Silently picking one side would
  hide divergence introduced by a partially-completed migration.
- `Err(Symlink | Invalid)` — symlink escape attempt or non-directory at
  the slug path. 403 / 400, same as today's `repo::locate_issue`.

**Side-effect ordering.** `locate_and_migrate` runs at step 2 of the
mutation sequence (§3.1), *before* the `expected_version` check at
step 4. A stale-version PATCH that ends in 409 still leaves the
directory permanently migrated. This is intentional — migration is a
strict layout cleanup, independent of mutation success — but it
means a 409 client cannot assume the on-disk path was unchanged.

**Watcher event suppression during migration.** The directory rename
in step 2 produces filesystem events the watcher would otherwise
translate into `IssueRemoved(slug)` (legacy path) +
`IssueUpserted(slug)` (canonical path), arriving alongside the
mutator's synthetic `IssueUpserted(slug, V_new)`. To avoid a
disappear/reappear flicker on connected clients, `mutate.rs` records
`(slug, V_new)` in a short-lived (≥ debounce window + slack) ignore
set; the watcher drops events whose `(slug, computed_version)` matches
*and* drops the paired `IssueRemoved` for the same slug within the
same debounce batch.

**`POST /api/issues` (create) collision behavior.** The create path
also runs `locate_and_migrate` first. If it returns `Ok(_)` —
canonical or legacy — the create is rejected with 409
`slug_conflict`. If it returns `Err(NotFound)`, the create proceeds
under the same flock. This makes "new" deterministic in the presence
of legacy paths.

### 3.2 Canonical hash

The version token is the SHA-256 of a **canonical** representation, not raw
file bytes. Empirically verified: a no-op `issuectl update foo --priority high`
rewrites both `updated:` (today's date) and `labels:` (block→flow style), so
raw-byte hashes diverge between read and write — they would 409 on every
PATCH.

```rust
fn canonical_hash(item: &Item) -> String {
    let json = canonical_frontmatter_value(&item.frontmatter);
    let mut h = Sha256::new();
    h.update(serde_jcs::to_vec(&json).unwrap()); // RFC 8785 JCS: sorted keys, no whitespace
    h.update(b"\n---\n");
    h.update(normalize_body(&item.body).as_bytes());
    format!("sha256:{}", hex::encode(h.finalize()))           // full 64-char hex
}

/// Deterministic projection of frontmatter into a `serde_json::Value`.
/// `updated:` is excluded — it is bumped on every save and would re-introduce
/// false-409s. Unknown keys preserved by the loader are included so
/// undocumented user fields participate in concurrency control.
fn canonical_frontmatter_value(fm: &Frontmatter) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    m.insert("type".into(),     fm.issue_type.clone().into());
    m.insert("status".into(),   fm.status.clone().into());
    m.insert("priority".into(), fm.priority.clone().into());
    m.insert("title".into(),    fm.title.clone().into());
    if let Some(v) = &fm.created   { m.insert("created".into(),   v.clone().into()); }
    if let Some(v) = &fm.closed    { m.insert("closed".into(),    v.clone().into()); }
    if let Some(v) = &fm.reporter  { m.insert("reporter".into(),  v.clone().into()); }
    if let Some(v) = &fm.assignee  { m.insert("assignee".into(),  v.clone().into()); }
    if let Some(v) = &fm.owner     { m.insert("owner".into(),     v.clone().into()); }
    if let Some(v) = &fm.epic      { m.insert("epic".into(),      v.clone().into()); }
    if let Some(v) = &fm.labels    { m.insert("labels".into(),    v.clone().into()); }
    if let Some(v) = &fm.related   { m.insert("related".into(),   v.clone().into()); }
    if let Some(v) = &fm.commits   { m.insert("commits".into(),   serde_json::to_value(v).unwrap()); }
    for (k, v) in &fm.unknown {     m.insert(k.clone(),           v.clone()); }
    // deliberately omitted: updated
    serde_json::Value::Object(m)
}

/// Normalize CRLF→LF and trim only trailing newlines (NOT arbitrary
/// Unicode whitespace — `trim_end()` would strip nbsp / U+2028 / etc.
/// which can be legitimate body content).
fn normalize_body(body: &str) -> Cow<'_, str> {
    let crlf_normalized = if body.contains('\r') {
        Cow::Owned(body.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(body)
    };
    Cow::Owned(crlf_normalized.trim_end_matches('\n').to_owned())
}
```

Notes on the projection:

- **Status comes straight from frontmatter.** There is no folder axis to
  reconcile, so no `directory_authoritative_status` indirection.
- **Sorted keys + no whitespace**: use `serde_jcs` (RFC 8785 JCS) or
  another conforming implementation. Two correct implementations must
  produce identical bytes for identical content.
- **`title` included.** `title` is a frontmatter field (`Issue.title`)
  and must participate in concurrency control. Although the web/CLI
  PATCH paths do not currently expose title mutation, hand-edits to
  `item.md` and `issuectl new` write title bytes; without it in the
  hash, two concurrent writers could clobber each other's title with
  no 409.
- **`updated:` excluded** — `do_update` bumps it on every save. Two
  files differing only in `updated:` are treated as equal. This is fine
  because `updated:` is generated, not user-authored.
- **Unknown fields included.** `Frontmatter::unknown: BTreeMap<String,
  serde_json::Value>` is populated at *load time*: the YAML→JSON
  conversion happens inside the parser. JSON-incompatible YAML
  constructs (non-string mapping keys, YAML tags, non-finite floats)
  fail this conversion and surface as `LoadWarning`s on the issue;
  the affected file is then refused at mutation entry as
  `MutateError::Corrupt` and never reaches the hash function. By the
  time `canonical_frontmatter_value` runs, every value in `unknown`
  is a well-formed `serde_json::Value`. Including unknowns in the
  projection prevents silent clobbers of fields the writer didn't
  read (`triage:`, `reviewer:`, etc.); the on-disk round-trip already
  preserves them via the raw `Mapping` in `write::ItemFile`, so this
  closes the version-hash side of the same gap.
- **`canonical_hash` is computed in `mutate.rs`** so CLI and server use
  the same function. The CLI exposes it via `issuectl show --json`
  (`version` field).

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
    #[cfg(unix)]
    if let Err(err) = fsync_dir(dir) {
        tracing::warn!(?err, path = %dir.display(), "fsync issue dir failed");
        // best-effort: rename succeeded; on crash the rename may not be
        // durable but the file content is. Acceptable for issue metadata.
    }
    Ok(())
}
```

- `.issuectl-tmp-` prefix is the signal that lets the watcher filter our
  temp files before debouncing (§5.1).
- On SIGKILL, `Drop` doesn't run; orphan tempfiles are swept on next
  startup (`doctor` extension).
- **Body line endings are normalised to LF before write.** The same
  CRLF→LF conversion that `normalize_body` performs for hashing
  (§3.2) is applied to the bytes written to disk — i.e. the on-disk
  body is the canonical form, and round-tripping through this code
  cannot introduce noisy CRLF↔LF diffs. Editors that paste CRLF into
  the textarea silently land LF on disk; this is intentional product
  policy.
- On Windows, `fsync_dir` is a no-op — `std::fs::File::open(dir)` is
  not portable to directories — so the call is gated `#[cfg(unix)]`.
  The atomic-rename and tempfile-cleanup paths still work; the only
  thing missing is parent-directory durability on power loss, which
  is acceptable for issue metadata.

### 3.4 Status change is a frontmatter PATCH

A status transition is a plain frontmatter mutation. The issue lives at
`issues/<slug>/item.md` regardless of status; nothing renames, nothing
moves. Inside the lock:

```
1. update frontmatter in memory:
   - new status
   - closed: today (if transitioning to a closing status)
   - remove `closed:` (if reopening)
2. write_item_atomic(<root>/issues/<slug>/item.md, content)
```

The kanban-bucket label that surfaces in API payloads
(`open` / `closed`) is derived from `is_closing_status(fm.status)` —
not stored, not authoritative.

### 3.5 Shared mutation DTO

`mutate.rs` exports one request type. Both clap and serde derive into it:

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateIssueRequest {
    pub slug: Slug,
    #[serde(default)] pub expected_version: Option<String>,
    #[serde(default)] pub status:   Patch<String>,
    #[serde(default)] pub priority: Patch<String>,
    #[serde(default)] pub assignee: Patch<String>,
    #[serde(default)] pub owner:    Patch<String>,
    #[serde(default)] pub epic:     Patch<String>,
    #[serde(default)] pub add_labels:     Vec<String>,
    #[serde(default)] pub remove_labels:  Vec<String>,
    #[serde(default)] pub add_related:    Vec<String>,
    #[serde(default)] pub remove_related: Vec<String>,
    #[serde(default)] pub add_commits:    Vec<CommitSpec>,
}

#[derive(Debug, Default)]
pub enum Patch<T> {
    #[default] Unspecified,
    Clear,
    Set(T),
}
```

`Patch<T>` distinguishes:

| Source | Variant |
| --- | --- |
| JSON: field absent | `Unspecified` (leave alone) |
| JSON: `"epic": null` | `Clear` |
| JSON: `"epic": "foo"` | `Set("foo")` |
| JSON: `"epic": ""` | **400 validation** (empty-string set is rejected; use `null` to clear) |
| JSON: `"epic": 123` | **400 validation** (no type coercion) |
| clap: `--no-epic` | `Clear` |
| clap: `--epic foo` | `Set("foo")` |
| clap: `--epic ""` | **CLI error** (clap's `parse_non_empty` already rejects this) |
| clap: omitted | `Unspecified` |

`#[serde(default)]` on every field is mandatory — without it, omitting
a field deserialises as an error instead of `Unspecified`.
`#[serde(deny_unknown_fields)]` catches typos like `"priorty": "high"`
that would otherwise silently parse as `Unspecified`.

Validation runs once after both clap and serde conversion via
`UpdateIssueRequest::validate()`:

- `add_X` and `remove_X` cannot share a value → 400 `conflicting_intent`.
- `add_X`/`remove_X` cannot contain duplicates within themselves.
- `add_X`/`remove_X` Vec entries that are empty or whitespace-only
  → 400 `validation` (same rule as scalar `Patch::Set("")`). This
  prevents `add_labels: [""]` from landing an empty label.
- Removing an absent value → no-op (idempotent).
- `status`, `priority` must be in their enum value sets (`STATUSES` /
  `PRIORITIES`).

**Immutable fields** not in the request type: `slug` (identity),
`created` (set at `new` time only), `reporter` (currently — could be
added if the use case appears), `title` (set at `new` time;
hand-editable in `item.md` but not exposed via PATCH), `type` (set at
`new` time, deliberately not mutable post-creation — type changes are
rare enough to warrant a CLI-only reset path if ever needed).
`closed:` is set/cleared automatically by the status-change logic; not
user-settable via PATCH. `updated:` is set by every successful
mutation.

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
  "type":   "urn:issuectl:errors:version_mismatch",
  "title":  "Version mismatch",
  "status": 409,
  "code":   "version_mismatch",
  "detail": "expected sha256:abc…, got sha256:xyz…",
  "issue":  { /* current full issue */ }
}
```

`code` is stable; `title`/`detail` are human strings. Concrete codes:
`version_mismatch` (409), `ambiguous_slug` (409), `slug_conflict`
(409, from `POST /api/issues` against an existing canonical or legacy
slug), `validation` (400), `conflicting_intent` (400), `not_found`
(404), `forbidden` (403), `rate_limited` (429), `storage_full` (507),
`internal` (500).

For `version_mismatch` (and only for it), the response includes
`issue: IssueDetailResponse` — same shape as `GET /api/issues/{slug}`,
i.e. full frontmatter + `body_markdown` + `body_html` + new `version`.
Clients use it directly without a follow-up GET. Other error codes do
not include `issue`.

### 4.4 Server-internal call vs CLI shell-out

The server calls `mutate::update_issue(...)` directly. Dogfooding holds
because the CLI uses the same Rust functions: one `UpdateIssueRequest`
type, clap and serde both derive into it. The symbolic value of
"shells out" doesn't justify its 30–80 ms fork+exec cost or its locking
races with the watcher.

### 4.5 CLI parity additions

To uphold "anything the web does is reachable from CLI":

- `issuectl show <slug> --json` and `issuectl list --json` add a
  `version: "sha256:…"` field per issue.
- `issuectl update --json …` **requires** `--expected-version sha256:…`.
  Human invocations without `--json` keep working without it; `flock`
  still prevents corruption.
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
- **Path-shape filter.** Only file events on `<root>/issues/<slug>/item.md`
  (or its legacy `<root>/issues/{open,closed}/<slug>/item.md` form)
  trigger `IssueUpserted`; only directory create/remove of
  `<root>/issues/<slug>` (or legacy) trigger `IssueUpserted` /
  `IssueRemoved` for slug add/remove. Side-doc files
  (`issues/<slug>/docs/foo.md`) and other non-`item.md` content do
  *not* fire issue events — otherwise editing a side doc would
  spam the broadcast channel with phantom upserts.
- Debounce window: 100–200 ms.
- All parse / sanitise / hash work runs in `tokio::task::spawn_blocking`,
  so a `git checkout` of 200 issues doesn't stall the watcher's event
  loop.

### 5.2 Slug resolution from event paths

The slug is the first component under `issues/`. The legacy
`open`/`closed` prefix is still accepted so compat-read repos still
light up the watcher; if a slug resolves to a legacy path, the
broadcast `IssueUpserted` carries a `legacy_layout` `LoadWarning` (or
`ambiguous_slug`, when both canonical and legacy locations exist)
alongside the issue summary. Clients render the warning chip; the
next mutation against that slug migrates it via `locate_and_migrate`
(§3.1.1).

```rust
fn issue_slug_from_event(issues_root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(issues_root).ok()?;
    let mut comps = rel.components();
    let first = comps.next()?.as_os_str().to_str()?;
    let slug = if first == "open" || first == "closed" {
        comps.next()?.as_os_str().to_str()? // legacy compat
    } else {
        first                                 // canonical flat layout
    };
    if !slug::is_valid(slug) { return None; }
    Some(slug.to_owned())
}
```

### 5.3 Slug renames

`git mv issues/old-slug issues/new-slug` produces a rename event with
both paths (the full debouncer preserves them). The watcher emits:

```
IssueRemoved  { slug: "old-slug" }
IssueUpserted { slug: "new-slug", ... }
```

Status changes are ordinary `IssueUpserted` events — there is no
disk-layout move involved. Clients re-bucket from `summary.status` and
the derived `is_closing_status` label.

### 5.4 Transport: SSE

SSE wins for one-way push. axum's `Sse` plus `EventSource` covers
reconnect.

### 5.5 EventHub and replay cursor

One explicit type. **All seq advancement and ring writes happen inside
one critical section** so seq order matches ring order; splitting them
across an `AtomicU64` and a separate mutex creates an out-of-order
publish bug.

```rust
pub struct EventHub {
    inner: parking_lot::Mutex<EventHubInner>,
    tx: broadcast::Sender<BoardEvent>,
    capacity: usize,           // assert > 0; e.g. 1024
    instance_id: Uuid,         // generated at startup; shipped to clients
}

struct EventHubInner {
    next_seq: u64,
    ring: VecDeque<BoardEvent>,
}

#[derive(Clone, Serialize)]
pub struct BoardEvent {
    pub seq: u64,
    #[serde(flatten)] pub payload: EventPayload,
    // Note: no Instant timestamp on the wire. Instant is not Serialize
    // and has no cross-process meaning. If a timestamp is needed for
    // diagnostics, add `chrono::DateTime<Utc>` later.
}

#[derive(Clone, Serialize)]
#[serde(tag = "type")]
pub enum EventPayload {
    IssueUpserted { slug: String, version: String, issue: IssueSummary },
    IssueRemoved  { slug: String },
    IssueInvalid  { slug: String, warnings: Vec<LoadWarning> },
    Resync        { reason: String },
    Degraded      { reason: String },
}

impl EventHub {
    pub fn publish(&self, payload: EventPayload) -> BoardEvent {
        let evt = {
            let mut g = self.inner.lock();
            g.next_seq += 1;
            let evt = BoardEvent { seq: g.next_seq, payload };
            while g.ring.len() >= self.capacity { g.ring.pop_front(); }
            g.ring.push_back(evt.clone());
            evt
        };                       // lock dropped before send
        let _ = self.tx.send(evt.clone());
        evt
    }

    pub fn current_seq(&self) -> u64 { self.inner.lock().next_seq }

    /// Subscribe AND replay in one operation, atomic w.r.t. publish.
    /// The caller must drop live broadcast events with seq <= drop_through
    /// to avoid duplicates that bridge the subscribe→replay window.
    pub fn subscribe_since(&self, since: u64) -> ReplayStream {
        let rx = self.tx.subscribe();          // 1. subscribe FIRST
        let g = self.inner.lock();             // 2. lock the hub
        let current = g.next_seq;
        let replay = if since > current {
            // Future seq → previous server instance, or stale client.
            Replay::TooOld { reason: "future_seq" }
        } else if since == current {
            Replay::Events(vec![])
        } else if g.ring.is_empty() || g.ring.front().unwrap().seq > since + 1 {
            Replay::TooOld { reason: "gap" }
        } else {
            let evts: Vec<_> = g.ring.iter()
                .filter(|e| e.seq > since)
                .cloned().collect();
            Replay::Events(evts)
        };
        let drop_through = match &replay {
            Replay::Events(v) => v.last().map(|e| e.seq).unwrap_or(since),
            Replay::TooOld { .. } => current,
        };
        ReplayStream { replay, rx, drop_through, instance_id: self.instance_id }
    }
}

pub struct ReplayStream {
    pub replay: Replay,
    pub rx: broadcast::Receiver<BoardEvent>,
    pub drop_through: u64,
    pub instance_id: Uuid,
}

pub enum Replay {
    Events(Vec<BoardEvent>),
    TooOld { reason: &'static str },
}
```

Notable invariants:

- `next_seq` and the ring share one mutex. `current_seq()` is a snapshot
  of "the highest seq the ring contains". Because mutate.rs publishes
  inside the repo `flock` (§3.1 step 8), seq order matches the order in
  which mutations land on disk.
- `subscribe_since` subscribes to the broadcast **before** snapshotting
  the ring under the same lock. Any event published after the lock is
  released arrives via `rx`; duplicates with seq ≤ `drop_through` are
  dropped at the SSE handler. Without this ordering, an event published
  between `replay_since` and `subscribe()` would be lost.
- `since > current_seq` returns `TooOld { reason: "future_seq" }`, never
  empty events. After a server restart, `next_seq` resets to 0, so a
  client reconnecting with `Last-Event-ID: 500` gets `TooOld` → full
  resync. `instance_id` (returned in every SSE handshake) lets the
  client detect server restart even when seqs happen to overlap.
- `since` exactly at `current_seq` returns `Events(vec![])` — client is
  caught up, no resync needed.
- Ring boundary: `since < oldest - 1` (strictly less; `since == oldest - 1`
  is replayable because `oldest` is the first event after the gap).
- `parking_lot::Mutex` (sync, not `tokio::sync::Mutex`). The critical
  section never `.await`s. Holding it across send is unnecessary — send
  goes after the lock drops.

#### 5.5.1 Initial-load cursor handoff

```
GET /api/issues
  → snapshot_seq = event_hub.current_seq()         (BEFORE filesystem scan)
  → scan filesystem
  → return { snapshot_seq, instance_id, issues, warnings }

client opens /events?since=<snapshot_seq>&instance=<instance_id>
  → server: ReplayStream = hub.subscribe_since(snapshot_seq)
  → if response.instance_id != client.instance_id → emit Resync first
  → send ring replay events
  → forward broadcast events with seq > drop_through
```

**Invariant supporting the cursor:** every mutation that changes disk
state publishes its event before releasing `flock` (§3.1 step 8). If
event seq=N exists, the filesystem state corresponding to N is already
visible to scans that begin after `current_seq() ≥ N`.

`broadcast::error::RecvError::Lagged(_)` → server emits
`Resync { reason: "lagged" }` to that subscriber and reconnects them
behind the scenes (or the SSE handler drops the connection and lets
`EventSource` reconnect).

For reconnects, the standard `Last-Event-ID` header works on top of the
same mechanism. Browsers can't set `Last-Event-ID` on the *initial*
`EventSource` connection, which is why `?since=` and `?instance=` query
params exist — first connect uses them, subsequent reconnects use
`Last-Event-ID`.

On the very first connection the client has no prior `instance_id`;
it omits the `?instance=` parameter entirely. A missing parameter is
treated as "match anything" — no synthetic `Resync` is prepended. The
client adopts the `instance_id` from the SSE handshake's first
event/handshake frame and uses it on subsequent reconnects.

### 5.6 Event types

The wire envelope is `BoardEvent { seq, payload }` with a `#[serde(tag
= "type")]` payload (see §5.5 and Appendix A).

Notable points:

- **No `body_html` in events.** Detail dialogs refetch
  `/api/issues/{slug}` on relevant events. Bodies are short; fetch is
  cheap. This keeps the broadcast channel single-shape.
- **No `origin` / `WriteToken`.** Echo suppression is client-side
  (§6.4) — server sends the same event to everyone, the originating
  tab recognises its own `version` from the PATCH 200 response.
- **`IssueInvalid`** — for malformed YAML, missing `item.md`,
  `<<<<<<<` merge markers, partially-written files. Reuses
  `repo::LoadWarning` shape; web UI already renders these in the
  `#warnings` strip. Card stays visible with an error badge instead
  of vanishing.
- **Lifecycle.** `IssueInvalid` for slug X is cleared when the next
  successful re-parse of X publishes an `IssueUpserted` (the issue
  is valid again) or the slug is deleted (`IssueRemoved`). Clients
  drop the error badge in either case.

**Client dedup contract** (single rule, applies regardless of event
origin):

- `IssueUpserted { slug, version }` is idempotent on `(slug, version)`.
  A tab that already shows version `V` for slug `S` skips re-render
  on any subsequent `IssueUpserted { slug: S, version: V }` — including
  the synthetic event from a self-originated PATCH and the watcher's
  follow-up event for the same write.
- `IssueRemoved { slug }` is idempotent on `slug`. A tab that already
  shows slug `S` as removed (or never had it) skips re-render. The
  mutator never publishes `IssueRemoved` (it always writes via
  `locate_and_migrate` + `write_item_atomic`); only the watcher does.
  Migration-driven `IssueRemoved` events are suppressed by the rule
  in §3.1.1, so the contract stays slug-only.

### 5.7 Bulk-change coalescing

If more than ~50 distinct slugs change within a single debounce window
(e.g. `git checkout` switching feature branches), the watcher emits one
`Resync { reason: "bulk_change" }` instead of fanning out per-issue
events. Threshold tunable via `--watch-bulk-threshold`.

The `Resync` event carries a `seq` like any other event (allocated
inside the EventHub mutex). On receipt, clients **discard all
per-issue local_version state**, refetch `/api/issues`, capture the new
`snapshot_seq` from that response, and continue consuming the SSE stream
with `seq > snapshot_seq`. The `Resync` itself is the only event a
client treats as "drop everything"; per-issue events between two
`Resync`s remain individually applicable.

### 5.8 Watcher restart

If the watcher task panics, exponential backoff retries up to 3 times.
On each successful (re)start — including the first — the watcher emits
`Resync { reason: "watcher_restart" }`. After 3 failures, the hub emits
`Degraded { reason: "watcher_unavailable" }` (clients show a red dot;
manual refresh button works).

## 6. Concurrency & conflict UX

### 6.1 Lost-update protection

Two layers, in order of strength:

1. **`flock(LOCK_EX)`** on `<root>/.issuectl/write.lock` — cross-process,
   shared by CLI and server. The only mechanism that survives "user runs
   `issuectl update` from terminal while server is up." Note: does not
   protect against `$EDITOR` saves or `git pull` — those are outside the
   tool's control and documented as such.
2. **`expected_version` optimistic check** — surfaces conflicts to the
   browser tab that started its edit before another writer changed the
   file. Required on AI-agent (`--json`) calls.

Layer 1 prevents corruption; layer 2 surfaces *user-visible* conflicts.
Removing layer 2 would silently overwrite changes; removing layer 1
would corrupt the file. There is no in-process per-slug mutex: `flock`
already serialises everything and a second async lock would only add
acquisition order risk for no concurrency win (see §2).

### 6.2 Status authority

`fm.status` is the single source of truth. The bucket label
(`open` / `closed`) on API payloads is derived: `is_closing_status(fm.status)`.

### 6.3 Body conflict UX (M2)

When `PUT /body` returns 409:

1. The server's fresh `IssueDetailResponse` is shown alongside the user's
   current draft (split pane).
2. The textarea **is not overwritten**. The user's typed content stays
   exactly where they put it. Silently wiping a body into an "in-memory
   clipboard" is data-loss UX.
3. `localStorage` already has a backup keyed by
   `(slug, started_editing_at)` — written every keystroke (free,
   synchronous, survives tab-close, crashes, network failure).
4. User picks: "keep mine" (PATCH again with new `expected_version`),
   "keep theirs" (discard local, accept server), or manually merge in
   the textarea then save.

For **metadata** PATCHes (status, priority, etc.), the simpler
"toast + reload" UX is fine — there's no draft to lose.

### 6.4 Echo suppression

Server-side write-token / request-id machinery is deliberately not
used; the dedup contract in §5.6 is symmetric across tabs and origins.
Concretely:

1. Every tab tracks `local_version[slug]` — the version it currently
   shows for each slug, regardless of how that version arrived (REST
   GET, SSE, own PATCH 200).
2. When an `IssueUpserted { slug, version }` arrives, if
   `version == local_version[slug]` the tab silently reconciles
   (updates "Saved" indicator if appropriate, no re-render flash).
   If different, full re-render and `local_version[slug] = version`.
3. The PATCH 200 response carries the new `version`, which the
   originating tab also writes into `local_version[slug]`.

**SSE-vs-PATCH-200 race.** The SSE event and PATCH 200 travel over
independent TCP connections; either can arrive first at the
originating tab. If SSE wins, the naive tab would see
`version != local_version[slug]` (still pre-PATCH) and full-rerender,
potentially flashing the user's just-clicked card. The originating
tab MUST therefore **buffer SSE events for slug `S` while a PATCH
request for `S` is in flight from this tab**, draining the buffer
when the PATCH completes (success or failure). Combined with the
dedup rule in step 2 and the active-textarea-preservation rule
below, this closes the data-loss path: the SSE event cannot replace
the textarea or full-rerender behind a click while the user's own
write is mid-flight.

**Active-textarea preservation.** A re-render driven by an
`IssueUpserted` (regardless of origin) MUST NOT overwrite the
contents of an open `<textarea>` for the same slug. Metadata
re-renders (status drag, label chips, etc.) update freely; the body
textarea has its own state machine — its content is only replaced
when the user explicitly accepts a server version in the conflict UX
(§6.3). Combined with `localStorage` per-keystroke (§6.5), there is
no path that loses an in-progress draft.

Other tabs (same user, different tab) treat the event as external
(`version != local_version[slug]`) — correct, because from their
perspective it *is* external — and re-render under the same
textarea-preservation rule. No server state required.

### 6.5 Body autosave (M2)

- 5 s debounce after last keystroke.
- Save on `blur`.
- `localStorage` written on every keystroke (independent of network).
- Manual `Ctrl+S` / save button always available.
- No save on `tab-hide` / `pagehide` — `sendBeacon` /
  `fetch(keepalive: true)` are too unreliable to count on, and
  `localStorage` already covers crash/close.

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
never half. Status changes are just `write_item_atomic` (§3.4) — no
rename, no half-state.

The one rename in the mutation path is the legacy→canonical directory
move performed by `locate_and_migrate` (§3.1.1). `rename(2)` is
atomic, so the on-disk slug is unambiguously at one path or the other
at any instant. A crash *between* the rename and the subsequent
`write_item_atomic` leaves the slug at the canonical path with its
pre-write content — a perfectly valid state, indistinguishable from
"a migration ran but no edit followed." No reconciliation is required.

Orphan tempfiles (`.issuectl-tmp-*`) left behind when `Drop` was
skipped (e.g. SIGKILL) are swept on next startup (`doctor` extension).

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

**Read-side boundary.** Any local process that can connect to
`127.0.0.1:<port>` can read all issues, regardless of the running
uid's filesystem permissions on the repo. Loopback bind does not
enforce uid separation: CSRF / `SameSite` cookies protect the
*browser context* (cross-origin / cross-site requests from a real
browser), not native local processes (`curl`, scripts) that can
read `Set-Cookie` and replay cookies at will. This is in scope as a
documented limitation; running `serve` on a uid that the user does
not trust is unsupported.

### 9.2 Mechanism

Per-process CSRF token:

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
  carrying the same per-process random token used for the CSRF
  header — there is no separate session store. The cookie's value is
  validated by string-comparing against `AppState.csrf_token`.
- `/events` requires the cookie; rejects without it. `Host`
  validation applies.
- **Server restart invalidates the cookie** because `csrf_token` is
  regenerated. `/events` returns 401; `EventSource` treats this as
  an error, the client falls back to `GET /api/session` to refresh
  the token+cookie pair, then re-opens `/events`. The client SHOULD
  catch the 401 explicitly rather than relying on `EventSource`'s
  default reconnect loop.
- The HTML shell unconditionally hits `GET /api/session` on page
  load before any other API call, so the cookie is set before the
  first `/events` connect.

The cookie and the `X-Issuectl-CSRF` header serve different
transport purposes (SSE vs state-change) but carry the same per-process
secret — their lifetimes are tied.

### 9.4 Non-loopback bind

When `--host` is non-loopback:

- All write routes return 403 unless `--allow-remote-writes` is also
  passed.
- `--allow-remote-writes` requires `--auth-token-file <path>` (this
  *is* a persisted token for remote use; loopback per-process model
  doesn't fit). If neither is set, `serve` refuses to start.

This is a future feature; M0–M3 stays loopback-only for writes.

### 9.5 Other hardening

- Request size: a single global 1 MiB envelope on every route. PATCH
  metadata payloads in practice sit well under 64 KiB; one global
  limit cuts the layer wiring complexity. If a future route needs a
  tighter cap (e.g. an audit-log endpoint), apply
  `route_layer(DefaultBodyLimit)` there.
- The atomic-write target path is canonicalised once at
  `locate_and_migrate` time (§3.1.1) — `symlink_metadata` +
  canonical-prefix check. The path is *not* re-checked between
  `locate_and_migrate` and `tf.persist(target)`; an attacker who can
  swap the directory under us during the gap can defeat the check.
  This is intentional: `flock` is advisory and does not exclude
  non-`flock`-holding processes, so an additional check at persist
  time would not provide real defence either. The threat model is
  local-trusted filesystem; harden via `*at` syscalls
  (`openat(O_NOFOLLOW)`, `renameat2`) only if the threat model
  changes.
- Watcher does not follow symlinks (`with_follow_symlinks(false)`).
- `NamedTempFile` placement inside the watched dir is fine because
  the tempfile prefix is filtered (§5.1).

## 10. Phasing

| Phase | Scope | Ship value |
| --- | --- | --- |
| **M0** | `EventHub` (single-mutex seq+ring), `notify-debouncer-full` watcher, `/events` SSE with `subscribe_since` race-free handoff, `snapshot_seq` + `instance_id` in `/api/issues`. No writes. | Live read-side updates: `$EDITOR` saves, `git pull`, agent edits all show up in the board immediately. |
| **M1** | `mutate.rs` refactor with `Patch<T>` + `UpdateIssueRequest` + `flock` + canonical hash + `locate_and_migrate` (flat-layout writes with legacy compat reads). `issuectl doctor migrate-layout` for one-shot bulk migration of legacy repos (ships alongside per-write migration so users have a clean recovery path for `ambiguous_slug`). CSRF token + `Host` validation. PATCH metadata routes. CLI: `--expected-version`, `version` field in `--json` output. Drag-to-move in UI. | Status drag, label/assignee edits, deterministic legacy-layout migration. |
| **M2** | `PUT /body` + textarea + `POST /api/preview` + `localStorage` draft + body conflict UX. `IssueInvalid` event surfacing. CLI: `issuectl body set`. | Full edit-in-place. |
| **M3** | `--watch-poll-ms`, `--no-watch`, `Degraded` banner, three-way merge UI for body conflicts. | Robustness for real multi-client use. |

**Spin-offs** (own issues, off the M0–M3 path):

- Startup reconciliation (extends `issuectl doctor`) — `item.md`-missing
  / merge-marker / orphan-epic / orphan-tempfile cleanup.
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
| Body autosave | None / 750 ms / 5 s + localStorage | **5 s + localStorage + manual** | Covers tab-close + avoids 409 storms |

## 12. Out of scope / open questions

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

The wire format is a `BoardEvent` envelope wrapping a tagged
`EventPayload`. `seq` lives on the envelope so every event has a
uniform `seq` accessor (avoids duplicating it across each variant).

```rust
// pseudocode — final shapes belong in events.rs

#[derive(Clone, Serialize)]
pub struct BoardEvent {
    pub seq: u64,
    #[serde(flatten)]
    pub payload: EventPayload,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type")]
pub enum EventPayload {
    IssueUpserted { slug: String, version: String, issue: IssueSummary },
    IssueRemoved  { slug: String },
    IssueInvalid  { slug: String, warnings: Vec<LoadWarning> },
    Resync        { reason: String },
    Degraded      { reason: String },
}
```

JSON wire shape (with `#[serde(flatten)]`):

```json
{ "seq": 42, "type": "IssueUpserted",
  "slug": "foo", "version": "sha256:…", "issue": { ... } }
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
  |                                  |  locate_and_migrate(foo)
  |                                  |  read item.md, hash
  |                                  |  check expected_version
  |                                  |  apply mutation in memory
  |                                  |  write_item_atomic
  |                                  |  recompute hash → V_new
  |                                  |  hub.publish(IssueUpserted{V_new})
  |                                  |    — STILL holding flock so seq
  |                                  |      order matches disk order
  |                                  |  release flock
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

## Appendix C — design history

This document went through two multi-LLM panel review passes against the
original `open`/`closed`-folder-split layout, plus a third pass after
the flat-layout migration. Findings synthesized in
`history/review-web-edit-sync.md` and `history/review-flat-layout.md`.

Pass 2 corrections that shaped §3 and §5.5: in-lock seq advancement
(§5.5), publish-before-flock-release (§3.1 step 8), `subscribe_since`
race-free handoff (§5.5), `instance_id` for server-restart detection,
algorithmic canonical-hash spec (§3.2), CRLF-aware body normalisation,
removal of the per-slug async mutex (§2 / §6.1), tightened `Patch<T>`
serde semantics (§3.5), and dropping the false `flock`-as-symlink-defence
claim (§9.5).

The flat-layout migration (issue `awfully-faint-sound`) retired
`issues/{open,closed}/<slug>/`. Sections that moved or collapsed:

- §3.4 was "status change = directory rename"; now a frontmatter PATCH.
- §3.1.1 was `locate_issue` over two folder roots; now `locate_and_migrate`.
- §5.2 / §5.3 lost the folder-move event class.
- §6.2 lost the "directory wins, frontmatter rewritten to match" rule.
- The startup reconciler's surface shrank — no folder/frontmatter
  mismatch class to repair.

Resolved discussion items (numbered against the original review
threads):

- **D3 (body autosave)** = **C**: 5 s debounce + `localStorage` + manual save.
- **D4 (CLI `--expected-version`)** = **B**: required when `--json`,
  optional otherwise. `flock` covers corruption either way.
- **D5 (PATCH array semantics)** = **A**: keep `add_*`/`remove_*` for
  CLI parity; specify edge cases (duplicate add+remove → 400; absent
  remove → no-op).
- **#17 (CSRF token persistence)** = **A**: per-process. Non-loopback
  uses a separate persistent `--auth-token-file`.
- **#19 (status/folder authority)** = originally **A** (directory wins
  + reconciler rewrites frontmatter). **Superseded** by the flat-layout
  migration: there is no folder axis to be authoritative over.

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
- **#29** Windows parent-directory `fsync`: defined as a no-op
  (gated `#[cfg(unix)]`, see §3.3). Atomic rename and tempfile
  cleanup work on Windows; only post-rename parent-directory
  durability under power loss is missing. Acceptable for issue
  metadata.
