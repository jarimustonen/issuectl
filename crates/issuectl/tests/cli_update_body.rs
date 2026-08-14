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

fn show_body(root: &std::path::Path, slug: &str) -> String {
    let show = run(root, &["--json", "show", slug]);
    assert_eq!(show.status.code(), Some(0), "{}", dump(&show));
    serde_json::from_slice::<serde_json::Value>(&show.stdout).expect("show stdout should be JSON")
        ["body"]
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
        body.contains("REPLACEMENT inline.") && !body.contains("ORIGINAL-BODY-MARKER"),
        "inline replace failed: {body:?}"
    );
}

#[test]
fn update_body_and_frontmatter_apply_atomically() {
    // A body replacement bundled with a frontmatter PATCH lands in one
    // call (single flock): the new body AND the new priority are both
    // present afterwards, proving `set_body` rides the `update_issue`
    // write path rather than a second, separate write.
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

    let body = show_body(tmp.path(), "ub-combo");
    assert!(body.contains("REPLACEMENT combined."), "body: {body:?}");
    let show = run(tmp.path(), &["--json", "show", "ub-combo"]);
    let json: serde_json::Value = serde_json::from_slice(&show.stdout).expect("json");
    assert_eq!(json["priority"].as_str(), Some("high"), "{}", dump(&show));
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
