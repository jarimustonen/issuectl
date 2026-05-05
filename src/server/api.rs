use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Serialize;

use crate::repo::{self, IssueSummary, LoadWarning};
use crate::slug;

use super::AppState;
use super::render::sanitize_markdown;

#[derive(Serialize)]
pub struct IssueListResponse {
    pub issues: Vec<IssueSummary>,
    /// Per-file parse warnings (e.g., malformed YAML, missing item.md).
    /// Empty when nothing is wrong; UI can flag broken issues from this list.
    pub warnings: Vec<LoadWarning>,
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
    let root = state.root.clone();
    let (issues, warnings) =
        tokio::task::spawn_blocking(move || repo::load_issue_summaries(root.as_path()))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(IssueListResponse { issues, warnings }))
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
        let dir = root
            .join("issues")
            .join(&issue.folder)
            .join(&issue.slug);
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
    let body = tokio::task::spawn_blocking(move || {
        let (folder, _item) = repo::locate_issue(root.as_path(), &slug_owned)?;
        let path = root
            .join("issues")
            .join(&folder)
            .join(&slug_owned)
            .join(&doc_owned);
        // Rebuilt-from-validated-segments path cannot escape the issue dir,
        // but assert via canonicalization that it stays under the issue
        // directory in case of symlinks.
        let canon = std::fs::canonicalize(&path)?;
        let issue_dir =
            std::fs::canonicalize(root.join("issues").join(&folder).join(&slug_owned))?;
        if !canon.starts_with(&issue_dir) {
            anyhow::bail!("doc path escapes issue directory");
        }
        Ok(std::fs::read_to_string(&path)?)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::NOT_FOUND)?;

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
fn is_safe_doc_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.starts_with('.') {
        return false;
    }
    if !name.ends_with(".md") {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.contains("..")
}
