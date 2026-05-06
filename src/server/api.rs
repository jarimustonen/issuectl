use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

/// Distinct failure modes for the docs endpoint. Mapping to status codes
/// from a single match keeps "I/O error", "doc missing", and "you tried to
/// escape the issue directory" from collapsing into one undifferentiated
/// 404 — which made debugging traversal issues painful.
enum DocError {
    NotFound,
    Forbidden,
    Internal,
}

impl From<DocError> for StatusCode {
    fn from(e: DocError) -> Self {
        match e {
            DocError::NotFound => StatusCode::NOT_FOUND,
            DocError::Forbidden => StatusCode::FORBIDDEN,
            DocError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

use crate::mutate::{self, MutateError, NewIssueRequest, UpdateIssueRequest};
use crate::repo::{self, IssueSummary, LoadWarning};
use crate::slug;

use super::events::{BoardEvent, EventPayload, Replay};
use super::render::sanitize_markdown;
use super::AppState;

#[derive(Serialize)]
pub struct IssueListResponse {
    pub issues: Vec<IssueSummary>,
    /// Per-file parse warnings (e.g., malformed YAML, missing item.md).
    /// Empty when nothing is wrong; UI can flag broken issues from this list.
    pub warnings: Vec<LoadWarning>,
    /// Highest event seq observed *before* this scan ran. Clients connect
    /// to `/events?since=<snapshot_seq>` to fill the gap between scan
    /// completion and live-stream subscription. See design doc §5.5.1.
    pub snapshot_seq: u64,
    /// Server-instance UUID; flips on every `serve` restart so a client
    /// reconnecting with a stale `since` from a prior process can detect
    /// it instead of skipping events.
    pub instance_id: Uuid,
}

#[derive(Serialize)]
pub struct IssueDetailResponse {
    #[serde(flatten)]
    pub issue: crate::models::Issue,
    pub body_html: String,
    /// Slugs of additional `*.md` files in the issue directory (excluding
    /// `item.md`). Fetched on demand via `/api/issues/<slug>/docs/<name>`.
    pub docs: Vec<String>,
}

pub async fn list_issues(
    State(state): State<AppState>,
) -> Result<Json<IssueListResponse>, StatusCode> {
    // Snapshot BEFORE scan so any event published while the scan runs is
    // captured by `/events?since=snapshot_seq` rather than lost. The
    // mutation protocol (M1, §3.1 step 8) guarantees seq=N's disk state
    // is visible to scans that begin after current_seq() ≥ N.
    let snapshot_seq = state.event_hub.current_seq();
    let instance_id = state.event_hub.instance_id();
    let root = state.root.clone();
    let (issues, warnings) =
        tokio::task::spawn_blocking(move || repo::load_issue_summaries(root.as_path()))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(IssueListResponse {
        issues,
        warnings,
        snapshot_seq,
        instance_id,
    }))
}

pub async fn get_issue(
    State(state): State<AppState>,
    Path(slug_param): Path<String>,
) -> Result<Json<IssueDetailResponse>, StatusCode> {
    if !slug::is_valid(&slug_param) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let root = state.root.clone();
    let slug_for_load = slug_param.clone();
    let (issue, docs) = tokio::task::spawn_blocking(move || {
        let issue = repo::load_issue(root.as_path(), &slug_for_load)?;
        let dir = root.join("issues").join(&issue.folder).join(&issue.slug);
        let docs = list_extra_docs(&dir);
        anyhow::Ok((issue, docs))
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::NOT_FOUND)?;

    let body_html = sanitize_markdown(&issue.body);
    Ok(Json(IssueDetailResponse {
        issue,
        body_html,
        docs,
    }))
}

#[derive(Serialize)]
pub struct DocResponse {
    pub name: String,
    pub body_html: String,
}

pub async fn get_doc(
    State(state): State<AppState>,
    Path((slug_param, doc_name)): Path<(String, String)>,
) -> Result<Json<DocResponse>, StatusCode> {
    if !slug::is_valid(&slug_param) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if !is_safe_doc_name(&doc_name) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let root = state.root.clone();
    let slug_owned = slug_param.clone();
    let doc_owned = doc_name.clone();
    let body = tokio::task::spawn_blocking(move || -> Result<String, DocError> {
        let (folder, _item) =
            repo::locate_issue(root.as_path(), &slug_owned).map_err(|_| DocError::NotFound)?;
        let path = root
            .join("issues")
            .join(&folder)
            .join(&slug_owned)
            .join(&doc_owned);
        // Rebuilt-from-validated-segments path cannot escape the issue dir
        // by string operations alone, but a symlink inside the dir could
        // still point outward. Canonicalize both sides and require the doc
        // to stay under the canonical issue directory.
        let canon = std::fs::canonicalize(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => DocError::NotFound,
            _ => DocError::Internal,
        })?;
        let issue_dir = std::fs::canonicalize(root.join("issues").join(&folder).join(&slug_owned))
            .map_err(|_| DocError::Internal)?;
        if !canon.starts_with(&issue_dir) {
            return Err(DocError::Forbidden);
        }
        // Read via the canonical path — `path` would re-resolve symlinks
        // that may have been swapped between canonicalize() and read().
        let meta = std::fs::metadata(&canon).map_err(|_| DocError::NotFound)?;
        if !meta.is_file() {
            return Err(DocError::NotFound);
        }
        std::fs::read_to_string(&canon).map_err(|_| DocError::Internal)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(StatusCode::from)?;

    Ok(Json(DocResponse {
        name: doc_name,
        body_html: sanitize_markdown(&body),
    }))
}

/// List `*.md` files in an issue directory, excluding `item.md`. Files are
/// returned by basename (no path separators), suitable for building
/// `/api/issues/<slug>/docs/<name>` URLs.
fn list_extra_docs(dir: &std::path::Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "item.md" || !name.ends_with(".md") {
            continue;
        }
        if !is_safe_doc_name(&name) {
            continue;
        }
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            out.push(name);
        }
    }
    out.sort();
    out
}

/// Doc filename safety: bare basename ending in `.md`, ASCII letters/digits
/// plus `-`, `_`, `.`. Blocks path separators, parent refs, and hidden files.
///
/// SECURITY: the explicit `..` check is load-bearing — `a..b.md` and `....md`
/// would otherwise pass the char-class predicate (since `.` is allowed).
/// Don't drop it as a "redundant" cleanup. Likewise the `starts_with('.')`
/// rule blocks dotfiles like `.git.md` and the empty/`.`/`..` literal cases.
pub(super) fn is_safe_doc_name(name: &str) -> bool {
    if name.is_empty() || name.starts_with('.') {
        return false;
    }
    if !name.ends_with(".md") {
        return false;
    }
    if name.contains("..") {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[derive(Deserialize)]
pub struct EventsQuery {
    /// Last seq the client already has. Omitted means "stream from now"
    /// (no replay). When `Last-Event-ID` is present on a reconnect the
    /// header takes precedence over this query parameter.
    #[serde(default)]
    pub since: Option<u64>,
    /// Server `instance_id` the client believes it's still talking to.
    /// If it differs from the current instance, the stream opens with a
    /// single `Resync { reason: "instance_changed" }` and forwards only
    /// new live events; no stale replay from a prior process is sent.
    #[serde(default)]
    pub instance: Option<Uuid>,
}

/// Read `Last-Event-ID`. Empty values are ignored (per SSE spec, an
/// empty header means the client has no remembered cursor — likely the
/// initial connect). Non-numeric values are also ignored rather than
/// erroring; the worst case is the client gets streamed from "now".
fn parse_last_event_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<u64>().ok())
}

/// Stream board events as Server-Sent Events.
///
/// Reconnect cursor resolution order: `Last-Event-ID` header > `?since=`
/// query > "from now" (current_seq).
///
/// On instance mismatch the stream emits one `Resync { instance_changed
/// }` and forwards only events the new instance produces — no stale
/// replay from a prior process. On `Lagged` the stream emits `Resync {
/// lagged }` and ends, prompting `EventSource` to reconnect cleanly.
/// See design doc §5.5.
pub async fn events_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let server_instance = state.event_hub.instance_id();
    let instance_mismatch = q.instance.is_some_and(|id| id != server_instance);

    // F3: omitted/0 since → stream-from-now. Otherwise replay.
    // F1: Last-Event-ID overrides ?since= on reconnect.
    let since = parse_last_event_id(&headers)
        .or(q.since)
        .filter(|&v| v != 0)
        .unwrap_or_else(|| state.event_hub.current_seq());

    // D2: instance mismatch — short-circuit. Old cursor is meaningless
    // in this process; subscribe at current_seq, send only the Resync,
    // and forward only live events from this point onward.
    let (mut prefix, stream_handle): (Vec<std::sync::Arc<BoardEvent>>, _) = if instance_mismatch {
        let handle = state
            .event_hub
            .subscribe_since(state.event_hub.current_seq());
        let prefix = vec![std::sync::Arc::new(BoardEvent {
            seq: 0,
            payload: EventPayload::Resync {
                reason: "instance_changed".to_string(),
            },
        })];
        (prefix, handle)
    } else {
        let handle = state.event_hub.subscribe_since(since);
        let mut prefix = Vec::new();
        match &handle.replay {
            Replay::Events(v) => prefix.extend(v.iter().cloned()),
            Replay::TooOld { reason } => prefix.push(std::sync::Arc::new(BoardEvent {
                seq: 0,
                payload: EventPayload::Resync {
                    reason: reason.to_string(),
                },
            })),
        }
        (prefix, handle)
    };

    let drop_through = stream_handle.drop_through;

    // F14: after Lagged, terminate the stream so EventSource reconnects.
    // `take_while` ends as soon as we yield the synthetic Resync.
    let live = BroadcastStream::new(stream_handle.rx)
        .scan(false, move |ended, res| {
            let item = if *ended {
                None
            } else {
                match res {
                    Ok(evt) if evt.seq > drop_through => Some(Some(evt)),
                    Ok(_) => Some(None), // duplicate covered by replay
                    Err(_lag) => {
                        *ended = true;
                        Some(Some(std::sync::Arc::new(BoardEvent {
                            seq: 0,
                            payload: EventPayload::Resync {
                                reason: "lagged".to_string(),
                            },
                        })))
                    }
                }
            };
            std::future::ready(item)
        })
        .filter_map(|opt| async move { opt });

    // Drain the prefix vec into a stream once.
    let prefix_stream = futures_util::stream::iter(std::mem::take(&mut prefix));
    let combined = prefix_stream
        .chain(live)
        .map(|evt: std::sync::Arc<BoardEvent>| {
            let mut event = Event::default()
                .data(serde_json::to_string(&*evt).expect("BoardEvent serialization cannot fail"));
            // F2: only attach `id:` for real events. Empty `id:` per SSE
            // spec sets lastEventId to empty, breaking reconnect cursor.
            if evt.seq != 0 {
                event = event.id(evt.seq.to_string());
            }
            Ok::<Event, Infallible>(event)
        });

    Sse::new(combined).keep_alive(
        // 15 s is well under typical reverse-proxy/browser idle timeouts
        // (30–60 s) so loopback users with no events pending still see
        // the connection stay alive.
        KeepAlive::new().interval(Duration::from_secs(15)),
    )
}

// ── M1: write endpoints ────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SessionResponse {
    pub csrf_token: String,
    pub instance_id: Uuid,
}

