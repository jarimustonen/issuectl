//! HTTP handlers for user-defined boards (`/board/<name>` and the
//! `/api/boards*` JSON endpoints). The wire shape and routing decisions
//! are documented in `docs/design/custom-boards.md`. Mutations reuse
//! `PATCH /api/issues/<slug>` via `mutate::update_issue` — there is no
//! board-specific write path.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    Json,
};
use serde::Serialize;
use uuid::Uuid;

use crate::boards::{self, BoardError};
use crate::repo::{self, IssueSummary, LoadWarning};

use super::AppState;

#[derive(Serialize)]
pub struct BoardListResponse {
    pub boards: Vec<String>,
}

#[derive(Serialize)]
pub struct BoardColumnDto {
    pub value: String,
    pub label: String,
}

#[derive(Serialize)]
pub struct BoardIssueDto {
    #[serde(flatten)]
    pub summary: IssueSummary,
    /// Server-resolved value of the board's `group_by` field for this
    /// issue. Empty string is the "unassigned" bucket key. Computed
    /// here so the JS can stay agnostic to whether `group_by` points
    /// at a built-in slot or a custom-frontmatter key.
    pub group_value: String,
}

#[derive(Serialize)]
pub struct BoardResponse {
    pub name: String,
    pub group_by: String,
    pub columns: Vec<BoardColumnDto>,
    pub filter: Option<String>,
    pub issues: Vec<BoardIssueDto>,
    pub warnings: Vec<LoadWarning>,
    pub snapshot_seq: u64,
    pub instance_id: Uuid,
    /// True when the board renders but drag is disabled. The matching
    /// `read_only_reason` carries the user-facing message for the
    /// banner (missing schema field or unparseable filter).
    pub read_only: bool,
    pub read_only_reason: Option<String>,
    /// True when the group_by maps to a dedicated `UpdateIssueRequest`
    /// slot. JS uses this to choose between the dedicated PATCH shape
    /// (`{<field>: ...}`) and the `custom_fields` map.
    pub builtin_group_by: bool,
}

pub async fn list_boards_api(State(state): State<AppState>) -> Response {
    let root = state.root.clone();
    let names = tokio::task::spawn_blocking(move || crate::boards::list(root.as_path())).await;
    match names {
        Ok(names) => Json(BoardListResponse { boards: names }).into_response(),
        Err(_) => super::api::error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "task panicked",
        ),
    }
}

pub async fn get_board_api(State(state): State<AppState>, Path(name): Path<String>) -> Response {
    let root = state.root.clone();
    let cache = state.config.clone();
    let board_name = name.clone();
    let snapshot_seq = state.event_hub.current_seq();
    let instance_id = state.event_hub.instance_id();

    let result = tokio::task::spawn_blocking(move || {
        let _g = crate::repo_config::enter(cache);
        let board = boards::load(root.as_path(), &board_name)?;

        // Filter parses guaranteed-OK here for non-read-only boards;
        // for read-only-due-to-bad-filter we just skip filtering.
        let parsed_filter = match (&board.filter, board.read_only_reason.as_deref()) {
            (Some(f), reason) if reason.map(|r| r.contains("filter")).unwrap_or(false) => {
                let _ = f;
                None
            }
            (Some(f), _) => match crate::query::parse(f) {
                Ok(q) => Some(q),
                Err(_) => None,
            },
            (None, _) => None,
        };

        let (full_issues, warnings) = repo::load_issues_with_warnings(root.as_path());
        let issues: Vec<BoardIssueDto> = full_issues
            .into_iter()
            .filter(|i| match &parsed_filter {
                Some(q) => crate::query::matches(q, i),
                None => true,
            })
            .map(|i| {
                let group_value = boards::group_value_for(&i, &board.group_by);
                BoardIssueDto {
                    summary: IssueSummary::from(i),
                    group_value,
                }
            })
            .collect();

        Ok::<_, BoardError>((board, issues, warnings))
    })
    .await;

    match result {
        Ok(Ok((board, issues, warnings))) => {
            let read_only = board.read_only_reason.is_some();
            let builtin = boards::is_builtin_group_by(&board.group_by);
            Json(BoardResponse {
                name: board.name,
                builtin_group_by: builtin,
                group_by: board.group_by,
                columns: board
                    .columns
                    .into_iter()
                    .map(|c| BoardColumnDto {
                        value: c.value,
                        label: c.label,
                    })
                    .collect(),
                filter: board.filter,
                issues,
                warnings,
                snapshot_seq,
                instance_id,
                read_only,
                read_only_reason: board.read_only_reason,
            })
            .into_response()
        }
        Ok(Err(BoardError::NotFound)) => {
            super::api::error_response(StatusCode::NOT_FOUND, "not_found", "board not found")
        }
        Ok(Err(BoardError::Validation(s))) => {
            super::api::error_response(StatusCode::NOT_FOUND, "invalid_board", &s)
        }
        Ok(Err(BoardError::Io(e))) => {
            super::api::error_response(StatusCode::INTERNAL_SERVER_ERROR, "io", &e.to_string())
        }
        Err(_) => super::api::error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "task panicked",
        ),
    }
}

/// `GET /board/<name>` — same shell as the default board, but with
/// `<body data-board-name="...">` so `board.js` knows to fetch
/// `/api/boards/<name>` instead of `/api/issues`.
pub async fn board_view_html(Path(name): Path<String>) -> Result<Html<String>, StatusCode> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Html(super::render::render_custom_board_shell(&name)))
}
