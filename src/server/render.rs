use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use pulldown_cmark::{Options, Parser};

use crate::repo;
use crate::slug;

use super::AppState;

const BOARD_CSS: &str = include_str!("client/board.css");
const BOARD_JS: &str = include_str!("client/board.js");
const THEME_TOGGLE_JS: &str = include_str!("client/theme-toggle.js");

pub async fn board_html() -> Html<String> {
    Html(render_board_shell())
}

pub async fn board_css() -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], BOARD_CSS).into_response()
}

pub async fn board_js() -> Response {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        BOARD_JS,
    )
        .into_response()
}

pub async fn issue_html(
    State(state): State<AppState>,
    Path(slug_param): Path<String>,
) -> Result<Html<String>, StatusCode> {
    if !slug::is_valid(&slug_param) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let issues = repo::load_issues(state.root.as_path());
    let issue = issues
        .into_iter()
        .find(|i| i.slug == slug_param)
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Html(render_issue_page(&issue)))
}

fn render_board_shell() -> String {
    let theme_js = THEME_TOGGLE_JS;
    let board_js = BOARD_JS;
    format!(
        r##"<!DOCTYPE html>
<html lang="en" data-theme="auto">
<head>
<meta charset="utf-8">
<title>issuectl board</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="/assets/board.css">
</head>
<body>
<header class="page-header">
  <div>
    <h1>Issues</h1>
    <p class="description" id="issue-count">Loading…</p>
  </div>
  <div class="header-actions">
    <button id="refresh" class="theme-toggle" title="Refresh">&#x21bb;</button>
    <button id="theme-toggle" class="theme-toggle" title="Toggle theme">&#x263e;</button>
  </div>
</header>

<section class="filter-bar" aria-label="Filters">
  <label>Search <input id="filter-search" type="search" placeholder="slug or title"></label>
  <label>Type <select id="filter-type"><option value="">all</option></select></label>
  <label>Assignee <select id="filter-assignee"><option value="">all</option></select></label>
  <label>Epic <select id="filter-epic"><option value="">all</option></select></label>
  <label>Label <select id="filter-label"><option value="">all</option></select></label>
</section>

<main id="board" class="board" aria-busy="true"></main>

<dialog id="detail" class="detail-dialog">
  <article id="detail-body"></article>
  <button id="detail-close" autofocus>Close</button>
</dialog>

<script>{theme_js}</script>
<script>{board_js}</script>
</body>
</html>"##
    )
}

fn render_issue_page(issue: &crate::models::Issue) -> String {
    let body_html = sanitize_markdown(&issue.body);
    let title = html_escape(&issue.title);
    let slug = html_escape(&issue.slug);
    let status = html_escape(&issue.status);
    let issue_type = html_escape(&issue.issue_type);
    format!(
        r##"<!DOCTYPE html>
<html lang="en" data-theme="auto">
<head>
<meta charset="utf-8">
<title>{title} — issuectl</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="/assets/board.css">
</head>
<body>
<header class="page-header">
  <div>
    <p class="description"><a href="/">&larr; board</a></p>
    <h1>{title}</h1>
    <p class="description"><code>{slug}</code> · {issue_type} · {status}</p>
  </div>
  <button id="theme-toggle" class="theme-toggle" title="Toggle theme">&#x263e;</button>
</header>
<article class="issue-detail markdown-body">{body_html}</article>
<script>{theme}</script>
</body>
</html>"##,
        theme = THEME_TOGGLE_JS,
    )
}

/// Render markdown to sanitized HTML. Used for issue bodies.
pub fn sanitize_markdown(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(md, opts);
    let mut raw = String::new();
    pulldown_cmark::html::push_html(&mut raw, parser);
    ammonia::clean(&raw)
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
