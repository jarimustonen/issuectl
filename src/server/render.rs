use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use pulldown_cmark::{Event, Options, Parser};

use crate::repo;
use crate::slug;

use super::AppState;

const BOARD_CSS: &str = include_str!("client/board.css");
const BOARD_JS: &str = include_str!("client/board.js");
const THEME_TOGGLE_JS: &str = include_str!("client/theme-toggle.js");
const THEME_BOOTSTRAP_JS: &str = include_str!("client/theme-bootstrap.js");

pub async fn board_html() -> Html<String> {
    Html(render_board_shell())
}

fn asset_response(content_type: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            // Local dev tool: re-fetch on each load (we serve straight from
            // the binary; no content hash to long-cache against). Kept
            // explicit to reduce bandwidth surprises on `--host 0.0.0.0`.
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

pub async fn board_css() -> Response {
    asset_response("text/css; charset=utf-8", BOARD_CSS)
}

pub async fn board_js() -> Response {
    asset_response("application/javascript; charset=utf-8", BOARD_JS)
}

pub async fn theme_toggle_js() -> Response {
    asset_response("application/javascript; charset=utf-8", THEME_TOGGLE_JS)
}

pub async fn theme_bootstrap_js() -> Response {
    asset_response("application/javascript; charset=utf-8", THEME_BOOTSTRAP_JS)
}

pub async fn issue_html(
    State(state): State<AppState>,
    Path(slug_param): Path<String>,
) -> Result<Html<String>, StatusCode> {
    if !slug::is_valid(&slug_param) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let root = state.root.clone();
    let slug_for_load = slug_param.clone();
    let issue = tokio::task::spawn_blocking(move || repo::load_issue(root.as_path(), &slug_for_load))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Html(render_issue_page(&issue)))
}

fn render_board_shell() -> String {
    // No interpolated values; the literal lives here only so the same
    // function can grow markup later without changing the route signature.
    r##"<!DOCTYPE html>
<html lang="en" data-theme="auto">
<head>
<meta charset="utf-8">
<title>issuectl board</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="/assets/board.css">
<script src="/assets/theme-bootstrap.js"></script>
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

<section id="warnings" class="warnings" hidden></section>

<section class="filter-bar" aria-label="Filters">
  <label>Search <input id="filter-search" type="search" placeholder="slug or title"></label>
  <label>Type <select id="filter-type"><option value="">all</option></select></label>
  <label>Assignee <select id="filter-assignee"><option value="">all</option></select></label>
  <label>Epic <select id="filter-epic"><option value="">all</option></select></label>
  <label>Label <select id="filter-label"><option value="">all</option></select></label>
</section>

<main id="board" class="board" aria-busy="true"></main>

<dialog id="detail" class="detail-dialog" aria-label="Issue detail">
  <article id="detail-body"></article>
  <button id="detail-close" autofocus>Close</button>
</dialog>

<script src="/assets/theme-toggle.js" defer></script>
<script src="/assets/board.js" defer></script>
</body>
</html>"##
        .to_string()
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
<script src="/assets/theme-bootstrap.js"></script>
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
<script src="/assets/theme-toggle.js" defer></script>
</body>
</html>"##,
    )
}

/// Render markdown to sanitized HTML. Used for issue bodies and side docs.
///
/// Two layers of defense:
///
/// 1. **Raw HTML in the markdown source is dropped before rendering.** Markdown
///    normally passes through inline/block HTML untouched; we filter
///    `Event::Html` and `Event::InlineHtml` so an issue body can't sneak in
///    `<input type="password">`, `<div id="theme-toggle" class="card">`, or
///    other markup that would survive a pure attribute-level sanitizer. After
///    this filter the only HTML reaching the sanitizer is what
///    pulldown-cmark itself emits.
///
/// 2. **Explicit `ammonia::Builder` policy** (vs. `ammonia::clean` defaults)
///    so the allow-list is auditable. Ammonia defaults strip `<input>` (which
///    breaks tasklist checkboxes) and `id` attributes (which breaks footnote
///    backlinks); the additions below re-enable exactly those — and only
///    those — for renderer-generated markup, which the layer 1 filter
///    guarantees is the only source of those tags here.
pub fn sanitize_markdown(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(md, opts).filter(|ev| {
        !matches!(ev, Event::Html(_) | Event::InlineHtml(_))
    });
    let mut raw = String::new();
    pulldown_cmark::html::push_html(&mut raw, parser);

    let mut builder = ammonia::Builder::default();
    builder
        .add_tags(["input"])
        .add_tag_attributes("input", ["type", "disabled", "checked"])
        .add_tag_attributes("li", ["id", "class"])
        .add_tag_attributes("a", ["id", "class"])
        .add_tag_attributes("sup", ["id", "class"])
        .add_tag_attributes("div", ["id", "class"]);
    builder.clean(&raw).to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_script_tags() {
        let html = sanitize_markdown("<script>alert(1)</script>");
        assert!(!html.contains("script"));
        assert!(!html.contains("alert"));
    }

    #[test]
    fn sanitize_strips_javascript_urls() {
        let html = sanitize_markdown("[click](javascript:alert(1))");
        assert!(!html.contains("javascript:"));
    }

    #[test]
    fn sanitize_strips_event_handlers() {
        let html = sanitize_markdown(r#"<img src="x" onerror="alert(1)">"#);
        assert!(!html.contains("onerror"));
        assert!(!html.contains("alert"));
    }

    #[test]
    fn sanitize_keeps_tasklist_checkboxes() {
        let html = sanitize_markdown("- [x] done\n- [ ] todo\n");
        assert!(html.contains("<input"), "checkbox stripped: {html}");
        assert!(html.contains("type=\"checkbox\""));
    }

    #[test]
    fn sanitize_keeps_footnote_ids() {
        // pulldown-cmark uses bare digits as anchor ids: <div ... id="1">.
        let html = sanitize_markdown("see[^1]\n\n[^1]: details");
        assert!(html.contains("id=\"1\""), "footnote id stripped: {html}");
        assert!(html.contains("href=\"#1\""), "footnote ref stripped: {html}");
    }

    #[test]
    fn sanitize_drops_user_authored_input_tag() {
        // Even though we allow <input> at the ammonia layer for tasklist
        // checkboxes, raw HTML in the markdown body is filtered before
        // rendering. A phishing input is dropped wholesale.
        let html = sanitize_markdown(r#"<input type="password" placeholder="API token">"#);
        assert!(!html.contains("<input"), "raw input survived: {html}");
    }

    #[test]
    fn sanitize_drops_user_authored_div_with_chrome_classes() {
        // Without the raw-HTML filter, `<div id="theme-toggle" class="card">`
        // would survive (pulldown passes it through; ammonia would let id/class
        // through because we whitelist them on div for footnote styling). With
        // the filter, the entire block — open tag, inner text, close tag — is
        // dropped, which is fine because pulldown also drops the text inside a
        // raw-HTML block.
        let html = sanitize_markdown(
            r#"<div id="theme-toggle" class="card detail-dialog">spoof</div>"#,
        );
        assert!(!html.contains("theme-toggle"));
        assert!(!html.contains("detail-dialog"));
        assert!(!html.contains("<div"));
    }

    #[test]
    fn sanitize_drops_iframes_and_objects() {
        let html = sanitize_markdown(
            "<iframe src=\"https://evil\"></iframe>\n<object data=\"x\"></object>",
        );
        assert!(!html.contains("<iframe"));
        assert!(!html.contains("<object"));
    }
}
