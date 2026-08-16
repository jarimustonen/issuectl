//! End-to-end tests for `issuectl update`'s body-set flags
//! (`--description`/`--body`/`--body-file`), the `update` counterpart to
//! `new`'s body sources. They lock the DONE criteria of the
//! `update-set-body-flag` issue: setting/replacing an existing issue's
//! body from a file and from stdin (`--body-file -`), with the same flag
//! names and semantics as `new`.

use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

/// Tempdir with an empty `issues/` dir; the first `new` bootstraps the
/// default `.schema.yaml`.
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
        .current_dir(root)
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .expect("spawn issuectl")
}

fn dump(out: &Output) -> String {
    format!(
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    )
}

fn show_json(root: &std::path::Path, slug: &str) -> serde_json::Value {
    let show = run(root, &["--json", "show", slug]);
    assert_eq!(show.status.code(), Some(0), "{}", dump(&show));
    serde_json::from_slice::<serde_json::Value>(&show.stdout).expect("show stdout should be JSON")
        ["data"]
        .clone()
}

fn show_body(root: &std::path::Path, slug: &str) -> String {
    show_json(root, slug)["body"]
        .as_str()
        .expect("body field")
        .to_string()
}

/// Create an issue whose starting body carries a distinctive marker so
/// the replacement tests can prove the old body is gone, not merely that
/// the new text is present.
fn new_issue_with_starter_body(root: &std::path::Path, slug: &str) {
    let out = run(
        root,
        &[
            "new",
            "--type",
            "feature",
            "--title",
            "Body target",
            "--slug",
            slug,
            "--description",
            "ORIGINAL-BODY-MARKER",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert!(
        show_body(root, slug).contains("ORIGINAL-BODY-MARKER"),
        "starter body did not land"
    );
}

#[test]
fn update_body_file_replaces_existing_body() {
    let tmp = fresh_repo();
    new_issue_with_starter_body(tmp.path(), "ub-file");

    let notes = tmp.path().join("replacement.md");
    std::fs::write(&notes, "REPLACEMENT from a file.\n").expect("write notes");
    let out = run(
        tmp.path(),
        &["update", "ub-file", "--body-file", notes.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    let body = show_body(tmp.path(), "ub-file");
    assert!(
        body.contains("REPLACEMENT from a file."),
        "new body missing: {body:?}"
    );
    // REPLACE, not append: the original body must be gone.
    assert!(
        !body.contains("ORIGINAL-BODY-MARKER"),
        "old body survived a replace: {body:?}"
    );
}

#[test]
fn update_body_file_dash_reads_stdin() {
    use std::io::Write;

    let tmp = fresh_repo();
    new_issue_with_starter_body(tmp.path(), "ub-stdin");

    let mut child = Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .current_dir(tmp.path())
        .arg("--root")
        .arg(tmp.path())
        .args(["update", "ub-stdin", "--body-file", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn issuectl");
    child
        .stdin
        .take()
        .expect("stdin handle")
        .write_all(b"REPLACEMENT piped via stdin.\n")
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    let body = show_body(tmp.path(), "ub-stdin");
    assert!(
        body.contains("REPLACEMENT piped via stdin."),
        "stdin body missing: {body:?}"
    );
    assert!(
        !body.contains("ORIGINAL-BODY-MARKER"),
        "old body survived a stdin replace: {body:?}"
    );
}

#[test]
fn update_description_flag_replaces_body_inline() {
    let tmp = fresh_repo();
    new_issue_with_starter_body(tmp.path(), "ub-inline");

    let out = run(
        tmp.path(),
        &[
            "update",
            "ub-inline",
            "--description",
            "REPLACEMENT inline.",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    let body = show_body(tmp.path(), "ub-inline");
    assert!(
        body.contains("REPLACEMENT inline."),
        "new body missing: {body:?}"
    );
    assert!(
        !body.contains("ORIGINAL-BODY-MARKER"),
        "old body survived inline replace: {body:?}"
    );
}

#[test]
fn update_body_alias_replaces_body_inline() {
    // `--body` is the documented alias for `--description`; exercise the
    // public alias spelling, not just the canonical flag.
    let tmp = fresh_repo();
    new_issue_with_starter_body(tmp.path(), "ub-alias");

    let out = run(
        tmp.path(),
        &["update", "ub-alias", "--body", "REPLACEMENT via alias."],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    let body = show_body(tmp.path(), "ub-alias");
    assert!(
        body.contains("REPLACEMENT via alias."),
        "new body missing: {body:?}"
    );
    assert!(
        !body.contains("ORIGINAL-BODY-MARKER"),
        "old body survived alias replace: {body:?}"
    );
}

#[test]
fn update_body_file_missing_path_errors_cleanly() {
    let tmp = fresh_repo();
    new_issue_with_starter_body(tmp.path(), "ub-missing");
    let out = run(
        tmp.path(),
        &[
            "update",
            "ub-missing",
            "--body-file",
            tmp.path().join("does-not-exist.md").to_str().unwrap(),
        ],
    );
    // A missing file is a clean runtime error (exit 1), not a panic, and
    // the original body is left intact.
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(
        show_body(tmp.path(), "ub-missing").contains("ORIGINAL-BODY-MARKER"),
        "body must be untouched after a failed update"
    );
}

#[test]
fn update_empty_body_file_is_rejected_and_leaves_body_intact() {
    let tmp = fresh_repo();
    new_issue_with_starter_body(tmp.path(), "ub-empty");
    let empty = tmp.path().join("empty.md");
    std::fs::write(&empty, "   \n").expect("write empty");
    let out = run(
        tmp.path(),
        &["update", "ub-empty", "--body-file", empty.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(
        show_body(tmp.path(), "ub-empty").contains("ORIGINAL-BODY-MARKER"),
        "body must be untouched after a rejected empty replacement"
    );
}

#[test]
fn update_body_and_frontmatter_land_in_one_call() {
    // A body replacement bundled with a frontmatter PATCH: the new body
    // AND the new priority are both present after a single `update`
    // invocation. (The single-flock / single-write property is proven at
    // the core level by `update_issue_set_body_composes_with_frontmatter
    // _patch_atomically`, which observes both landing from one call.)
    let tmp = fresh_repo();
    new_issue_with_starter_body(tmp.path(), "ub-combo");

    let out = run(
        tmp.path(),
        &[
            "update",
            "ub-combo",
            "--description",
            "REPLACEMENT combined.",
            "--priority",
            "high",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    let json = show_json(tmp.path(), "ub-combo");
    assert!(
        json["body"]
            .as_str()
            .unwrap()
            .contains("REPLACEMENT combined."),
        "body: {json}"
    );
    assert_eq!(json["priority"].as_str(), Some("high"), "priority: {json}");
}

#[test]
fn update_body_alias_conflicts_with_body_file() {
    // The mutual-exclusion group must hold for the `--body` spelling too,
    // not only `--description`.
    let tmp = fresh_repo();
    new_issue_with_starter_body(tmp.path(), "ub-alias-conflict");
    let notes = tmp.path().join("notes.md");
    std::fs::write(&notes, "x\n").expect("write notes");
    let out = run(
        tmp.path(),
        &[
            "update",
            "ub-alias-conflict",
            "--body-file",
            notes.to_str().unwrap(),
            "--body",
            "also inline",
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
    // Body untouched — a clap usage error mutates nothing.
    assert!(
        show_body(tmp.path(), "ub-alias-conflict").contains("ORIGINAL-BODY-MARKER"),
        "body must be untouched after a usage error"
    );
}

#[test]
fn update_body_reserved_heading_warns_nonfatally_on_both_channels() {
    // Replacing the body with a reserved-legacy `## Notes` heading is
    // accepted (exit 0) but warns — on stderr for humans and in the JSON
    // `warnings` array for machines.
    let tmp = fresh_repo();
    new_issue_with_starter_body(tmp.path(), "ub-warn");
    let notes = tmp.path().join("legacy.md");
    std::fs::write(&notes, "Body.\n\n## Notes\nlegacy section.\n").expect("write notes");

    // Human channel: warning on stderr, still exit 0.
    let human = run(
        tmp.path(),
        &["update", "ub-warn", "--body-file", notes.to_str().unwrap()],
    );
    assert_eq!(human.status.code(), Some(0), "{}", dump(&human));
    let stderr = String::from_utf8_lossy(&human.stderr);
    assert!(
        stderr.contains("## Notes"),
        "warning missing on stderr: {stderr}"
    );

    // JSON channel: warning present in the `warnings` array.
    let json_out = run(
        tmp.path(),
        &[
            "--json",
            "update",
            "ub-warn",
            "--body-file",
            notes.to_str().unwrap(),
        ],
    );
    assert_eq!(json_out.status.code(), Some(0), "{}", dump(&json_out));
    let json: serde_json::Value = serde_json::from_slice(&json_out.stdout).expect("json");
    let warnings = json["warnings"].as_array().expect("warnings array");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().is_some_and(|s| s.contains("## Notes"))),
        "warning missing in JSON: {json}"
    );
}

#[test]
fn update_body_file_conflicts_with_description_is_clap_usage_error() {
    let tmp = fresh_repo();
    new_issue_with_starter_body(tmp.path(), "ub-conflict");
    let notes = tmp.path().join("notes.md");
    std::fs::write(&notes, "x\n").expect("write notes");
    let out = run(
        tmp.path(),
        &[
            "update",
            "ub-conflict",
            "--body-file",
            notes.to_str().unwrap(),
            "--description",
            "also inline",
        ],
    );
    // clap usage error exits 2, and the flags are named mutually exclusive.
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--body-file") && stderr.contains("cannot be used with"),
        "unexpected stderr: {stderr}"
    );
}
