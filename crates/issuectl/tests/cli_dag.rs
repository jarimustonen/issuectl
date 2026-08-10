//! Black-box CLI coverage for `issuectl dag`. The scheduling logic is
//! covered inline in `dag::tests`; this file pins the CLI envelope:
//! the `--json` shape (`schema_version`, lanes/unscheduled, head-of-line,
//! spawnable), the `lane`/`collision` round-trip through `update`, and the
//! `--reservations` input (inline JSON + stdin).

use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

fn fresh_repo() -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("issues")).unwrap();
    tmp
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .current_dir(root)
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .expect("spawn issuectl")
}

fn run_stdin(root: &Path, args: &[&str], stdin: &str) -> Output {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env("LC_ALL", "C")
        .current_dir(root)
        .arg("--root")
        .arg(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn issuectl");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().expect("wait")
}

fn new_issue(root: &Path, slug: &str) {
    let out = run(
        root,
        &[
            "--json", "new", "--slug", slug, "--title", slug, "--type", "task",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "new {slug}: {out:?}");
}

fn json(out: &Output) -> serde_json::Value {
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    serde_json::from_slice(&out.stdout).expect("json stdout")
}

/// Build a two-lane repo: schema(a→b) + main(x), plus one unscheduled.
fn scheduled_repo() -> TempDir {
    let tmp = fresh_repo();
    let r = tmp.path();
    for slug in ["schema-a", "schema-b", "main-x", "loose-one"] {
        new_issue(r, slug);
    }
    run(r, &["--json", "update", "schema-a", "--lane", "schema"]);
    run(
        r,
        &[
            "--json",
            "update",
            "schema-b",
            "--lane",
            "schema",
            "--add-collision",
            "shared.rs",
        ],
    );
    run(r, &["--json", "update", "main-x", "--lane", "main-rs"]);
    run(
        r,
        &[
            "--json",
            "depend",
            "add",
            "schema-b",
            "--blocked-by",
            "schema-a",
        ],
    );
    tmp
}

#[test]
fn dag_json_shape_and_head_of_line() {
    let tmp = scheduled_repo();
    let v = json(&run(tmp.path(), &["--json", "dag"]));
    assert_eq!(v["schema_version"].as_u64(), Some(1));
    assert_eq!(v["reservations_applied"], serde_json::json!(false));

    let lanes = v["lanes"].as_array().expect("lanes array");
    // Lanes are sorted by name: main-rs before schema.
    assert_eq!(lanes[0]["lane"], "main-rs");
    assert_eq!(lanes[1]["lane"], "schema");

    let schema = &lanes[1];
    assert_eq!(schema["head_of_line"], "schema-a");
    let issues = schema["issues"].as_array().unwrap();
    assert_eq!(issues[0]["slug"], "schema-a");
    assert_eq!(issues[0]["is_head_of_line"], serde_json::json!(true));
    assert_eq!(issues[0]["spawnable"], serde_json::json!(true));
    // schema-b sits behind the head and carries its blocker + collision.
    assert_eq!(issues[1]["slug"], "schema-b");
    assert_eq!(issues[1]["is_head_of_line"], serde_json::json!(false));
    assert_eq!(issues[1]["blocked_by"], serde_json::json!(["schema-a"]));
    assert_eq!(issues[1]["collision"], serde_json::json!(["shared.rs"]));

    // Unscheduled bucket carries the lane-less issue, its own head-of-line.
    let uns = v["unscheduled"].as_array().unwrap();
    assert_eq!(uns.len(), 1);
    assert_eq!(uns[0]["slug"], "loose-one");
    assert_eq!(uns[0]["spawnable"], serde_json::json!(true));
}

#[test]
fn dag_reservations_inline_json_marks_reserved() {
    let tmp = scheduled_repo();
    let v = json(&run(
        tmp.path(),
        &["--json", "dag", "--reservations", r#"{"lanes":["schema"]}"#],
    ));
    assert_eq!(v["reservations_applied"], serde_json::json!(true));
    let schema = v["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["lane"] == "schema")
        .unwrap();
    let head = &schema["issues"][0];
    assert_eq!(head["reserved"], serde_json::json!(true));
    assert_eq!(head["spawnable"], serde_json::json!(false));
}

#[test]
fn dag_reservations_from_stdin_by_collision_token() {
    let tmp = scheduled_repo();
    let out = run_stdin(
        tmp.path(),
        &["--json", "dag", "--reservations", "-"],
        r#"[{"run_id":"r1","collision":["shared.rs"]}]"#,
    );
    let v = json(&out);
    let schema = v["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["lane"] == "schema")
        .unwrap();
    // schema-b holds the reserved collision token → reserved.
    let b = schema["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["slug"] == "schema-b")
        .unwrap();
    assert_eq!(b["reserved"], serde_json::json!(true));
}

#[test]
fn dag_reject_field_lane_points_at_dedicated_flag() {
    let tmp = fresh_repo();
    new_issue(tmp.path(), "foo-bar");
    let out = run(
        tmp.path(),
        &["--json", "update", "foo-bar", "--field", "lane=x"],
    );
    assert_ne!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("built-in") && stderr.contains("--lane"),
        "expected reserved-key hint, got: {stderr}"
    );
}

#[test]
fn dag_invalid_reservations_errors() {
    let tmp = scheduled_repo();
    let out = run(
        tmp.path(),
        &["--json", "dag", "--reservations", "{not json"],
    );
    assert_ne!(out.status.code(), Some(0), "invalid reservations must fail");
}
