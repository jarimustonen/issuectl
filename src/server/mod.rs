//! Web board for the current `issues/` directory.
//!
//! Reads the filesystem on every request — `parser.rs`/`models.rs`
//! remain the single source of truth. The board pushes filesystem
//! changes live via the SSE `/events` endpoint backed by `events.rs`
//! and `watcher.rs` (M0). M1 will add POST/PATCH/PUT endpoints that
//! delegate to a future `mutate.rs`; for now everything is GET.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::http::{header, HeaderName, HeaderValue, Request};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;

mod api;
pub(crate) mod events;
pub(crate) mod ratelimit;
mod render;
pub(crate) mod watcher;

use events::EventHub;
use ratelimit::TokenBucketLimiter;

/// Options governing the optional filesystem watcher. `serve()` builds a
/// `WatcherConfig` from these and spawns `watcher::spawn(...)`. Set
/// `enabled=false` to drop the watcher entirely (read-only board, manual
/// refresh).
#[derive(Debug, Clone)]
pub struct ServeOptions {
    pub watch_enabled: bool,
    pub watch_bulk_threshold: usize,
    /// When true, allow PATCH/POST against issues even when bound to
    /// a non-loopback address. Default false: non-loopback binds are
    /// read-only, matching the design's "trusted-localhost" threat
    /// model. Reviewers flagged that the Host allow-list otherwise
    /// silently 403s every write from a remote browser.
    pub allow_remote_writes: bool,
}

impl Default for ServeOptions {
    fn default() -> Self {
        ServeOptions {
            watch_enabled: true,
            watch_bulk_threshold: 50,
            allow_remote_writes: false,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub root: Arc<PathBuf>,
    pub event_hub: Arc<EventHub>,
    /// Per-process random token (32 hex chars). Required in
    /// `X-Issuectl-CSRF` on every state-changing route. Bootstrap via
    /// `GET /api/session`. Restart → new token; `localStorage` from a
    /// prior process is invalidated.
    pub csrf_token: Arc<str>,
    /// `host:port` strings the server will accept as `Host` headers.
    /// Populated from the actual bound socket so DNS rebinding cannot
    /// reach the write surface (§9.2). Test routers use a permissive
    /// default; production uses the bound address plus its loopback
    /// aliases.
    pub allowed_hosts: Arc<Vec<String>>,
    /// When false, write routes reject every PATCH/POST regardless
    /// of CSRF token. Set true only on loopback or with explicit
    /// opt-in via `--allow-remote-writes`.
    pub writes_enabled: bool,
    /// Shared bucket for `PUT /body` and `POST /preview` (§6.6).
    /// 4 req/sec, burst 10. Keyed per-slug for body, single bucket
    /// for preview.
    pub body_limiter: Arc<TokenBucketLimiter>,
}

impl AppState {
    /// Test-only constructor with permissive Host allow-list and a
    /// fixed CSRF token. Production code uses `serve()` which builds
    /// the same struct with values derived from the bound socket.
    #[cfg(test)]
    pub fn for_test(root: PathBuf) -> Self {
        AppState {
            root: Arc::new(root),
            event_hub: Arc::new(EventHub::new()),
            csrf_token: Arc::from(""),
            allowed_hosts: Arc::new(Vec::new()),
            writes_enabled: true,
            body_limiter: Arc::new(TokenBucketLimiter::new(10.0, 4.0)),
        }
    }
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
        .route("/api/session", get(api::session))
        .route(
            "/api/issues",
            get(api::list_issues).post(api::create_issue),
        )
        .route(
            "/api/issues/{slug}",
            get(api::get_issue).patch(api::patch_issue),
        )
        .route("/api/issues/{slug}/body", axum::routing::put(api::put_body))
        .route("/api/issues/{slug}/docs/{name}", get(api::get_doc))
        .route("/api/preview", axum::routing::post(api::preview))
        .route("/events", get(api::events_stream))
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            host_and_csrf_guard,
        ))
        .with_state(state)
        // 1 MiB envelope: covers M1 PATCH metadata (well under
        // 64 KiB), POST `description` for new issues (which can be
        // multi-KB markdown), and leaves headroom for M2's body and
        // preview routes which the design caps at 1 MiB. Per-route
        // tighter limits land with M2.
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
}

