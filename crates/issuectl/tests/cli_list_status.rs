//! Regression tests for `@list-status-done`: `list --status <closing>`
//! must surface every issue that literally carries that status —
//! including archived ones — instead of printing `No issues found`.
//!
//! The bug: `cmd_list` applied an implicit `folder:open` default
//! whenever no *positional* query was given, so a `--status done`
//! flag (translated into a `status:done` term) was AND-ed against
//! `folder:open` and matched nothing, because closing statuses bucket
//! into the `closed` folder. The fix lets a positively-pinned
//! `status:`/`folder:` term step the open-only default out of the way,
//! mirroring `cmd_search`. See `crates/issuectl/src/main.rs` (`cmd_list`).

use std::process::{Command, Output};

use tempfile::TempDir;

fn fresh_repo() -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("issues")).expect("mkdir issues");
    tmp
}

fn run(root: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("NO_COLOR", "1")
        .current_dir(root)
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .expect("spawn issuectl")
}

fn run_ok(root: &std::path::Path, args: &[&str]) -> Output {
    let out = run(root, args);
    assert_eq!(
        out.status.code(),
        Some(0),
        "setup command failed; {}",
        dump(&out)
    );
    out
}

fn dump(out: &Output) -> String {
    format!(
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    )
}

/// Collect the `slug` of every issue in a `--json list` payload.
fn list_slugs(root: &std::path::Path, args: &[&str]) -> Vec<String> {
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("list --json stdout should be a JSON array");
    v.as_array()
        .expect("list --json emits an array")
        .iter()
        .map(|i| i["slug"].as_str().expect("slug is a string").to_string())
        .collect()
}

/// Create an issue of `issue_type` and close it to `status`. `done`
/// is a non-bug closing status, `fixed` is the bug one — the caller
/// picks a compatible type so the transition validator is happy.
fn new_closed(root: &std::path::Path, slug: &str, issue_type: &str, status: &str) {
    run_ok(
        root,
        &["new", "Title here", "--type", issue_type, "--slug", slug],
    );
    run_ok(root, &["close", slug, "--status", status]);
}

/// `list --status done` lists issues whose frontmatter status is
/// literally `done`, and `--status fixed` likewise — the flag is no
/// longer silently confined to the open folder.
#[test]
fn list_status_flag_finds_closing_statuses() {
    let tmp = fresh_repo();
    let root = tmp.path();

    new_closed(root, "done-one", "task", "done");
    new_closed(root, "done-two", "task", "done");
    new_closed(root, "fixed-one", "bug", "fixed");
    run_ok(
        root,
        &["new", "Open one", "--type", "bug", "--slug", "open-one"],
    );

    let done = list_slugs(root, &["--json", "list", "--status", "done"]);
    assert!(
        done.contains(&"done-one".to_string()) && done.contains(&"done-two".to_string()),
        "expected both done issues; got {done:?}"
    );
    assert!(
        !done.contains(&"fixed-one".to_string()) && !done.contains(&"open-one".to_string()),
        "done filter leaked other statuses; got {done:?}"
    );

    let fixed = list_slugs(root, &["--json", "list", "--status", "fixed"]);
    assert_eq!(fixed, vec!["fixed-one".to_string()], "got {fixed:?}");
}

/// The archive-aware common case: an issue archived to
/// `issues/archive/YYYY/MM/<slug>/` still matches `list --status done`.
#[test]
fn list_status_done_finds_archived_issue() {
    let tmp = fresh_repo();
    let root = tmp.path();

    new_closed(root, "archived-done", "task", "done");
    // Force it into cold storage regardless of close date.
    run_ok(root, &["archive", "--older-than", "0d"]);
    // Sanity: it really moved under issues/archive/.
    assert!(
        std::fs::read_dir(root.join("issues").join("archive")).is_ok(),
        "archive dir should exist after `archive`"
    );

    let done = list_slugs(root, &["--json", "list", "--status", "done"]);
    assert!(
        done.contains(&"archived-done".to_string()),
        "archived done issue must be listable via --status done; got {done:?}"
    );
}

/// Guard the backward-compatible default: bare `list` (no flags, no
/// query) stays open-only and must NOT surface closed issues.
#[test]
fn bare_list_stays_open_only() {
    let tmp = fresh_repo();
    let root = tmp.path();

    new_closed(root, "closed-one", "task", "done");
    run_ok(
        root,
        &["new", "Still open", "--type", "bug", "--slug", "still-open"],
    );

    let slugs = list_slugs(root, &["--json", "list"]);
    assert_eq!(slugs, vec!["still-open".to_string()], "got {slugs:?}");
}
