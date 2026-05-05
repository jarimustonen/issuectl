use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Serialize;

use crate::repo;
use crate::slug;

use super::AppState;
use super::render::sanitize_markdown;

#[derive(Serialize)]
pub struct IssueListResponse {
    pub issues: Vec<crate::models::Issue>,
}

#[derive(Serialize)]
pub struct IssueDetailResponse {
    #[serde(flatten)]
    pub issue: crate::models::Issue,
    pub body_html: String,
}

pub async fn list_issues(State(state): State<AppState>) -> Json<IssueListResponse> {
    let mut issues = repo::load_issues(state.root.as_path());
    // Drop the markdown body from the listing payload — the board only renders
    // metadata; bodies are fetched on demand via /api/issues/<slug>. This keeps
    // /api/issues O(slug-count * frontmatter-size) rather than O(total bytes).
    for issue in &mut issues {
        issue.body.clear();
    }
    Json(IssueListResponse { issues })
}

pub async fn get_issue(
    State(state): State<AppState>,
    Path(slug_param): Path<String>,
) -> Result<Json<IssueDetailResponse>, StatusCode> {
    if !slug::is_valid(&slug_param) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let issues = repo::load_issues(state.root.as_path());
    let issue = issues
        .into_iter()
        .find(|i| i.slug == slug_param)
        .ok_or(StatusCode::NOT_FOUND)?;
    let body_html = sanitize_markdown(&issue.body);
    Ok(Json(IssueDetailResponse { issue, body_html }))
}

