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
use axum::Router;
use axum::routing::get;
use tokio::net::TcpListener;

mod api;
mod render;

#[derive(Clone)]
pub struct AppState {
    pub root: Arc<PathBuf>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(render::board_html))
        .route("/issue/{slug}", get(render::issue_html))
        .route("/assets/board.css", get(render::board_css))
        .route("/assets/board.js", get(render::board_js))
        .route("/api/issues", get(api::list_issues))
        .route("/api/issues/{slug}", get(api::get_issue))
        .with_state(state)
}

pub fn run(root: PathBuf, host: String, port: u16) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("cannot build tokio runtime")?;
    runtime.block_on(serve(root, host, port))
}

async fn serve(root: PathBuf, host: String, port: u16) -> Result<()> {
    let state = AppState {
        root: Arc::new(root),
    };
    let addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("cannot bind {addr}"))?;
    let bound = listener.local_addr()?;
    eprintln!("issuectl serving on http://{bound}");
    eprintln!("Ctrl-C to stop");

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
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
        // Listing endpoint omits body to keep payloads small.
        assert_eq!(fox["body"], "");
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
