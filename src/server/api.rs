use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
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
    /// Last seq the client already has. `0` (or omitted) means "stream
    /// from now"; any other value triggers replay from the EventHub
    /// ring. Out-of-range values yield `Resync { reason: "future_seq"
    /// | "gap" }` instead of silent skip.
    #[serde(default)]
    pub since: Option<u64>,
    /// Server `instance_id` the client believes it's still talking to.
    /// If it differs from the current instance, the stream opens with a
    /// `Resync { reason: "instance_changed" }` so the client invalidates
    /// its cache.
    #[serde(default)]
    pub instance: Option<Uuid>,
}

/// Stream board events as Server-Sent Events.
///
/// The first frame is always a `Resync` if the client's cursor is too
/// old or its `instance_id` doesn't match this process; otherwise the
/// stream opens with replayed ring events (closing the gap between
/// `/api/issues` snapshot and now), then forwards live events with
/// `seq > drop_through` to suppress the duplicate of the last replay
/// frame. See design doc §5.5.
pub async fn events_stream(
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let since = q.since.unwrap_or(0);
    let stream_handle = state.event_hub.subscribe_since(since);

    // Build the prefix: a synthetic `instance_changed` Resync if needed,
    // then either the replay events or a `TooOld` Resync.
    let mut prefix: Vec<BoardEvent> = Vec::new();
    let drop_through = stream_handle.drop_through;
    let server_instance = stream_handle.instance_id;

    if let Some(client_instance) = q.instance {
        if client_instance != server_instance {
            // Synthetic event with seq=0 so it never collides with real
            // ones — clients treat any Resync as "drop everything".
            prefix.push(BoardEvent {
                seq: 0,
                payload: EventPayload::Resync {
                    reason: "instance_changed".to_string(),
                },
            });
        }
    }
    match stream_handle.replay {
        Replay::Events(v) => prefix.extend(v),
        Replay::TooOld { reason } => prefix.push(BoardEvent {
            seq: 0,
            payload: EventPayload::Resync {
                reason: reason.to_string(),
            },
        }),
    }

    let live = BroadcastStream::new(stream_handle.rx).filter_map(move |res| async move {
        match res {
            Ok(evt) if evt.seq > drop_through => Some(evt),
            Ok(_) => None, // duplicate already covered by replay
            Err(_lag) => Some(BoardEvent {
                seq: 0,
                payload: EventPayload::Resync {
                    reason: "lagged".to_string(),
                },
            }),
        }
    });

    let prefix_stream = futures_util::stream::iter(prefix);
    let combined = prefix_stream.chain(live).map(|evt: BoardEvent| {
        let id = if evt.seq == 0 {
            String::new()
        } else {
            evt.seq.to_string()
        };
        let json = serde_json::to_string(&evt).unwrap_or_else(|_| "{}".to_string());
        Ok::<Event, Infallible>(Event::default().id(id).data(json))
    });

    Sse::new(combined).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
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