/// Reject state-changing requests whose `Host` header does not match
/// the bound socket, or that lack a valid `X-Issuectl-CSRF` token. The
/// `Host` check defeats DNS rebinding (§9.1); the CSRF check defeats
/// ambient-authority CSRF from a malicious local process or browser
/// tab on another origin (§9.2). Read endpoints are allowed through
/// host-checked but token-free so the SSE handshake works without a
/// header (cookies cover its auth).
async fn host_and_csrf_guard(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    use axum::http::{Method, StatusCode};

    // Host validation: applied to every request when an allow-list
    // exists. Empty allow-list (test fixtures) waives the check.
    // RFC 3986 says hostnames are case-insensitive; the trailing dot
    // is also legal — both are stripped before compare so e.g.
    // `Host: Localhost.:7878` matches `localhost:7878`.
    if !state.allowed_hosts.is_empty() {
        let host_ok = req
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(|h| {
                let normalized = normalize_host_header(h);
                state.allowed_hosts.iter().any(|a| a == &normalized)
            })
            .unwrap_or(false);
        if !host_ok {
            return (
                StatusCode::FORBIDDEN,
                axum::Json(error_body("forbidden", "Host header not allowed", 403)),
            )
                .into_response();
        }
    }

    // CSRF token required on PATCH / POST / PUT / DELETE. Compared in
    // constant time so a malicious local process cannot recover the
    // 256-bit token byte-by-byte via response-time deltas (loopback
    // jitter is sub-ms, making timing oracles practical).
    let mutating = matches!(
        *req.method(),
        Method::PATCH | Method::POST | Method::PUT | Method::DELETE
    );
    if mutating && !state.writes_enabled {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(error_body(
                "forbidden",
                "writes disabled (non-loopback bind without --allow-remote-writes)",
                403,
            )),
        )
            .into_response();
    }
    if mutating && !state.csrf_token.is_empty() {
        let header_ok = req
            .headers()
            .get("x-issuectl-csrf")
            .and_then(|v| v.to_str().ok())
            .map(|t| constant_time_eq(t.as_bytes(), state.csrf_token.as_bytes()))
            .unwrap_or(false);
        if !header_ok {
            return (
                StatusCode::FORBIDDEN,
                axum::Json(error_body(
                    "forbidden",
                    "missing or invalid X-Issuectl-CSRF token",
                    403,
                )),
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// Constant-time bytewise compare. `a.len() != b.len()` short-circuits
/// — that leaks length only, which is fine for a fixed-length
/// 64-hex-char token.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Lowercase + trailing-dot strip. `Host: Localhost.:7878` →
/// `localhost:7878`.
fn normalize_host_header(h: &str) -> String {
    // RFC 3986: only the hostname is case-insensitive; the port is
    // numeric and unaffected. Lowercasing the whole string is
    // equivalent because digits map to themselves under to_ascii.
    let lowered = h.to_ascii_lowercase();
    // Trailing-dot strip handles `localhost.` (root-zone fully
    // qualified). It must run *before* port handling: `localhost.:80`
    // becomes `localhost:80`. Split on the last `:` to isolate the
    // host part, strip dot, rejoin.
    if let Some(idx) = lowered.rfind(':') {
        if lowered[idx + 1..].chars().all(|c| c.is_ascii_digit()) {
            let host = lowered[..idx].trim_end_matches('.');
            return format!("{}{}", host, &lowered[idx..]);
        }
    }
    lowered.trim_end_matches('.').to_string()
}

fn error_body(code: &str, detail: &str, status: u16) -> serde_json::Value {
    serde_json::json!({
        "type": format!("https://issuectl/errors/{code}"),
        "title": code.replace('_', " "),
        "status": status,
        "code": code,
        "detail": detail,
    })
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
    let csrf_token: Arc<str> = Arc::from(generate_csrf_token());

    let addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("cannot bind {addr} (try `--port 0` for a random free port)"))?;
    let bound = listener.local_addr()?;
    let allowed_hosts = host_allow_list(&bound);
    let writes_enabled = bound.ip().is_loopback() || options.allow_remote_writes;
    if !bound.ip().is_loopback() && !options.allow_remote_writes {
        eprintln!(
            "issuectl[serve]: bound to non-loopback {bound} — writes disabled. \
             Pass --allow-remote-writes to enable PATCH/POST."
        );
    }

    let state = AppState {
        root: Arc::new(root.clone()),
        event_hub: event_hub.clone(),
        csrf_token,
        allowed_hosts: Arc::new(allowed_hosts),
        writes_enabled,
        body_limiter: Arc::new(TokenBucketLimiter::new(10.0, 4.0)),
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

/// 256-bit hex token. Used as the per-process CSRF gate. Reusing
/// `Uuid::new_v4` would also work and pulls no new deps; explicit hex
/// of `rand::random::<[u8;32]>()` makes the entropy budget obvious.
fn generate_csrf_token() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

/// Host allow-list for the loopback bind. Includes the bound socket
/// plus the standard loopback aliases at the same port. A non-loopback
/// bind only allows the literal bound `host:port` (no aliases) — the
/// network case is documented as trusted-network-only.
fn host_allow_list(bound: &std::net::SocketAddr) -> Vec<String> {
    let port = bound.port();
    let mut out = vec![format!("{}", bound)];
    if bound.ip().is_loopback() {
        out.push(format!("127.0.0.1:{port}"));
        out.push(format!("localhost:{port}"));
        out.push(format!("[::1]:{port}"));
    }
    out.sort();
    out.dedup();
    out
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
        router(AppState::for_test(root.to_path_buf()))
    }

    /// Router with CSRF + Host enforcement turned on, for the M1
    /// security-gate tests. The token and host list are well-known so
    /// tests can either supply them (and pass) or omit them (and 403).
    fn make_secured_router(root: &Path) -> axum::Router {
        router(AppState {
            root: Arc::new(root.to_path_buf()),
            event_hub: Arc::new(EventHub::new()),
            csrf_token: Arc::from("testtoken"),
            allowed_hosts: Arc::new(vec!["test.invalid:7878".into()]),
            writes_enabled: true,
            body_limiter: Arc::new(TokenBucketLimiter::new(10.0, 4.0)),
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

    // ── M1: write surface ─────────────────────────────────────────

    fn seed_open_issue(root: &Path, slug: &str) {
        let dir = root.join("issues/open").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
    }

    /// Compute the canonical version directly from disk for a seeded
    /// issue.
    fn version_on_disk(root: &Path, slug: &str) -> String {
        let p = root.join("issues/open").join(slug).join("item.md");
        let parsed =
            crate::parser::parse_item_md_with_warnings(&p, slug, "open");
        crate::canonical::canonical_hash(&parsed.issue)
    }

    #[tokio::test]
    async fn session_returns_csrf_token() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        let r = make_secured_router(tmp.path());
        let resp = r
            .oneshot(
                Request::get("/api/session")
                    .header("host", "test.invalid:7878")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(body["csrf_token"], "testtoken");
        assert!(body["instance_id"].is_string());
    }

    #[tokio::test]
    async fn host_header_normalization_accepts_uppercase_and_trailing_dot() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        let r = make_secured_router(tmp.path());
        // RFC 3986: hostnames are case-insensitive; trailing dot is
        // legal. Both must pass the Host allow-list.
        for host in ["TEST.INVALID:7878", "test.invalid.:7878", "Test.Invalid.:7878"] {
            let resp = r
                .clone()
                .oneshot(
                    Request::get("/api/session")
                        .header("host", host)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "host {host} should match");
        }
    }

    #[tokio::test]
    async fn patch_without_csrf_token_is_forbidden() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "patch-without-token");
        let r = make_secured_router(tmp.path());
        let resp = r
            .oneshot(
                Request::patch("/api/issues/patch-without-token")
                    .header("host", "test.invalid:7878")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"priority":"high"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn patch_with_wrong_host_is_forbidden() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "patch-wrong-host");
        let r = make_secured_router(tmp.path());
        let resp = r
            .oneshot(
                Request::patch("/api/issues/patch-wrong-host")
                    .header("host", "evil.example:80")
                    .header("x-issuectl-csrf", "testtoken")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"priority":"high"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn patch_with_fresh_version_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "patch-fresh-vers");
        let v = version_on_disk(tmp.path(), "patch-fresh-vers");
        let r = make_router(tmp.path());
        let payload = serde_json::json!({
            "expected_version": v,
            "priority": "high",
        });
        let resp = r
            .oneshot(
                Request::patch("/api/issues/patch-fresh-vers")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert!(body["version"].as_str().unwrap().starts_with("sha256:"));
    }

    #[tokio::test]
    async fn patch_with_stale_version_returns_409_with_issue() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "patch-stale-vers");
        let r = make_router(tmp.path());
        let payload = serde_json::json!({
            "expected_version": "sha256:deadbeef",
            "priority": "high",
        });
        let resp = r
            .oneshot(
                Request::patch("/api/issues/patch-stale-vers")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(body["code"], "version_mismatch");
        assert_eq!(body["issue"]["slug"], "patch-stale-vers");
        assert!(body["version"].as_str().unwrap().starts_with("sha256:"));
    }

    #[tokio::test]
    async fn patch_status_to_closing_renames_directory() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "patch-close-dir");
        let r = make_router(tmp.path());
        let payload = serde_json::json!({ "status": "fixed" });
        let resp = r
            .oneshot(
                Request::patch("/api/issues/patch-close-dir")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert!(body["moved_to_closed"].as_bool().unwrap());
        assert!(tmp
            .path()
            .join("issues/closed/patch-close-dir/item.md")
            .exists());
        assert!(!tmp.path().join("issues/open/patch-close-dir").exists());
    }

    #[tokio::test]
    async fn patch_with_unknown_field_returns_400() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "patch-typo-fld");
        let r = make_router(tmp.path());
        let payload = r#"{"priorty": "high"}"#;
        let resp = r
            .oneshot(
                Request::patch("/api/issues/patch-typo-fld")
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn patch_with_overlapping_add_remove_returns_400() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "patch-overlap-lbl");
        let r = make_router(tmp.path());
        let payload = serde_json::json!({
            "add_labels": ["x"],
            "remove_labels": ["x"],
        });
        let resp = r
            .oneshot(
                Request::patch("/api/issues/patch-overlap-lbl")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(body["code"], "conflicting_intent");
    }

    #[tokio::test]
    async fn patch_corrupt_issue_returns_422() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("issues/open/corrupt-yml-here");
        fs::create_dir_all(&dir).unwrap();
        // Unterminated quote → parser warns, recovers to defaults.
        fs::write(
            dir.join("item.md"),
            "---\nstatus: \"open\nbroken\n---\n# T\n",
        )
        .unwrap();
        let r = make_router(tmp.path());
        let resp = r
            .oneshot(
                Request::patch("/api/issues/corrupt-yml-here")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"priority":"high"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(body["code"], "corrupt");
    }

    #[tokio::test]
    async fn patch_status_crossing_publishes_issue_moved() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "patch-publish-mvd");
        let event_hub = Arc::new(EventHub::new());
        let mut rx = event_hub.tx_subscribe_for_test();
        let r = router(AppState {
            root: Arc::new(tmp.path().to_path_buf()),
            event_hub: event_hub.clone(),
            csrf_token: Arc::from(""),
            allowed_hosts: Arc::new(Vec::new()),
            writes_enabled: true,
            body_limiter: Arc::new(TokenBucketLimiter::new(10.0, 4.0)),
        });
        let payload = serde_json::json!({ "status": "fixed" });
        let resp = r
            .oneshot(
                Request::patch("/api/issues/patch-publish-mvd")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // Drain the broadcast channel and look for the IssueMoved
        // payload. The mutate layer must publish exactly one
        // IssueMoved (no Remove+Upsert pair) for status crossings.
        let mut saw_moved = false;
        while let Ok(evt) = rx.try_recv() {
            if let crate::server::events::EventPayload::IssueMoved {
                slug, from_folder, to_folder, ..
            } = &evt.payload
            {
                assert_eq!(slug, "patch-publish-mvd");
                assert_eq!(from_folder, "open");
                assert_eq!(to_folder, "closed");
                saw_moved = true;
            }
        }
        assert!(saw_moved, "expected an IssueMoved event");
    }

    #[tokio::test]
    async fn patch_with_empty_string_label_returns_400() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "patch-empty-label");
        let r = make_router(tmp.path());
        let payload = serde_json::json!({ "add_labels": [""] });
        let resp = r
            .oneshot(
                Request::patch("/api/issues/patch-empty-label")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn patch_disabled_when_writes_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "patch-write-disab");
        let r = router(AppState {
            root: Arc::new(tmp.path().to_path_buf()),
            event_hub: Arc::new(EventHub::new()),
            csrf_token: Arc::from(""),
            allowed_hosts: Arc::new(Vec::new()),
            writes_enabled: false,
            body_limiter: Arc::new(TokenBucketLimiter::new(10.0, 4.0)),
        });
        let resp = r
            .oneshot(
                Request::patch("/api/issues/patch-write-disab")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"priority":"high"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn post_creates_new_issue() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        let r = make_router(tmp.path());
        let payload = serde_json::json!({
            "type": "bug",
            "title": "API created",
            "slug": "api-create-test",
            "reporter": "alice",
            "assignee": "bob",
            "priority": "high",
        });
        let resp = r
            .oneshot(
                Request::post("/api/issues")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(body["slug"], "api-create-test");
        assert!(body["version"].as_str().unwrap().starts_with("sha256:"));
        assert!(tmp
            .path()
            .join("issues/open/api-create-test/item.md")
            .exists());
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

    // ── M2: body + preview + rate limit ────────────────────────────

    #[tokio::test]
    async fn put_body_with_fresh_version_succeeds_and_advances() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "body-fresh-vers1");
        let v0 = version_on_disk(tmp.path(), "body-fresh-vers1");
        let r = make_router(tmp.path());
        let payload = serde_json::json!({
            "expected_version": v0,
            "body": "# New title\n\nfresh content."
        });
        let resp = r
            .oneshot(
                Request::put("/api/issues/body-fresh-vers1/body")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let v_new = body["version"].as_str().unwrap();
        assert!(v_new.starts_with("sha256:"));
        // version advanced compared to the pre-write hash
        assert_ne!(v_new, v0);
        // and matches what we just wrote on disk
        assert_eq!(v_new, version_on_disk(tmp.path(), "body-fresh-vers1"));
        // post-condition: version field also matches what we just wrote
        let on_disk = std::fs::read_to_string(
            tmp.path().join("issues/open/body-fresh-vers1/item.md"),
        )
        .unwrap();
        assert!(on_disk.contains("fresh content."));
    }

    #[tokio::test]
    async fn put_body_with_stale_version_returns_409_with_issue() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "body-stale-vers1");
        let r = make_router(tmp.path());
        let payload = serde_json::json!({
            "expected_version": "sha256:deadbeef",
            "body": "# stale\n",
        });
        let resp = r
            .oneshot(
                Request::put("/api/issues/body-stale-vers1/body")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(body["code"], "version_mismatch");
        assert_eq!(body["issue"]["slug"], "body-stale-vers1");
    }

    #[tokio::test]
    async fn put_body_rejects_invalid_slug_shape() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        let r = make_router(tmp.path());
        let resp = r
            .oneshot(
                Request::put("/api/issues/UPPER/body")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"body":"x"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_body_over_one_mib_rejected_under_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "body-size-limit1");
        let r = make_router(tmp.path());
        // 1 MiB + 1 raw bytes; well over the global 1 MiB envelope.
        let oversize = "a".repeat(1024 * 1024 + 256);
        let payload = serde_json::json!({ "body": oversize });
        let resp = r
            .clone()
            .oneshot(
                Request::put("/api/issues/body-size-limit1/body")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        // axum's DefaultBodyLimit returns 413 for over-cap requests.
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

        // ~512 KiB body — well under cap, must succeed.
        let small = "b".repeat(512 * 1024);
        let payload = serde_json::json!({ "body": small });
        let resp = r
            .oneshot(
                Request::put("/api/issues/body-size-limit1/body")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn put_body_rate_limit_fires_with_retry_after() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "body-rate-limit1");
        let r = make_router(tmp.path());
        let mut last = None;
        // Burst capacity is 10 in the default limiter; the 11th in
        // rapid succession on the same slug should trip 429.
        for _ in 0..12 {
            let payload =
                serde_json::json!({ "body": "# rate test\n\nmore content here.\n" });
            let resp = r
                .clone()
                .oneshot(
                    Request::put("/api/issues/body-rate-limit1/body")
                        .header("content-type", "application/json")
                        .body(Body::from(payload.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            last = Some(resp);
            if last.as_ref().unwrap().status() == StatusCode::TOO_MANY_REQUESTS {
                break;
            }
        }
        let resp = last.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "expected 429 after burst"
        );
        assert!(resp.headers().get("retry-after").is_some());
    }

    #[tokio::test]
    async fn preview_renders_and_sanitises_xss() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        let r = make_router(tmp.path());
        let payload = serde_json::json!({
            "body": "# Hello\n\n<script>alert(1)</script>\n[x](javascript:alert(1))"
        });
        let resp = r
            .oneshot(
                Request::post("/api/preview")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let html = body["body_html"].as_str().unwrap();
        assert!(html.contains("<h1>"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("javascript:"));
    }

    #[tokio::test]
    async fn preview_without_csrf_rejected_when_token_required() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        let r = make_secured_router(tmp.path());
        let resp = r
            .oneshot(
                Request::post("/api/preview")
                    .header("host", "test.invalid:7878")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"body":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn issue_detail_includes_version_for_body_editor() {
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "detail-vers-here");
        let r = make_router(tmp.path());
        let resp = r
            .oneshot(
                Request::get("/api/issues/detail-vers-here")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert!(body["version"].as_str().unwrap().starts_with("sha256:"));
    }
}
