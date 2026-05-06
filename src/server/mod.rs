//! Read-only web board for the current `issues/` directory.
//!
//! Reads the filesystem on every request — `parser.rs`/`models.rs` remain the
//! single source of truth. A future iteration will add POST/PATCH endpoints
//! that delegate to `write.rs`; for now everything is GET.
//!
//! Realtime updates would slot in here as an SSE endpoint hung off `Router`,
//! reusing `repo::load_issues` per tick.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::http::{header, HeaderName, HeaderValue, Request};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;

mod api;
pub(crate) mod events;
mod render;
pub(crate) mod watcher;

use events::EventHub;

/// Options governing the optional filesystem watcher. `serve()` builds a
/// `WatcherConfig` from these and spawns `watcher::spawn(...)`. Set
/// `enabled=false` to drop the watcher entirely (read-only board, manual
/// refresh).
#[derive(Debug, Clone)]
pub struct ServeOptions {
    pub watch_enabled: bool,
    pub watch_bulk_threshold: usize,
}

impl Default for ServeOptions {
    fn default() -> Self {
        ServeOptions {
            watch_enabled: true,
            watch_bulk_threshold: 50,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub root: Arc<PathBuf>,
    pub event_hub: Arc<EventHub>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(render::board_html))
        .route("/issue/{slug}", get(render::issue_html))
        .route("/assets/board.css", get(render::board_css))
        .route("/assets/board.js", get(render::board_js))
        .route("/assets/theme-toggle.js", get(render::theme_toggle_js))
        .route(
            "/assets/theme-bootstrap.js",
            get(render::theme_bootstrap_js),
        )
        .route("/api/issues", get(api::list_issues))
        .route("/api/issues/{slug}", get(api::get_issue))
        .route("/api/issues/{slug}/docs/{name}", get(api::get_doc))
        .route("/events", get(api::events_stream))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

/// Inject a defense-in-depth security header set on every response. The CSP
/// is strict because the page only loads same-origin assets and never
/// dynamic scripts; tighten further if you remove inline use anywhere.
async fn security_headers(req: Request<axum::body::Body>, next: Next) -> Response {
    let mut response = next.run(req).await;
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; script-src 'self'; style-src 'self'; \
             img-src 'self' data:; connect-src 'self'; font-src 'self'; \
             object-src 'none'; base-uri 'none'; frame-ancestors 'none'; \
             form-action 'self'",
        ),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    response
}

pub fn run(root: PathBuf, host: String, port: u16, options: ServeOptions) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("cannot build tokio runtime")?;
    runtime.block_on(serve(root, host, port, options))
}

