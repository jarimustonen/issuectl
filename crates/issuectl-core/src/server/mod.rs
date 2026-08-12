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
mod boards;
pub(crate) mod events;
pub(crate) mod ratelimit;
mod render;
pub(crate) mod watcher;

use events::EventHub;
use ratelimit::TokenBucketLimiter;

use crate::repo_config::RepoConfigCache;

/// Options governing the optional filesystem watcher. `serve()` builds a
/// `WatcherConfig` from these and spawns `watcher::spawn(...)`. Set
/// `enabled=false` to drop the watcher entirely (read-only board, manual
/// refresh).
#[derive(Debug, Clone)]
pub struct ServeOptions {
    pub watch_enabled: bool,
    pub watch_bulk_threshold: usize,
    /// When `Some(interval)`, force the polling watcher backend (the
    /// documented network-FS workaround per design §8.1). `None` →
    /// recommended platform backend (inotify/FSEvents/ReadDirectoryChanges).
    pub watch_poll_interval: Option<std::time::Duration>,
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
            watch_poll_interval: None,
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
    /// Reflects whether the watcher is *actually* running. False when
    /// `--no-watch` was passed, when `issues/` doesn't exist on
    /// startup, or when watcher spawn failed. Surfaced via
    /// `/api/session` so the UI can promote the manual refresh button
    /// and skip "live updates" affordances. Write-originated SSE
    /// events still flow regardless.
    pub watch_enabled: bool,
    /// Latched terminal degradation reason: `Some(...)` after the
    /// supervisor has given up on the watcher (3 failed restarts per
    /// §8.5). Read by `/api/session` so a client connecting *after*
    /// the SSE `Degraded` event aged out of replay still sees the
    /// banner. Writer is the watcher supervisor; readers are the
    /// session handler. parking_lot::Mutex for cheap uncontended
    /// access from the request thread.
    pub watch_degraded: Arc<parking_lot::Mutex<Option<String>>>,
    /// Per-process parsed `issues/.schema.yaml` +
    /// `.issuectl/transitions.yaml`. Each PATCH/POST stats both files
    /// and only re-parses when mtime advances; CLI behaviour is
    /// untouched because the cache is plumbed in via a thread-local
    /// guard activated only inside server mutate handlers.
    pub config: Arc<RepoConfigCache>,
}

