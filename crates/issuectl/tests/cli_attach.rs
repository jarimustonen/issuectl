//! Black-box CLI coverage for `issuectl attach`. The handler logic and
//! collision/error taxonomy are covered inline in
//! `mutate::attach::tests`; this file pins the `--json` envelope shape,
//! the exit-code contract (`0` happy / `1` validation), and the
//! stdout-vs-stderr split that agents rely on.

use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

fn fresh_repo_with_issue(slug: &str) -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("issues").join(slug);
    std::fs::create_dir_all(&dir).expect("mkdir issue");
    std::fs::write(
        dir.join("item.md"),
        format!("---\nstatus: open\n---\n\n# {slug}\n"),
    )
    .expect("write item.md");
    tmp
}

fn run(root: &Path, args: &[&str]) -> Output {
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

#[test]
fn attach_json_success_emits_bare_payload_on_stdout() {
    let tmp = fresh_repo_with_issue("calm-quiet-otter");
    let src = tmp.path().join("shot.png");
    std::fs::write(&src, b"PNGDATA").unwrap();

    let out = run(
        tmp.path(),
        &["--json", "attach", "calm-quiet-otter", src.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    assert!(out.stderr.is_empty(), "stderr={:?}", out.stderr);

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid json");
    assert_eq!(v["slug"], "calm-quiet-otter");
    assert_eq!(v["dir"], "issues/calm-quiet-otter");
    let attached = v["attached"].as_array().expect("attached array");
    assert_eq!(attached.len(), 1);
    assert_eq!(attached[0]["name"], "shot.png");
    assert_eq!(attached[0]["renamed"], false);
    assert_eq!(
        attached[0]["path"],
        "issues/calm-quiet-otter/attachments/shot.png"
    );
    assert!(tmp
        .path()
        .join("issues/calm-quiet-otter/attachments/shot.png")
        .is_file());
}

#[test]
fn attach_json_unknown_slug_emits_error_envelope_exit_1() {
    let tmp = fresh_repo_with_issue("calm-quiet-otter");
    let src = tmp.path().join("note.txt");
    std::fs::write(&src, b"hi").unwrap();

    let out = run(
        tmp.path(),
        &["--json", "attach", "no-such-slug", src.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "stdout={:?}", out.stdout);
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("valid json error");
    assert_eq!(v["error"]["code"], "validation");
}
