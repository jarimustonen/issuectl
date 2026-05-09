//! Integration tests for `issuectl agents init`. These cover the
//! pieces an inline test cannot: process exit code, byte-level
//! stdout, JSON envelope shape, `--force` flag handling.

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
fn init_writes_agents_md_with_sentinels() {
    let tmp = fresh_repo();
    let out = run(tmp.path(), &["agents", "init"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let path = tmp.path().join(".issuectl/AGENTS.md");
    assert!(path.is_file());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("<!-- issuectl-managed:start -->"));
    assert!(text.contains("<!-- issuectl-managed:end -->"));
    assert!(text.contains("Never edit frontmatter manually"));
}

#[test]
fn init_refuses_to_overwrite_without_force() {
    let tmp = fresh_repo();
    assert_eq!(run(tmp.path(), &["agents", "init"]).status.code(), Some(0));
    let path = tmp.path().join(".issuectl/AGENTS.md");
    std::fs::write(&path, "user content\n").unwrap();
    let out = run(tmp.path(), &["agents", "init"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "user content\n");
}

#[test]
fn init_force_overwrites() {
    let tmp = fresh_repo();
    let path = tmp.path().join(".issuectl/AGENTS.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "stale\n").unwrap();
    let out = run(tmp.path(), &["agents", "init", "--force"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("<!-- issuectl-managed:start -->"));
    assert!(!text.contains("stale"));
}

#[test]
fn init_json_envelope_reports_wrote_true() {
    let tmp = fresh_repo();
    let out = run(tmp.path(), &["--json", "agents", "init"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json parse");
    assert_eq!(v["wrote"], serde_json::Value::Bool(true));
    assert_eq!(v["overwrote_existing"], serde_json::Value::Bool(false));
    assert_eq!(v["path"], ".issuectl/AGENTS.md");
}

#[test]
fn doctor_fix_regenerates_drifted_managed_block() {
    let tmp = fresh_repo();
    // Bootstrap an AGENTS.md against the default schema.
    assert_eq!(run(tmp.path(), &["agents", "init"]).status.code(), Some(0));
    let path = tmp.path().join(".issuectl/AGENTS.md");
    let original = std::fs::read_to_string(&path).unwrap();

    // Mutate the schema so the managed block falls out of sync.
    std::fs::write(
        tmp.path().join("issues/.schema.yaml"),
        "version: 1\nbody_sections:\n  bug:\n    - Reproduction\n",
    )
    .unwrap();

    // Read-only doctor should flag drift.
    let out = run(tmp.path(), &["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(".issuectl/AGENTS.md") && stdout.contains("out of date"),
        "expected drift line, got: {stdout}"
    );

    // --fix regenerates and exits clean.
    let out = run(tmp.path(), &["doctor", "--fix"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let after = std::fs::read_to_string(&path).unwrap();
    assert_ne!(after, original);
    assert!(after.contains("`## Reproduction`"));
    assert!(after.contains("<!-- issuectl-managed:start -->"));
}