impl AppState {
    /// Test-only constructor with permissive Host allow-list and a
    /// fixed CSRF token. Production code uses `serve()` which builds
    /// the same struct with values derived from the bound socket.
    #[cfg(test)]
    pub fn for_test(root: PathBuf) -> Self {
        let config = Arc::new(RepoConfigCache::new(root.clone()));
        AppState {
            root: Arc::new(root),
            event_hub: Arc::new(EventHub::new()),
            csrf_token: Arc::from(""),
            allowed_hosts: Arc::new(Vec::new()),
            writes_enabled: true,
            body_limiter: Arc::new(TokenBucketLimiter::new(10.0, 4.0)),
            watch_enabled: true,
            watch_degraded: Arc::new(parking_lot::Mutex::new(None)),
            config,
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(render::board_html))
        .route("/board/{name}", get(boards::board_view_html))
        .route("/api/boards", get(boards::list_boards_api))
        .route("/api/boards/{name}", get(boards::get_board_api))
        .route("/issue/{slug}", get(render::issue_html))
        .route("/assets/board.css", get(render::board_css))
        .route("/assets/board.js", get(render::board_js))
        .route("/assets/theme-toggle.js", get(render::theme_toggle_js))
        .route(
            "/assets/theme-bootstrap.js",
            get(render::theme_bootstrap_js),
        )
        .route("/api/session", get(api::session))
        .route("/api/issues", get(api::list_issues).post(api::create_issue))
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

    let watch_degraded: Arc<parking_lot::Mutex<Option<String>>> =
        Arc::new(parking_lot::Mutex::new(None));

    // Watcher: a separate tokio task. We materialise `issues/` at startup
    // so the watcher always has something to hook — without this,
    // `issuectl serve` in a fresh repo followed by `issuectl new` from
    // the CLI never lights up the board.
    let watcher_handle = if options.watch_enabled {
        let p = root.join("issues");
        if let Err(e) = std::fs::create_dir_all(&p) {
            eprintln!(
                "issuectl[serve]: cannot create {}: {} — watcher disabled",
                p.display(),
                e
            );
        }
        if root.join("issues").is_dir() {
            let backend = match options.watch_poll_interval {
                Some(interval) => watcher::WatcherBackend::Poll(interval),
                None => watcher::WatcherBackend::Recommended,
            };
            // M3 log: confirm to the operator which backend is active —
            // hard to debug "polling didn't take effect" without it.
            match backend {
                watcher::WatcherBackend::Poll(interval) => eprintln!(
                    "issuectl[serve]: watcher = polling backend, interval {}ms",
                    interval.as_millis()
                ),
                watcher::WatcherBackend::Recommended => {
                    eprintln!("issuectl[serve]: watcher = native backend")
                }
            }
            let cfg = watcher::WatcherConfig {
                root: root.clone(),
                debounce: std::time::Duration::from_millis(150),
                bulk_threshold: options.watch_bulk_threshold,
                backend,
            };
            Some(watcher::spawn(
                event_hub.clone(),
                watch_degraded.clone(),
                cfg,
            ))
        } else {
            eprintln!(
                "issuectl[serve]: {} is not a directory — watcher disabled",
                root.join("issues").display()
            );
            None
        }
    } else {
        eprintln!(
            "issuectl[serve]: --no-watch — filesystem watcher disabled. \
             External edits won't propagate; use the manual refresh button."
        );
        None
    };
    // H1: AppState.watch_enabled reflects *actual* watcher state, not
    // the user's intent. If `--no-watch` was passed, or the issues dir
    // could not be created, or `is_dir()` was false, the watcher
    // didn't spawn and clients should see the manual-refresh banner.
    let actual_watch_enabled = watcher_handle.is_some();

    let state = AppState {
        root: Arc::new(root.clone()),
        event_hub: event_hub.clone(),
        csrf_token,
        allowed_hosts: Arc::new(allowed_hosts),
        writes_enabled,
        body_limiter: Arc::new(TokenBucketLimiter::new(10.0, 4.0)),
        watch_enabled: actual_watch_enabled,
        watch_degraded,
        config: Arc::new(RepoConfigCache::new(root.clone())),
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

    fn write_issue(root: &Path, _folder: &str, slug: &str, fm: &str, body: &str) {
        // Flat layout post-`awfully-faint-sound`: `_folder` retained for
        // call-site compatibility and now derived from frontmatter status.
        let dir = root.join("issues").join(slug);
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
            watch_enabled: true,
            watch_degraded: Arc::new(parking_lot::Mutex::new(None)),
            config: Arc::new(RepoConfigCache::new(root.to_path_buf())),
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();

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
    async fn api_issues_q_param_filters_via_query_engine() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();

        write_issue(
            tmp.path(),
            "open",
            "amber-loud-fox",
            "type: bug\nstatus: in-progress\npriority: high\nassignee: alice\n",
            "# Login is broken\n",
        );
        write_issue(
            tmp.path(),
            "open",
            "calm-bright-newt",
            "type: feature\nstatus: open\npriority: normal\nassignee: bob\n",
            "# Add export\n",
        );

        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/issues?q=status:in-progress")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let issues = json["issues"].as_array().unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0]["slug"], "amber-loud-fox");

        // Bareword text search hits the body.
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/issues?q=export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let issues = json["issues"].as_array().unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0]["slug"], "calm-bright-newt");

        // Malformed query → 400.
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/issues?q=bogus:value")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn api_issue_detail_returns_rendered_html_body() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        let dir = tmp.path().join("issues/clever-quiet-stage");
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
        let dir = tmp.path().join("issues/loud-spicy-fox");
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
        let dir = tmp.path().join("issues/loud-spicy-fox");
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
        let dir = tmp.path().join("issues/safe-quiet-otter");
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            outside.path().join("item.md"),
            "---\nstatus: open\n---\n# leaked\n",
        )
        .unwrap();
        symlink(outside.path(), tmp.path().join("issues/escaped-not-otter")).unwrap();

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
        let dir = tmp.path().join("issues/broken-yaml-here");
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        let dir = root.join("issues").join(slug);
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
        let p = root.join("issues").join(slug).join("item.md");
        let parsed = crate::parser::parse_item_md_with_warnings(&p, slug, "open");
        let mut issue = parsed.issue;
        let schema = crate::schema::default_schema();
        issue.folder = crate::repo::folder_for_status(&schema, &issue.status).to_string();
        crate::canonical::canonical_hash(&issue)
    }

    /// `--no-watch` flips `AppState.watch_enabled`; `/api/session`
    /// must surface that so the client can show the manual-refresh
    /// affordance and skip "live updates" UI. The default-true case is
    /// covered implicitly by other session tests; lock the false case
    /// explicitly because it's the one the M3 banner depends on.
    #[tokio::test]
    async fn session_reports_watch_disabled_when_no_watch() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        let r = router(AppState {
            root: Arc::new(tmp.path().to_path_buf()),
            event_hub: Arc::new(EventHub::new()),
            csrf_token: Arc::from(""),
            allowed_hosts: Arc::new(Vec::new()),
            writes_enabled: true,
            body_limiter: Arc::new(TokenBucketLimiter::new(10.0, 4.0)),
            watch_enabled: false,
            watch_degraded: Arc::new(parking_lot::Mutex::new(None)),
            config: Arc::new(RepoConfigCache::new(tmp.path().to_path_buf())),
        });
        let resp = r
            .oneshot(Request::get("/api/session").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(body["watch_enabled"], false);
    }

    /// The Degraded `EventPayload` variant must round-trip through the
    /// broadcast channel. The supervisor's "3 failed restarts →
    /// Degraded" sequence is exercised end-to-end by
    /// `server::watcher::tests::supervisor_emits_degraded_after_three_start_failures`
    /// (the actually-load-bearing path); this test only locks the
    /// publish/recv contract independently.
    #[tokio::test]
    async fn degraded_event_propagates_to_subscribers() {
        let hub = Arc::new(EventHub::new());
        let mut rx = hub.tx_subscribe_for_test();
        hub.publish(crate::server::events::EventPayload::Degraded {
            reason: "watcher_unavailable".to_string(),
        });
        let evt = rx.try_recv().expect("subscriber receives Degraded");
        match &evt.payload {
            crate::server::events::EventPayload::Degraded { reason } => {
                assert_eq!(reason, "watcher_unavailable");
            }
            other => panic!("expected Degraded, got {other:?}"),
        }
    }

    /// H1: `/api/session.degraded_reason` lets a client connecting
    /// *after* the SSE Degraded event aged out of replay still
    /// surface the banner. Without this, fresh tabs would silently
    /// see "live updates on" while the watcher is dead.
    #[tokio::test]
    async fn session_surfaces_latched_degraded_reason() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        let watch_degraded: Arc<parking_lot::Mutex<Option<String>>> = Arc::new(
            parking_lot::Mutex::new(Some("watcher_unavailable".to_string())),
        );
        let r = router(AppState {
            root: Arc::new(tmp.path().to_path_buf()),
            event_hub: Arc::new(EventHub::new()),
            csrf_token: Arc::from(""),
            allowed_hosts: Arc::new(Vec::new()),
            writes_enabled: true,
            body_limiter: Arc::new(TokenBucketLimiter::new(10.0, 4.0)),
            watch_enabled: false,
            watch_degraded,
            config: Arc::new(RepoConfigCache::new(tmp.path().to_path_buf())),
        });
        let resp = r
            .oneshot(Request::get("/api/session").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(body["degraded_reason"], "watcher_unavailable");
    }

    #[tokio::test]
    async fn session_returns_csrf_token() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        let r = make_secured_router(tmp.path());
        // RFC 3986: hostnames are case-insensitive; trailing dot is
        // legal. Both must pass the Host allow-list.
        for host in [
            "TEST.INVALID:7878",
            "test.invalid.:7878",
            "Test.Invalid.:7878",
        ] {
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

    /// Two PATCHes against the same router share the schema +
    /// transitions parses. This is the user-visible contract of the
    /// `RepoConfigCache`: each request stats both config files but
    /// only re-parses when one has changed. Catches regressions where
    /// `api.rs` forgets to install the cache before delegating to
    /// `mutate::*`.
    #[tokio::test]
    async fn two_patches_share_one_config_parse() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        seed_open_issue(&root, "patch-cache-share");
        // Pre-bootstrap the schema file so `ensure_default_written`
        // does not advance its mtime between the two PATCHes (which
        // would invalidate the cache mid-test).
        crate::schema::ensure_default_written(&root).unwrap();

        let cache = Arc::new(RepoConfigCache::new(root.clone()));
        let state = AppState {
            root: Arc::new(root.clone()),
            event_hub: Arc::new(EventHub::new()),
            csrf_token: Arc::from(""),
            allowed_hosts: Arc::new(Vec::new()),
            writes_enabled: true,
            body_limiter: Arc::new(TokenBucketLimiter::new(10.0, 4.0)),
            watch_enabled: true,
            watch_degraded: Arc::new(parking_lot::Mutex::new(None)),
            config: cache.clone(),
        };
        let r = router(state);

        // First PATCH: schema + transitions both miss → 2 parses.
        let v1 = version_on_disk(&root, "patch-cache-share");
        let resp1 = r
            .clone()
            .oneshot(
                Request::patch("/api/issues/patch-cache-share")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "expected_version": v1,
                            "priority": "high",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);
        let after_first = cache.refresh_count();
        assert_eq!(
            after_first, 2,
            "first PATCH should parse schema + rules once each",
        );

        // Second PATCH: stamps unchanged → cache hit on both. The
        // version token has rotated because the first PATCH wrote;
        // re-read from disk so the optimistic-concurrency check
        // passes and we exercise the full mutate path again.
        let v2 = version_on_disk(&root, "patch-cache-share");
        let resp2 = r
            .oneshot(
                Request::patch("/api/issues/patch-cache-share")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "expected_version": v2,
                            "priority": "normal",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        assert_eq!(
            cache.refresh_count(),
            after_first,
            "second PATCH must reuse cached schema + rules — got {} parses, expected {}",
            cache.refresh_count(),
            after_first,
        );
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
    async fn patch_status_to_closing_does_not_move_directory() {
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
        // Flat layout: the issue stays at issues/<slug>/ regardless of status.
        let item = tmp.path().join("issues/patch-close-dir/item.md");
        assert!(item.exists(), "flat path must remain after close");
        let on_disk = fs::read_to_string(&item).unwrap();
        assert!(on_disk.contains("status: fixed"));
        assert!(!tmp.path().join("issues/closed/patch-close-dir").exists());
        assert!(!tmp.path().join("issues/open/patch-close-dir").exists());
    }

    #[tokio::test]
    async fn patch_status_open_clears_closed_date_and_returns_summary() {
        // Drag-and-drop reopen path through the HTTP layer: a card in
        // the closed column dragged to an active column issues a
        // status-only PATCH. The response must reflect both the
        // status change and the cleared `closed:` date so the board
        // can refresh in place via applyIssueToBoard without a
        // follow-up GET. Mirrors mutate.rs' reopening test, but at
        // the API level — that's where the drag-and-drop client
        // actually hits.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("issues/reopen-via-patch");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-01\nstatus: fixed\npriority: normal\nclosed: 2026-05-05\n---\n\n# T\n",
        )
        .unwrap();
        let v = version_on_disk(tmp.path(), "reopen-via-patch");
        let r = make_router(tmp.path());
        let payload = serde_json::json!({
            "expected_version": v,
            "status": "open",
        });
        let resp = r
            .oneshot(
                Request::patch("/api/issues/reopen-via-patch")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert!(body["moved_to_open"].as_bool().unwrap_or(false));
        assert_eq!(body["issue"]["status"], "open");
        assert!(
            body["issue"]["closed"].is_null(),
            "response must signal cleared closed date, got: {}",
            body["issue"]["closed"]
        );
        let on_disk = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(on_disk.contains("status: open"));
        assert!(
            !on_disk.contains("closed:"),
            "frontmatter must drop closed: on reopen, got:\n{on_disk}"
        );
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
        let dir = tmp.path().join("issues/corrupt-yml-here");
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
    async fn patch_status_crossing_publishes_issue_upserted() {
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
            watch_enabled: true,
            watch_degraded: Arc::new(parking_lot::Mutex::new(None)),
            config: Arc::new(RepoConfigCache::new(tmp.path().to_path_buf())),
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
        // Post-flat-layout, status crossings publish a single
        // IssueUpserted with the new version. Clients re-bucket via
        // `summary.status`/`summary.folder`; no folder-rename event.
        let mut saw_upserted = false;
        while let Ok(evt) = rx.try_recv() {
            if let crate::server::events::EventPayload::IssueUpserted { slug, issue, .. } =
                &evt.payload
            {
                assert_eq!(slug, "patch-publish-mvd");
                assert_eq!(issue.status, "fixed");
                assert_eq!(issue.folder, "closed");
                saw_upserted = true;
            }
        }
        assert!(saw_upserted, "expected an IssueUpserted event");
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
            watch_enabled: true,
            watch_degraded: Arc::new(parking_lot::Mutex::new(None)),
            config: Arc::new(RepoConfigCache::new(tmp.path().to_path_buf())),
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        assert!(tmp.path().join("issues/api-create-test/item.md").exists());
    }

    #[tokio::test]
    async fn assets_have_correct_content_type() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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

    #[tokio::test]
    async fn board_js_wires_abortcontroller_on_write_paths() {
        // Smoke-pin the @absolutely-aberrant-caption fix: the bundled
        // client must keep the AbortController + per-write timeout +
        // pagehide-cancel wiring. Catches accidental removal of any
        // of the three (e.g. someone resurrecting a `fetch(...)`
        // without `signal:`).
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        let r = make_router(tmp.path());
        let resp = r
            .oneshot(
                Request::get("/assets/board.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        let js = std::str::from_utf8(&bytes).unwrap();
        assert!(
            js.contains("new AbortController"),
            "board.js missing AbortController"
        );
        assert!(
            js.contains("WRITE_TIMEOUT_MS"),
            "board.js missing write timeout"
        );
        assert!(
            js.contains("'pagehide'"),
            "board.js missing pagehide listener for in-flight aborts"
        );
        // Both write paths (PATCH and PUT) must pass `signal: abort.signal`
        // to `fetch`. Counting occurrences pins both — a regression that
        // removes the signal from just one path no longer slips through
        // the previous "any-occurrence" check.
        let signal_count = js.matches("signal: abort.signal").count();
        assert_eq!(
            signal_count, 2,
            "expected `signal: abort.signal` on both write paths, got {signal_count}",
        );
        // Both write paths must also clean up via `.finally(...)` so an
        // exception in a `.then` body cannot leak the abort registry or
        // double-drain `pending_writes` via `.catch`.
        let finally_count = js.matches(".finally(").count();
        assert!(
            finally_count >= 2,
            "expected `.finally(` on both write paths, got {finally_count}",
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
        let on_disk =
            std::fs::read_to_string(tmp.path().join("issues/body-fresh-vers1/item.md")).unwrap();
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        // Freeze the limiter's clock so no tokens refill mid-burst.
        // Wall-clock refill (4/s) is what made this flaky: under CI
        // load each request took long enough that the bucket topped up
        // and the 11th could still be allowed. With a frozen clock the
        // first 10 (capacity) requests are allowed and the 11th
        // rejects, deterministically regardless of request timing.
        let mut state = AppState::for_test(tmp.path().to_path_buf());
        state.body_limiter = Arc::new(ratelimit::TokenBucketLimiter::with_clock(
            10.0,
            4.0,
            Arc::new(ratelimit::FrozenClock::new()),
        ));
        let r = router(state);

        let put = |r: axum::Router| async move {
            let payload = serde_json::json!({ "body": "# rate test\n\nmore content here.\n" });
            r.oneshot(
                Request::put("/api/issues/body-rate-limit1/body")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
        };

        // Capacity is 10: the first 10 requests on the same slug must
        // all be allowed.
        for i in 1..=10 {
            let resp = put(r.clone()).await;
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "request {i} within capacity should be allowed"
            );
        }
        // The 11th exhausts the bucket and must trip 429 + Retry-After.
        let resp = put(r.clone()).await;
        assert_eq!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "11th request should exceed burst capacity"
        );
        assert!(resp.headers().get("retry-after").is_some());
    }

    #[tokio::test]
    async fn preview_renders_and_sanitises_xss() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
    async fn put_body_409_envelope_includes_version_and_body_html_in_issue() {
        // M2 review F1: clients reading `theirs.version` from the 409
        // envelope's embedded `issue` were looping on stale tokens
        // because the field wasn't there. Lock the response shape
        // (§4.3): `issue` carries `version` + `body_html`, and the
        // top-level `version` mirrors it.
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "body-409-shape1");
        let r = make_router(tmp.path());
        let payload = serde_json::json!({
            "expected_version": "sha256:deadbeef",
            "body": "stale draft",
        });
        let resp = r
            .oneshot(
                Request::put("/api/issues/body-409-shape1/body")
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
        let top_v = body["version"].as_str().unwrap();
        assert!(top_v.starts_with("sha256:"));
        let issue_v = body["issue"]["version"].as_str().unwrap();
        assert_eq!(
            top_v, issue_v,
            "issue.version must mirror top-level version"
        );
        assert!(body["issue"]["body_html"].as_str().is_some());
    }

    #[tokio::test]
    async fn put_body_rejects_unknown_field() {
        // M2 review F7: typos in optimistic-concurrency field names
        // must 400, not silently parse as `expected_version: None` and
        // proceed with a blind overwrite.
        let tmp = tempfile::tempdir().unwrap();
        seed_open_issue(tmp.path(), "body-deny-unknown");
        let r = make_router(tmp.path());
        let payload = serde_json::json!({
            "expected_verison": "sha256:abc",
            "body": "x",
        });
        let resp = r
            .oneshot(
                Request::put("/api/issues/body-deny-unknown/body")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn preview_rejects_unknown_field() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        let r = make_router(tmp.path());
        let payload = serde_json::json!({
            "body": "# x",
            "extra": "boom",
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
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
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

    fn write_board(root: &Path, name: &str, body: &str) {
        let dir = root.join(".issuectl").join("boards");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{name}.yaml")), body).unwrap();
    }

    /// Happy path: epic-grouped board renders with columns ordered as
    /// declared and group_value resolved per issue.
    #[tokio::test]
    async fn api_board_returns_columns_and_grouped_issues() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        write_issue(
            tmp.path(),
            "open",
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: high\nepic: alpha\n",
            "# t\n",
        );
        write_issue(
            tmp.path(),
            "open",
            "calm-bright-newt",
            "type: feature\nstatus: open\npriority: normal\n",
            "# u\n",
        );
        write_board(
            tmp.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns:\n  - {value: '', label: Unscoped}\n  - {value: alpha, label: Alpha}\n",
        );
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/boards/triage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(json["group_by"], "epic");
        assert_eq!(json["builtin_group_by"], true);
        assert_eq!(json["read_only"], false);
        let cols = json["columns"].as_array().unwrap();
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0]["value"], "");
        assert_eq!(cols[1]["value"], "alpha");
        let issues = json["issues"].as_array().unwrap();
        assert_eq!(issues.len(), 2);
        let fox = issues
            .iter()
            .find(|i| i["slug"] == "amber-loud-fox")
            .unwrap();
        assert_eq!(fox["group_value"], "alpha");
        let newt = issues
            .iter()
            .find(|i| i["slug"] == "calm-bright-newt")
            .unwrap();
        assert_eq!(newt["group_value"], "");
    }

    #[tokio::test]
    async fn api_board_filter_excludes_non_matching_issues() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        write_issue(
            tmp.path(),
            "open",
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: high\nepic: alpha\n",
            "# t\n",
        );
        write_issue(
            tmp.path(),
            "open",
            "calm-bright-newt",
            "type: feature\nstatus: open\npriority: normal\n",
            "# u\n",
        );
        write_board(
            tmp.path(),
            "bugs",
            "name: bugs\ngroup_by: epic\ncolumns: [{value: '', label: U}, {value: alpha, label: A}]\nfilter: \"type:bug\"\n",
        );
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/boards/bugs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let issues = json["issues"].as_array().unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0]["slug"], "amber-loud-fox");
    }

    /// Drag-mutation round-trip: PATCH the issue's `epic` field via
    /// the existing `/api/issues/<slug>` PATCH path, then re-fetch the
    /// board and confirm the card moved columns.
    #[tokio::test]
    async fn board_drag_mutation_round_trip_via_patch() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        seed_open_issue(tmp.path(), "drag-roundtrip-1");
        write_board(
            tmp.path(),
            "release",
            "name: release\ngroup_by: epic\ncolumns:\n  - {value: '', label: Unscoped}\n  - {value: v-six, label: v0.6}\n",
        );
        let r = make_router(tmp.path());

        // 1. Initial fetch: card sits in unassigned bucket.
        let resp = r
            .clone()
            .oneshot(
                Request::get("/api/boards/release")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let issue = json["issues"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["slug"] == "drag-roundtrip-1")
            .cloned()
            .unwrap();
        assert_eq!(issue["group_value"], "");
        let version = issue["version"].as_str().unwrap().to_string();

        // 2. Simulate the drag PATCH: set epic = v-six.
        let payload = serde_json::json!({
            "expected_version": version,
            "epic": "v-six",
        });
        let resp = r
            .clone()
            .oneshot(
                Request::patch("/api/issues/drag-roundtrip-1")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 3. Re-fetch the board: card is now in the v-six column.
        let resp = r
            .oneshot(
                Request::get("/api/boards/release")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let issue = json["issues"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["slug"] == "drag-roundtrip-1")
            .cloned()
            .unwrap();
        assert_eq!(issue["group_value"], "v-six");
    }

    #[tokio::test]
    async fn api_board_unknown_name_404s() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/boards/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn api_board_unknown_group_by_renders_read_only() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        write_board(
            tmp.path(),
            "broken",
            "name: broken\ngroup_by: not_a_field\ncolumns: [{value: '', label: U}]\n",
        );
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/boards/broken")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(json["read_only"], true);
        let reasons = json["read_only_reasons"].as_array().unwrap();
        assert_eq!(reasons.len(), 1);
        assert!(reasons[0].as_str().unwrap().contains("not declared"));
    }

    #[tokio::test]
    async fn board_html_route_includes_board_name() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        write_board(
            tmp.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns: [{value: '', label: U}]\n",
        );
        let resp = make_router(tmp.path())
            .oneshot(Request::get("/board/triage").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("data-board-name=\"triage\""));
        // Bad slug shape → 404 before any disk access.
        let resp = make_router(tmp.path())
            .oneshot(Request::get("/board/INVALID").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        // Valid-shaped name with no matching YAML → also 404. Avoids
        // the "broken shell with confusing JS error" UX where the
        // route 200s and the API then 404s.
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/board/no-such-board")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        // Hard-validation error (bad YAML) → 404. Operator must fix
        // the file; the API call would 422.
        write_board(tmp.path(), "broken-yaml", "this is: not: valid yaml: : :");
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/board/broken-yaml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// Same drag-mutation round-trip as `board_drag_mutation_round_trip_via_patch`,
    /// but for a custom scalar field declared in `.schema.yaml`. The
    /// PATCH body uses `custom_fields` instead of a dedicated slot.
    /// Verifies the on-disk YAML is updated and the next board fetch
    /// resolves the new `group_value` server-side.
    #[tokio::test]
    async fn board_drag_mutation_round_trip_via_custom_fields() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        // Custom scalar field `team` declared in the schema so the
        // mutate layer accepts custom_fields writes for it.
        fs::write(
            tmp.path().join("issues").join(".schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: false\n",
        )
        .unwrap();
        seed_open_issue(tmp.path(), "drag-customfield-1");
        write_board(
            tmp.path(),
            "byteam",
            "name: byteam\ngroup_by: team\ncolumns:\n  - {value: '', label: Unscoped}\n  - {value: alpha, label: Alpha}\n",
        );
        let r = make_router(tmp.path());

        // 1. Initial board fetch: the issue starts in the unassigned bucket.
        let resp = r
            .clone()
            .oneshot(
                Request::get("/api/boards/byteam")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(json["builtin_group_by"], false, "team is a custom field");
        let issue = json["issues"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["slug"] == "drag-customfield-1")
            .cloned()
            .unwrap();
        assert_eq!(issue["group_value"], "");
        let version = issue["version"].as_str().unwrap().to_string();

        // 2. PATCH via custom_fields shape — what the JS sends for a
        //    custom-board drag.
        let payload = serde_json::json!({
            "expected_version": version,
            "custom_fields": { "team": "alpha" },
        });
        let resp = r
            .clone()
            .oneshot(
                Request::patch("/api/issues/drag-customfield-1")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 3. Re-fetch the board — server-side group_value resolution
        //    reads `team` from `extra` and surfaces it.
        let resp = r
            .clone()
            .oneshot(
                Request::get("/api/boards/byteam")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let issue = json["issues"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["slug"] == "drag-customfield-1")
            .cloned()
            .unwrap();
        assert_eq!(issue["group_value"], "alpha");

        // 4. Empty-bucket clear: drag back to "Unscoped" via
        //    custom_fields: { team: null }.
        let version = issue["version"].as_str().unwrap().to_string();
        let payload = serde_json::json!({
            "expected_version": version,
            "custom_fields": { "team": null },
        });
        let resp = r
            .clone()
            .oneshot(
                Request::patch("/api/issues/drag-customfield-1")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = r
            .oneshot(
                Request::get("/api/boards/byteam")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        let issue = json["issues"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["slug"] == "drag-customfield-1")
            .cloned()
            .unwrap();
        assert_eq!(issue["group_value"], "", "null clears the custom field");
    }

    /// Hard validation errors at `/api/boards/<name>` return 422 (not
    /// 404) so clients can distinguish "URL typo" from "broken YAML".
    #[tokio::test]
    async fn api_board_hard_validation_returns_422() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        // Required built-in (priority) with empty bucket: hard reject.
        write_board(
            tmp.path(),
            "broken",
            "name: broken\ngroup_by: priority\ncolumns:\n  - {value: '', label: U}\n  - {value: high, label: High}\n",
        );
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/boards/broken")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Filter-bar visibility config round-trips into `BoardResponse`.
    #[tokio::test]
    async fn api_board_surfaces_filters_config() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        write_board(
            tmp.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns: [{value: '', label: U}]\nfilters: [search, type]\n",
        );
        let resp = make_router(tmp.path())
            .oneshot(
                Request::get("/api/boards/triage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        assert_eq!(json["filters"], serde_json::json!(["search", "type"]));
    }
}
