//! Black-box CLI coverage for `issuectl cycle`. The rollup logic is
//! covered inline in `cycle::tests`; this file pins the CLI envelope:
//! `--json` shapes, `current` alias semantics, and the implicit
//! open-only default on `cycle plan`.

use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

/// Write an issue with a status of `open` (or `done` when `closed` is
/// requested — the closed bucket is derived from the status, not a
/// folder field). `extra_fm` is appended verbatim before the trailing
/// `---` and must not redeclare `status`/`type`/`priority`.
fn write_issue(root: &Path, slug: &str, folder: &str, extra_fm: &str) {
    let dir = root.join("issues").join(slug);
    std::fs::create_dir_all(&dir).expect("mkdir issue");
    let status = if folder == "closed" { "done" } else { "open" };
    let mut fm = format!("status: {status}\ntype: bug\npriority: normal\n");
    if folder == "closed" {
        fm.push_str("closed: 2026-05-30\n");
    }
    fm.push_str(extra_fm);
    std::fs::write(dir.join("item.md"), format!("---\n{fm}---\n\n# {slug}\n"))
        .expect("write item.md");
}

fn fresh_repo() -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("issues")).unwrap();
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
fn cycle_current_json_returns_iso_week_shape() {
    let tmp = fresh_repo();
    let out = run(tmp.path(), &["--json", "cycle", "current"]);
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let label = v["cycle"].as_str().expect("cycle string");
    assert!(
        label.len() == 8 && &label[4..6] == "-W",
        "expected YYYY-Www, got {label:?}"
    );
}

#[test]
fn cycle_plan_lists_only_matching_open_issues() {
    let tmp = fresh_repo();
    write_issue(tmp.path(), "red-ant", "open", "cycle: 2026-W22\n");
    write_issue(tmp.path(), "blue-bat", "open", "cycle: 2026-W23\n");
    // Closed issue in the same cycle — should be hidden without --all.
    write_issue(tmp.path(), "green-cat", "closed", "cycle: 2026-W22\n");

    let out = run(tmp.path(), &["--json", "cycle", "plan", "2026-W22"]);
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["cycle"], "2026-W22");
    let slugs: Vec<&str> = v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, vec!["red-ant"]);
}

#[test]
fn cycle_plan_with_all_includes_closed() {
    let tmp = fresh_repo();
    write_issue(tmp.path(), "red-ant", "open", "cycle: W1\n");
    write_issue(tmp.path(), "blue-bat", "closed", "cycle: W1\n");
    let out = run(tmp.path(), &["--json", "cycle", "plan", "W1", "--all"]);
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let mut slugs: Vec<&str> = v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["slug"].as_str().unwrap())
        .collect();
    slugs.sort();
    assert_eq!(slugs, vec!["blue-bat", "red-ant"]);
}

#[test]
fn cycle_status_rolls_up_open_and_closed() {
    let tmp = fresh_repo();
    write_issue(tmp.path(), "red-ant", "open", "cycle: W1\n");
    write_issue(tmp.path(), "blue-bat", "open", "cycle: W1\n");
    write_issue(tmp.path(), "green-cat", "closed", "cycle: W1\n");
    write_issue(tmp.path(), "gray-dog", "open", "cycle: W2\n");

    let out = run(tmp.path(), &["--json", "cycle", "status", "W1"]);
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["cycle"], "W1");
    assert_eq!(v["open"], 2);
    assert_eq!(v["closed"], 1);
    assert_eq!(v["total"], 3);
}

#[test]
fn cycle_status_all_lists_every_cycle() {
    let tmp = fresh_repo();
    write_issue(tmp.path(), "red-ant", "open", "cycle: W1\n");
    write_issue(tmp.path(), "blue-bat", "open", "cycle: W2\n");
    write_issue(tmp.path(), "green-cat", "open", ""); // no cycle

    let out = run(tmp.path(), &["--json", "cycle", "status", "--all"]);
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let arr = v.as_array().expect("array");
    let labels: Vec<&str> = arr.iter().map(|r| r["cycle"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["W1", "W2"]);
}

#[test]
fn cycle_status_default_uses_current() {
    let tmp = fresh_repo();
    // No matching issues: just verifies the command resolves "no name"
    // to today's ISO week without erroring, and the rollup shape is
    // present with zeros.
    let out = run(tmp.path(), &["--json", "cycle", "status"]);
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(v["cycle"].as_str().is_some_and(|s| s.contains("-W")));
    assert_eq!(v["total"], 0);
}

#[test]
fn cycle_plan_current_alias_resolves_today() {
    let tmp = fresh_repo();
    // Grab today's label first, then store an issue under it.
    let current = run(tmp.path(), &["--json", "cycle", "current"]);
    let cur_v: serde_json::Value = serde_json::from_slice(&current.stdout).unwrap();
    let label = cur_v["cycle"].as_str().unwrap().to_string();

    write_issue(tmp.path(), "red-ant", "open", &format!("cycle: {label}\n"));

    let out = run(tmp.path(), &["--json", "cycle", "plan", "current"]);
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["cycle"], label);
    assert_eq!(v["issues"].as_array().unwrap().len(), 1);
}