async fn serve(root: PathBuf, host: String, port: u16, options: ServeOptions) -> Result<()> {
    let event_hub = Arc::new(EventHub::new());
    let state = AppState {
        root: Arc::new(root.clone()),
        event_hub: event_hub.clone(),
    };

    // Watcher: a separate tokio task. We materialise `issues/open` and
    // `issues/closed` at startup so the watcher always has something to
    // hook — without this, `issuectl serve` in a fresh repo followed by
    // `issuectl new` from the CLI never lights up the board.
    let watcher_handle = if options.watch_enabled {
        for sub in &["open", "closed"] {
            let p = root.join("issues").join(sub);
            if let Err(e) = std::fs::create_dir_all(&p) {
                eprintln!(
                    "issuectl[serve]: cannot create {}: {} — watcher disabled",
                    p.display(),
                    e
                );
            }
        }
        if root.join("issues").is_dir() {
            let cfg = watcher::WatcherConfig {
                root: root.clone(),
                debounce: std::time::Duration::from_millis(150),
                bulk_threshold: options.watch_bulk_threshold,
            };
            Some(watcher::spawn(event_hub.clone(), cfg))
        } else {
            None
        }
    } else {
        None
    };

    let addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("cannot bind {addr} (try `--port 0` for a random free port)"))?;
    let bound = listener.local_addr()?;
    eprintln!("issuectl serving on http://{bound}");
    if !bound.ip().is_loopback() {
        eprintln!(
            "WARNING: bound to a non-loopback address — issue contents are reachable on this network."
        );
        eprintln!("         There is no authentication. Use only on trusted networks.");
    }
    eprintln!("Ctrl-C to stop");

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    if let Some(h) = watcher_handle {
        h.abort();
    }
    eprintln!("shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::fs;
    use std::path::Path;
    use tower::util::ServiceExt;

    fn write_issue(root: &Path, folder: &str, slug: &str, fm: &str, body: &str) {
        let dir = root.join("issues").join(folder).join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), format!("---\n{fm}---\n\n{body}\n")).unwrap();
    }

    fn make_router(root: &Path) -> axum::Router {
        router(AppState {
            root: Arc::new(root.to_path_buf()),
            event_hub: Arc::new(EventHub::new()),
        })
    }

    async fn body_string(body: Body) -> String {
        use http_body_util::BodyExt;
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn api_issues_returns_all_issues_with_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        fs::create_dir_all(tmp.path().join("issues/closed")).unwrap();
        write_issue(
            tmp.path(),
            "open",
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: high\nassignee: alice\n",
            "# Login is broken\n\nDetails here.",
        );
        write_issue(
            tmp.path(),
            "closed",
            "tiny-wild-comet",
            "type: task\nstatus: done\n",
            "# Old task\n",
        );

        let resp = make_router(tmp.path())
            .oneshot(Request::get("/api/issues").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let issues = json["issues"].as_array().unwrap();
        assert_eq!(issues.len(), 2);
        let fox = issues
            .iter()
            .find(|i| i["slug"] == "amber-loud-fox")
            .unwrap();
        assert_eq!(fox["type"], "bug");
        assert_eq!(fox["title"], "Login is broken");
        // Listing endpoint uses IssueSummary DTO — body is not a field at all.
        assert!(fox.get("body").is_none(), "summary should not include body");
        // Warnings array is always present, empty when nothing's wrong.
        assert_eq!(json["warnings"], serde_json::json!([]));
        // Cursor + instance_id are required for the SSE handoff.
        assert!(json["snapshot_seq"].is_u64());
        assert!(json["instance_id"].is_string());
    }

    #[tokio::test]
    async fn api_issue_detail_returns_rendered_html_body() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        write_issue(
            tmp.path(),
            "open",
            "quiet-brave-otter",
            "type: feature\nstatus: open\n",
            "# Add export\n\nWe need **CSV** export.",
        );
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/issues/quiet-brave-otter")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(json["slug"], "quiet-brave-otter");
        let html = json["body_html"].as_str().unwrap();
        assert!(html.contains("<strong>CSV</strong>"));
    }

    #[tokio::test]
    async fn api_issue_detail_404s_for_unknown_slug() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/issues/nope-nope-nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn api_issue_detail_rejects_invalid_slug() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        // `..` is not a valid slug shape, so it must be rejected before
        // touching the filesystem regardless of URL decoding details.
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/issues/NOT-VALID-SLUG-UPPER")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn root_returns_html_shell_with_assets() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        let resp = make_router(tmp.path())
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("<title>issuectl board</title>"));
        assert!(body.contains("/assets/board.css"));
    }

    #[tokio::test]
    async fn issue_html_page_renders_for_valid_slug() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        write_issue(
            tmp.path(),
            "open",
            "quiet-brave-otter",
            "type: feature\nstatus: open\n",
            "# Hello world\n\nbody.",
        );
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/issue/quiet-brave-otter")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("Hello world"));
        assert!(body.contains("quiet-brave-otter"));
    }

    #[tokio::test]
    async fn issue_html_page_404s_for_unknown_slug() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/issue/no-such-thing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn issue_html_escapes_title_content() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        write_issue(
            tmp.path(),
            "open",
            "spicy-clever-mole",
            "type: bug\nstatus: open\n",
            "# <img src=x onerror=alert(1)>\n",
        );
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/issue/spicy-clever-mole")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_string(resp.into_body()).await;
        // Title appears in <title> and <h1> as HTML-escaped text (so the
        // characters render as literals; "onerror=alert" appears as text but
        // is harmless). Body is the rendered+sanitized markdown — there
        // ammonia must strip the live onerror attribute.
        assert!(
            !body.contains("<img src=x onerror"),
            "live img tag survived: {body}"
        );
        // The escaped form must be present in the title — proves we didn't
        // emit the raw payload anywhere structural.
        assert!(body.contains("&lt;img src=x onerror=alert(1)&gt;"));
    }

    #[tokio::test]
    async fn detail_endpoint_strips_xss_from_body_html() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        write_issue(
            tmp.path(),
            "open",
            "tricky-noisy-toad",
            "type: bug\nstatus: open\n",
            "# T\n\n<script>alert(1)</script>\n[js](javascript:alert(1))\n",
        );
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/issues/tricky-noisy-toad")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let html = json["body_html"].as_str().unwrap();
        assert!(!html.contains("<script"));
        assert!(!html.contains("javascript:"));
        assert!(!html.contains("alert(1)"));
    }

    #[tokio::test]
    async fn detail_endpoint_lists_extra_md_docs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("issues/open/clever-quiet-stage");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: task\nstatus: open\n---\n\n# T\n",
        )
        .unwrap();
        fs::write(dir.join("analysis.md"), "# Analysis\n\nfindings.").unwrap();
        fs::write(dir.join("decisions.md"), "# Decisions\n").unwrap();
        // dotfiles + non-md should not appear:
        fs::write(dir.join(".hidden.md"), "ignore").unwrap();
        fs::write(dir.join("notes.txt"), "ignore").unwrap();

        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/issues/clever-quiet-stage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let docs: Vec<String> = json["docs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(docs, vec!["analysis.md", "decisions.md"]);
    }

    #[tokio::test]
    async fn doc_endpoint_serves_extra_md_rendered() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("issues/open/loud-spicy-fox");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\nstatus: open\n---\n# T\n").unwrap();
        fs::write(dir.join("analysis.md"), "# Analysis\n\n**bold**").unwrap();

        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/issues/loud-spicy-fox/docs/analysis.md")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(json["name"], "analysis.md");
        let html = json["body_html"].as_str().unwrap();
        assert!(html.contains("<strong>bold</strong>"));
    }

    #[tokio::test]
    async fn doc_endpoint_rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("issues/open/loud-spicy-fox");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\nstatus: open\n---\n# T\n").unwrap();
        // Caller cannot escape the issue directory via `../`, slashes, or
        // backslashes — is_safe_doc_name() rejects them all before the
        // filesystem is touched.
        for evil in [
            "..%2Fitem.md",
            "%2E%2E%2Fitem.md",
            "../../etc/passwd",
            "..%5Citem.md",
        ] {
            let resp = make_router(tmp.path())
                .clone()
                .oneshot(
                    Request::get(format!("/api/issues/loud-spicy-fox/docs/{evil}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(
                resp.status() == StatusCode::BAD_REQUEST || resp.status() == StatusCode::NOT_FOUND,
                "expected 400/404 for {evil}, got {}",
                resp.status()
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn docs_endpoint_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.md"), "# secret").unwrap();
        let dir = tmp.path().join("issues/open/safe-quiet-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\nstatus: open\n---\n# T\n").unwrap();
        // Plant a symlink inside the issue dir pointing to a file outside.
        symlink(outside.path().join("secret.md"), dir.join("evil.md")).unwrap();

        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/issues/safe-quiet-otter/docs/evil.md")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Must NOT be 200 — escaping the issue dir is forbidden. Either the
        // FORBIDDEN status from the prefix-check or NOT_FOUND if the symlink
        // can't be canonicalized; both are acceptable as long as the body
        // never leaks.
        assert_ne!(resp.status(), StatusCode::OK);
        assert!(resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::NOT_FOUND);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn issue_endpoint_rejects_symlinked_issue_dir() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        fs::write(
            outside.path().join("item.md"),
            "---\nstatus: open\n---\n# leaked\n",
        )
        .unwrap();
        symlink(
            outside.path(),
            tmp.path().join("issues/open/escaped-not-otter"),
        )
        .unwrap();

        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/issues/escaped-not-otter")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn warnings_surface_invalid_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("issues/open/broken-yaml-here");
        fs::create_dir_all(&dir).unwrap();
        // Unterminated quote → invalid YAML.
        fs::write(
            dir.join("item.md"),
            "---\nstatus: \"open\nbroken\n---\n# T\n",
        )
        .unwrap();
        let resp = make_router(tmp.path())
            .oneshot(Request::get("/api/issues").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let warnings = json["warnings"].as_array().unwrap();
        assert!(
            !warnings.is_empty(),
            "expected at least one parse warning, got {:?}",
            warnings
        );
        assert_eq!(warnings[0]["slug"], "broken-yaml-here");
    }

    #[tokio::test]
    async fn security_headers_present_on_root() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        let resp = make_router(tmp.path())
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let h = resp.headers();
        assert!(h.get("content-security-policy").is_some());
        assert_eq!(h.get("x-content-type-options").unwrap(), "nosniff");
        assert_eq!(h.get("referrer-policy").unwrap(), "no-referrer");
        assert_eq!(h.get("x-frame-options").unwrap(), "DENY");
    }

    #[tokio::test]
    async fn assets_have_correct_content_type() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        let r = make_router(tmp.path());
        let css = r
            .clone()
            .oneshot(
                Request::get("/assets/board.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(css.status(), StatusCode::OK);
        assert_eq!(
            css.headers().get("content-type").unwrap(),
            "text/css; charset=utf-8"
        );
        let js = r
            .oneshot(
                Request::get("/assets/board.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(js.status(), StatusCode::OK);
        assert_eq!(
            js.headers().get("content-type").unwrap(),
            "application/javascript; charset=utf-8"
        );
    }
}