/// Bootstrap endpoint. Returns the per-process CSRF token plus the
/// server `instance_id`. The HTML shell loads this once on page load
/// and stashes the token in memory; subsequent state-changing requests
/// echo it back in `X-Issuectl-CSRF`.
///
/// SSE auth: `/events` is gated by Host-only on the loopback threat
/// model — same machine, same user, no realistic attacker. The
/// design's earlier "cookie auth on /events" plan was scrapped after
/// the user confirmed the trust boundary. No cookie is set.
pub async fn session(State(state): State<super::AppState>) -> Response {
    Json(SessionResponse {
        csrf_token: state.csrf_token.to_string(),
        instance_id: state.event_hub.instance_id(),
    })
    .into_response()
}

#[derive(Serialize)]
pub struct UpdateIssueResponse {
    pub slug: String,
    pub version: String,
    pub final_dir: String,
    pub moved_to_closed: bool,
    pub moved_to_open: bool,
    pub issue: crate::models::Issue,
}

#[derive(Serialize)]
pub struct CreateIssueResponse {
    pub slug: String,
    pub version: String,
    pub final_dir: String,
    pub issue: crate::models::Issue,
}

pub async fn patch_issue(
    State(state): State<super::AppState>,
    Path(slug_param): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if !slug::is_valid(&slug_param) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "validation",
            "invalid slug shape",
        );
    }
    let req: UpdateIssueRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "validation",
                &format!("invalid JSON body: {e}"),
            );
        }
    };
    let root = state.root.clone();
    let hub = state.event_hub.clone();
    let slug_owned = slug_param.clone();
    let result = tokio::task::spawn_blocking(move || {
        mutate::update_issue(root.as_path(), &slug_owned, req, Some(&hub))
    })
    .await;
    match result {
        Ok(Ok(out)) => Json(UpdateIssueResponse {
            slug: slug_param,
            version: out.version,
            final_dir: out.issue_dir.to_string_lossy().into_owned(),
            moved_to_closed: out.moved_to_closed,
            moved_to_open: out.moved_to_open,
            issue: out.issue,
        })
        .into_response(),
        Ok(Err(err)) => mutate_error_to_response(err),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "task panicked",
        ),
    }
}

