//! Black-box CLI coverage for `issuectl epic tree`. The tree-building
//! logic is covered inline in `epic_tree::tests`; this file pins the CLI
//! envelope: the single-epic `--json` node shape, the no-slug forest
//! array, the human-readable indentation, and the `not-found` exit code /
//! error envelope for a missing slug.

use std::path::Path;
use std::process::{Command, Output};

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

fn new_epic(root: &Path, slug: &str) {
    let out = run(
        root,
        &[
            "--json", "new", "--slug", slug, "--title", slug, "--type", "epic",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "new epic {slug}: {out:?}");
}

fn new_child(root: &Path, slug: &str, epic: &str) {
    let out = run(
        root,
        &[
            "--json", "new", "--slug", slug, "--title", slug, "--type", "task", "--epic", epic,
        ],
    );
    assert_eq!(out.status.code(), Some(0), "new child {slug}: {out:?}");
}

fn json(out: &Output) -> serde_json::Value {
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    serde_json::from_slice(&out.stdout).expect("json stdout")
}

/// One epic with two children plus one unrelated issue.
fn epic_repo() -> TempDir {
    let tmp = fresh_repo();
    let r = tmp.path();
    new_epic(r, "big-epic");
    new_child(r, "zebra-task", "big-epic");
    new_child(r, "alpha-task", "big-epic");
    // Unrelated top-level issue that must not appear under the epic.
    let out = run(
        r,
        &[
            "--json",
            "new",
            "--slug",
            "loose-one",
            "--title",
            "loose-one",
            "--type",
            "task",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "new loose-one: {out:?}");
    tmp
}

#[test]
fn tree_json_emits_epic_with_sorted_children() {
    let tmp = epic_repo();
    let out = run(tmp.path(), &["--json", "epic", "tree", "big-epic"]);
    let v = json(&out);
    assert_eq!(v["slug"], "big-epic");
    assert_eq!(v["type"], "epic");
    let kids: Vec<&str> = v["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["slug"].as_str().unwrap())
        .collect();
    assert_eq!(kids, vec!["alpha-task", "zebra-task"]);
    // The unrelated issue is not pulled in.
    assert!(kids.iter().all(|s| *s != "loose-one"));
}

#[test]
fn tree_human_indents_children() {
    let tmp = epic_repo();
    let out = run(tmp.path(), &["epic", "tree", "big-epic"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("@big-epic"), "root line: {text}");
    // Children are drawn with box connectors and appear below the root.
    assert!(text.contains("├─ @alpha-task"), "first child: {text}");
    assert!(text.contains("└─ @zebra-task"), "last child: {text}");
    assert!(text.contains("2 descendants"), "summary line: {text}");
}

#[test]
fn tree_prefix_resolves_unique_slug() {
    let tmp = epic_repo();
    // `big` is a unique prefix of `big-epic`.
    let out = run(tmp.path(), &["--json", "epic", "tree", "big"]);
    let v = json(&out);
    assert_eq!(v["slug"], "big-epic");
}

#[test]
fn tree_missing_slug_is_not_found() {
    let tmp = epic_repo();
    let out = run(tmp.path(), &["--json", "epic", "tree", "ghost-epic"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty(), "stdout must be empty on error");
    let err: serde_json::Value = serde_json::from_slice(&out.stderr).expect("json stderr");
    assert_eq!(err["error"]["code"], "not-found");
}

#[test]
fn tree_no_slug_lists_forest_of_top_level_epics() {
    let tmp = fresh_repo();
    let r = tmp.path();
    new_epic(r, "epic-two");
    new_epic(r, "epic-one");
    // A nested epic must appear under its parent, not as a forest root.
    let out = run(
        r,
        &[
            "--json",
            "new",
            "--slug",
            "nested-epic",
            "--title",
            "nested-epic",
            "--type",
            "epic",
            "--epic",
            "epic-one",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "new nested-epic: {out:?}");

    let out = run(r, &["--json", "epic", "tree"]);
    let v = json(&out);
    let roots: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["slug"].as_str().unwrap())
        .collect();
    assert_eq!(roots, vec!["epic-one", "epic-two"]);
    let epic_one = v
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["slug"] == "epic-one")
        .unwrap();
    assert_eq!(epic_one["children"][0]["slug"], "nested-epic");
}
