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
    /// Filter-bar fields the client should render. Subset of
    /// `["search", "type", "assignee", "epic", "label"]`. Empty means
    /// hide the filter bar entirely.
    pub filters: Vec<String>,
    pub issues: Vec<BoardIssueDto>,
    pub warnings: Vec<LoadWarning>,
    pub snapshot_seq: u64,
    pub instance_id: Uuid,
    /// True when the board renders but drag is disabled.
    pub read_only: bool,
    /// All soft-error messages, one per misconfiguration. Concatenated
    /// `\n` for display in a single banner; individual entries are
    /// also available for tooling.
    pub read_only_reasons: Vec<String>,
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
        let board = boards::load(root.as_path(), &board_name, &*cache)?;

        // `parsed_filter` is populated at load time; no string-matching
        // dance, no late re-parse, no fail-open on bad filter (those
        // are now hard errors at load time per AGENTS-AI-FIRST-CLI).
        let parsed_filter = board.parsed_filter.clone();

        let (full_issues, warnings) = repo::load_issues_with_warnings_via(root.as_path(), &*cache);
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
            let read_only = !board.soft_errors.is_empty();
            let read_only_reasons: Vec<String> =
                board.soft_errors.iter().map(|s| s.message()).collect();
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
                filter: board.filter_src,
                filters: board.filters,
                issues,
                warnings,
                snapshot_seq,
                instance_id,
                read_only,
                read_only_reasons,
            })
            .into_response()
        }
        Ok(Err(BoardError::NotFound)) => {
            super::api::error_response(StatusCode::NOT_FOUND, "not_found", "board not found")
        }
        // Hard validation errors get 422 (Unprocessable Entity) — the
        // YAML exists but is malformed. Distinct from NotFound so the
        // JS can render a different banner ("fix this file" vs.
        // "typo in URL").
        Ok(Err(BoardError::Validation(s))) => {
            super::api::error_response(StatusCode::UNPROCESSABLE_ENTITY, "invalid_board", &s)
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

/// `GET /board/<name>` — renders the shell only when the board YAML
/// exists and parses. A bad/missing board returns a 404 page instead
/// of a broken shell so the user can distinguish a URL typo from a
/// broken YAML.
pub async fn board_view_html(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Html<String>, StatusCode> {
    if !boards::is_valid_board_name(&name) {
        return Err(StatusCode::NOT_FOUND);
    }
    let root = state.root.clone();
    let cache = state.config.clone();
    let board_name = name.clone();
    let result =
        tokio::task::spawn_blocking(move || boards::load(root.as_path(), &board_name, &*cache))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match result {
        Ok(_) => Ok(Html(super::render::render_custom_board_shell(&name))),
        // Soft errors (UnknownGroupBy) still produce a `Board`; that's
        // the read-only path and the shell renders normally. Hard
        // validation errors return 404 here too — there's no useful
        // shell to show when the YAML itself is broken.
        Err(BoardError::NotFound) | Err(BoardError::Validation(_)) => Err(StatusCode::NOT_FOUND),
        Err(BoardError::Io(_)) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