pub async fn create_issue(
    State(state): State<super::AppState>,
    body: axum::body::Bytes,
) -> Response {
    let req: NewIssueRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "validation",
                &format!("invalid JSON body: {e}"),
            );
        }
    };
    let root = state.root.clone();
    let hub = state.event_hub.clone();
    let result = tokio::task::spawn_blocking(move || {
        mutate::new_issue(root.as_path(), req, Some(&hub))
    })
    .await;
    match result {
        Ok(Ok(out)) => {
            let slug = out.issue.slug.clone();
            let mut resp = Json(CreateIssueResponse {
                slug,
                version: out.version,
                final_dir: out.issue_dir.to_string_lossy().into_owned(),
                issue: out.issue,
            })
            .into_response();
            *resp.status_mut() = StatusCode::CREATED;
            resp
        }
        Ok(Err(err)) => mutate_error_to_response(err),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "task panicked",
        ),
    }
}

fn mutate_error_to_response(err: MutateError) -> Response {
    match err {
        MutateError::NotFound => {
            error_response(StatusCode::NOT_FOUND, "not_found", "issue not found")
        }
        MutateError::AmbiguousSlug => error_response(
            StatusCode::CONFLICT,
            "ambiguous_slug",
            "slug exists in both open/ and closed/ — resolve manually",
        ),
        MutateError::VersionMismatch { current, version } => {
            // 409 with the full current issue so the client can
            // refresh without an extra GET roundtrip (§4.3).
            let body = serde_json::json!({
                "type": "https://issuectl/errors/version_mismatch",
                "title": "Version mismatch",
                "status": 409,
                "code": "version_mismatch",
                "detail": format!("expected version did not match current: {version}"),
                "issue": current,
                "version": version,
            });
            (StatusCode::CONFLICT, Json(body)).into_response()
        }
        MutateError::Corrupt { warnings } => {
            // 422: the on-disk file has parser warnings. Refusing
            // here protects the user from overwriting recovered
            // defaults with garbage.
            let body = serde_json::json!({
                "type": "https://issuectl/errors/corrupt",
                "title": "Corrupt issue",
                "status": 422,
                "code": "corrupt",
                "detail": "issue file has parse warnings — fix on disk before mutating",
                "warnings": warnings,
            });
            (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
        }
        MutateError::Validation(msg) => {
            error_response(StatusCode::BAD_REQUEST, "validation", &msg)
        }
        MutateError::ConflictingIntent(msg) => {
            error_response(StatusCode::BAD_REQUEST, "conflicting_intent", &msg)
        }
        MutateError::Io(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &format!("{e}"),
        ),
    }
}

fn error_response(status: StatusCode, code: &str, detail: &str) -> Response {
    let body = serde_json::json!({
        "type": format!("https://issuectl/errors/{code}"),
        "title": code.replace('_', " "),
        "status": status.as_u16(),
        "code": code,
        "detail": detail,
    });
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::is_safe_doc_name;

    #[test]
    fn doc_name_accepts_ordinary_md_files() {
        for name in [
            "analysis.md",
            "design-v2.md",
            "foo_bar.md",
            "notes_2026.md",
            "a-b-c.md",
            "x.md",
        ] {
            assert!(is_safe_doc_name(name), "{name} should be accepted");
        }
    }

    #[test]
    fn doc_name_rejects_dangerous_patterns() {
        for name in [
            "",
            ".",
            "..",
            ".hidden.md",
            "..hidden.md",
            "../item.md",
            "..\\item.md",
            "a/b.md",
            "a\\b.md",
            "a..b.md",
            "....md",
            "notes.txt",
            "notes",
            "é.md",        // non-ASCII
            "file.md:zip", // alternate stream / colon
            "name with space.md",
        ] {
            assert!(!is_safe_doc_name(name), "{name} should be rejected");
        }
    }
}
