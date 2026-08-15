//! Regression test for `@sync-commits-empty-main`: run directly on
//! `main`, the default range `<merge-base(HEAD, main)>..HEAD` collapses
//! to `HEAD..HEAD` and scans zero commits. That must NOT look silently
//! successful — `sync-commits` surfaces a `warnings[]` entry (in `--json`
//! and text) and still exits 0 (an empty range is surfaced, not an error).
//!
//! Convention for `tests/` vs inline `#[cfg(test)]`: see `AGENTS.md`
//! (`Tests`) — this locks byte-level stdout/stderr/exit behaviour.

use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("spawn git");
    assert!(st.success(), "git {args:?} failed");
}

/// A git repo on `main` with an empty `issues/` dir.
fn fresh_repo() -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("issues")).expect("mkdir issues");
    git(root, &["init", "-q", "-b", "main"]);
    for (k, v) in [("user.email", "t@example.com"), ("user.name", "t")] {
        git(root, &["config", "--local", k, v]);
    }
    tmp
}

fn run(root: &Path, args: &[&str]) -> Output {
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

fn dump(out: &Output) -> String {
    format!(
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    )
}

#[test]
fn empty_default_range_on_main_warns_and_exits_zero_json() {
    let tmp = fresh_repo();
    let root = tmp.path();
    // Create an issue and land a commit on `main`.
    let new = run(
        root,
        &["--json", "new", "--type", "improvement", "record the thing"],
    );
    assert_eq!(new.status.code(), Some(0), "{}", dump(&new));
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "seed on main"]);

    // Bare (default-range) sync on `main`: merge-base(HEAD, main) == HEAD,
    // so the default collapses to an empty `HEAD..HEAD`.
    let out = run(root, &["--json", "sync-commits"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "empty default range is surfaced, not an error; {}",
        dump(&out)
    );

    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("sync-commits --json stdout should be JSON");
    let warnings = v["warnings"]
        .as_array()
        .expect("payload should carry a warnings[] array");
    assert!(
        warnings.iter().any(|w| w
            .as_str()
            .is_some_and(|s| s.contains("default range") && s.contains("HEAD~1..HEAD"))),
        "expected an empty-default-range warning; {}",
        dump(&out)
    );
}

#[test]
fn empty_default_range_on_main_warns_in_text_mode() {
    let tmp = fresh_repo();
    let root = tmp.path();
    let new = run(
        root,
        &["--json", "new", "--type", "improvement", "record the thing"],
    );
    assert_eq!(new.status.code(), Some(0), "{}", dump(&new));
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "seed on main"]);

    let out = run(root, &["sync-commits"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Warning:") && stderr.contains("default range"),
        "expected a text-mode warning on stderr; {}",
        dump(&out)
    );
}

#[test]
fn explicit_empty_range_does_not_warn() {
    let tmp = fresh_repo();
    let root = tmp.path();
    let new = run(
        root,
        &["--json", "new", "--type", "improvement", "record the thing"],
    );
    assert_eq!(new.status.code(), Some(0), "{}", dump(&new));
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "seed on main"]);

    // An explicit, legitimately-empty range is the user's choice — respect it.
    let out = run(root, &["--json", "sync-commits", "--range", "HEAD..HEAD"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout should be JSON");
    let warnings = v["warnings"].as_array().expect("warnings[] array");
    assert!(
        warnings.is_empty(),
        "explicit --range should not trigger the empty-default warning; {}",
        dump(&out)
    );
}
