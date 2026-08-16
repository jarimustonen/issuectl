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
    serde_json::from_slice::<serde_json::Value>(&out.stdout).expect("json stdout")["data"].clone()
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
fn dag_excludes_closed_issue_from_unscheduled() {
    // `dag` is a scheduling view: a closed (terminal-status) issue can never
    // be scheduled, so it must not surface in the unscheduled bucket. Only
    // the still-open lane-less issue is listed.
    let tmp = fresh_repo();
    let r = tmp.path();
    new_issue(r, "open-one");
    new_issue(r, "shipped-one");
    let out = run(r, &["--json", "close", "shipped-one", "--status", "done"]);
    assert_eq!(out.status.code(), Some(0), "close: {out:?}");

    let v = json(&run(r, &["--json", "dag"]));
    let uns = v["unscheduled"].as_array().expect("unscheduled array");
    let slugs: Vec<&str> = uns.iter().map(|i| i["slug"].as_str().unwrap()).collect();
    assert_eq!(slugs, vec!["open-one"], "closed issue must be excluded");
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

#[test]
fn dag_no_lane_and_remove_collision_round_trip() {
    let tmp = fresh_repo();
    let r = tmp.path();
    new_issue(r, "foo-bar");
    run(
        r,
        &[
            "--json",
            "update",
            "foo-bar",
            "--lane",
            "schema",
            "--add-collision",
            "a.rs",
            "--add-collision",
            "b.rs",
        ],
    );
    // Remove one collision token and confirm the other survives.
    run(
        r,
        &["--json", "update", "foo-bar", "--remove-collision", "a.rs"],
    );
    let v = json(&run(r, &["--json", "show", "foo-bar"]));
    assert_eq!(v["lane"], "schema");
    assert_eq!(v["collision"], serde_json::json!(["b.rs"]));
    // Removing the last collision token drops the frontmatter key entirely.
    run(
        r,
        &["--json", "update", "foo-bar", "--remove-collision", "b.rs"],
    );
    // Clear the lane.
    run(r, &["--json", "update", "foo-bar", "--no-lane"]);
    let v = json(&run(r, &["--json", "show", "foo-bar"]));
    assert_eq!(v["lane"], serde_json::Value::Null);
    assert_eq!(v["collision"], serde_json::Value::Null);
}

#[test]
fn dag_fields_ok_on_pre_existing_v1_schema_without_them() {
    // A repo whose committed `.schema.yaml` predates lane/collision (it
    // does not declare them) must still accept `update --lane` and pass
    // `doctor` clean — the built-in default schema always contributes the
    // fields, so doctor's unknown-key check recognises them.
    let tmp = fresh_repo();
    let r = tmp.path();
    std::fs::write(
        r.join("issues").join(".schema.yaml"),
        "version: 1\nfields:\n  status:\n    required: true\n    enum: [open, done]\n",
    )
    .unwrap();
    new_issue(r, "foo-bar");
    let out = run(r, &["--json", "update", "foo-bar", "--lane", "schema"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "update --lane on old schema: {out:?}"
    );
    // doctor must not flag `lane` as an unknown key.
    let doc = run(r, &["--json", "doctor"]);
    let v: serde_json::Value = serde_json::from_slice(&doc.stdout).expect("doctor json");
    let v = v["data"].clone();
    let unknown = v["unknown_keys"].as_array().cloned().unwrap_or_default();
    assert!(
        !unknown.iter().any(|u| u.to_string().contains("lane")),
        "lane flagged as unknown key: {unknown:?}"
    );
}
