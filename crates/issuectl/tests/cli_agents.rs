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
    let v = v["data"].clone();
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

    // Re-run scan-only doctor and assert drift is cleared.
    let out = run(tmp.path(), &["--json", "doctor"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json parse");
    let v = v["data"].clone();
    assert_eq!(v["agents_md_drift"], serde_json::Value::Bool(false));
    assert!(v["agents_md_malformed"].is_null());
    assert!(v["agents_md_check_skipped"].is_null());
}

#[test]
fn doctor_refuses_malformed_agents_md_and_blocks_exit() {
    let tmp = fresh_repo();
    let path = tmp.path().join(".issuectl/AGENTS.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // Two managed blocks → malformed.
    let bad = "Prose.\n\n<!-- issuectl-managed:start -->\nA\n<!-- issuectl-managed:end -->\n\n<!-- issuectl-managed:start -->\nB\n<!-- issuectl-managed:end -->\n";
    std::fs::write(&path, bad).unwrap();

    // Read-only doctor flags malformed and exits non-zero (critical).
    let out = run(tmp.path(), &["doctor"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("malformed"), "{stdout}");

    // --fix refuses to mutate a malformed file. Bytes unchanged.
    let out = run(tmp.path(), &["doctor", "--fix"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(after, bad);
}

#[test]
fn doctor_skips_drift_check_on_schema_parse_error() {
    let tmp = fresh_repo();
    // Bootstrap a clean AGENTS.md against the default schema first.
    assert_eq!(run(tmp.path(), &["agents", "init"]).status.code(), Some(0));
    let path = tmp.path().join(".issuectl/AGENTS.md");
    let original = std::fs::read_to_string(&path).unwrap();

    // Now corrupt the schema file with invalid YAML.
    std::fs::write(
        tmp.path().join("issues/.schema.yaml"),
        "version: 1\nfields: not-a-mapping\n",
    )
    .unwrap();

    // doctor --fix must NOT regenerate AGENTS.md from defaults; the
    // file should be untouched.
    let out = run(tmp.path(), &["doctor", "--fix"]);
    // Schema parse error is critical → exit 1 expected.
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(after, original, "AGENTS.md must not be rewritten");
}
