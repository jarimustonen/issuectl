//! Black-box parity coverage for ADR 0004's canonical `update` forms.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .current_dir(root)
        .arg("--root")
        .arg(root)
        .arg("--json")
        .args(args)
        .output()
        .expect("spawn issuectl")
}

fn pair() -> (TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    std::fs::create_dir_all(a.join("issues")).unwrap();
    std::fs::create_dir_all(b.join("issues")).unwrap();
    (tmp, a, b)
}

fn create(root: &Path, slug: &str) {
    let out = run(
        root,
        &["create", "--type", "task", "--title", slug, "--slug", slug],
    );
    assert!(out.status.success(), "{out:?}");
}

fn normalized_json(out: &Output, root: &Path) -> Value {
    assert!(out.status.success(), "{out:?}");
    let mut value: Value = serde_json::from_slice(&out.stdout).expect("JSON stdout");
    normalize_strings(&mut value, &root.to_string_lossy());
    value
}

fn normalize_strings(value: &mut Value, root: &str) {
    match value {
        Value::String(s) => *s = s.replace(root, "<ROOT>"),
        Value::Array(values) => {
            for value in values {
                normalize_strings(value, root);
            }
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                normalize_strings(value, root);
            }
        }
        _ => {}
    }
}

fn assert_same_success(a_root: &Path, a_args: &[&str], b_root: &Path, b_args: &[&str]) {
    let a = run(a_root, a_args);
    let b = run(b_root, b_args);
    assert_eq!(
        normalized_json(&a, a_root),
        normalized_json(&b, b_root),
        "a={a:?}\nb={b:?}"
    );
}

fn assert_same_failure(a_root: &Path, a_args: &[&str], b_root: &Path, b_args: &[&str]) {
    let a = run(a_root, a_args);
    let b = run(b_root, b_args);
    assert_eq!(a.status.code(), Some(1), "{a:?}");
    assert_eq!(b.status.code(), Some(1), "{b:?}");
    assert!(a.stdout.is_empty(), "{a:?}");
    assert!(b.stdout.is_empty(), "{b:?}");
    let mut a_json: Value = serde_json::from_slice(&a.stderr).expect("a JSON stderr");
    let mut b_json: Value = serde_json::from_slice(&b.stderr).expect("b JSON stderr");
    normalize_strings(&mut a_json, &a_root.to_string_lossy());
    normalize_strings(&mut b_json, &b_root.to_string_lossy());
    assert_eq!(a_json, b_json, "a={a:?}\nb={b:?}");
}

fn item(root: &Path, slug: &str) -> String {
    std::fs::read_to_string(root.join("issues").join(slug).join("item.md")).unwrap()
}

#[test]
fn patch_file_matches_apply_for_mutation_output_and_conflict() {
    let (tmp, apply_root, update_root) = pair();
    create(&apply_root, "patch-target");
    create(&update_root, "patch-target");
    let shown = run(&apply_root, &["show", "patch-target"]);
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    let version = shown["data"]["version"].as_str().unwrap();
    let patch = tmp.path().join("patch.yaml");
    std::fs::write(
        &patch,
        format!(
            "slug: patch-target\nexpected_version: {version}\npriority: high\nbody_ops:\n  - append_note:\n      author: ci-bot\n      message: all green\n"
        ),
    )
    .unwrap();
    let patch_s = patch.to_string_lossy();

    assert_same_success(
        &apply_root,
        &["apply", &patch_s],
        &update_root,
        &["update", "--patch-file", &patch_s],
    );
    assert_eq!(
        item(&apply_root, "patch-target"),
        item(&update_root, "patch-target")
    );

    // Reusing the now-stale patch must produce the same compare-and-swap error.
    assert_same_failure(
        &apply_root,
        &["apply", &patch_s],
        &update_root,
        &["update", "--patch-file", &patch_s],
    );
}

#[test]
fn query_matches_bulk_for_apply_and_dry_run() {
    let (_tmp, bulk_root, update_root) = pair();
    for slug in ["first-task", "second-task"] {
        create(&bulk_root, slug);
        create(&update_root, slug);
    }
    assert_same_success(
        &bulk_root,
        &[
            "bulk",
            "status:open",
            "--set",
            "priority=high",
            "--add-label",
            "triaged",
        ],
        &update_root,
        &[
            "update",
            "--query",
            "status:open",
            "--priority",
            "high",
            "--add-label",
            "triaged",
        ],
    );
    for slug in ["first-task", "second-task"] {
        assert_eq!(item(&bulk_root, slug), item(&update_root, slug));
    }

    let (_tmp, bulk_root, update_root) = pair();
    create(&bulk_root, "dry-target");
    create(&update_root, "dry-target");
    let before = item(&bulk_root, "dry-target");
    assert_same_success(
        &bulk_root,
        &["bulk", "status:open", "--add-label", "planned", "--dry-run"],
        &update_root,
        &[
            "update",
            "--query",
            "status:open",
            "--add-label",
            "planned",
            "--dry-run",
        ],
    );
    assert_eq!(item(&bulk_root, "dry-target"), before);
    assert_eq!(item(&update_root, "dry-target"), before);
}

#[test]
fn existing_update_flags_match_the_folded_commands() {
    let cases: &[(&[&str], &[&str])] = &[
        (
            &["label", "subject-task", "add", "triaged"],
            &["update", "subject-task", "--add-label", "triaged"],
        ),
        (
            &["assign", "subject-task", "alice"],
            &["update", "subject-task", "--assignee", "alice"],
        ),
        (
            &[
                "depend",
                "add",
                "subject-task",
                "--blocked-by",
                "blocker-task",
            ],
            &["update", "subject-task", "--add-blocked-by", "blocker-task"],
        ),
        (
            &["set", "subject-task", "priority", "high"],
            &["update", "subject-task", "--priority", "high"],
        ),
        (
            &["close", "subject-task", "--status", "done"],
            &["update", "subject-task", "--status", "done"],
        ),
    ];

    for (folded, canonical) in cases {
        let (_tmp, folded_root, update_root) = pair();
        for slug in ["subject-task", "blocker-task"] {
            create(&folded_root, slug);
            create(&update_root, slug);
        }
        assert_same_success(&folded_root, folded, &update_root, canonical);
        assert_eq!(
            item(&folded_root, "subject-task"),
            item(&update_root, "subject-task"),
            "folded={folded:?} canonical={canonical:?}"
        );
    }
}

#[test]
fn folded_forms_and_query_surface_the_same_invalid_input_errors() {
    let stale = "sha256:v1:0000000000000000000000000000000000000000000000000000000000000000";
    let cases: &[(Vec<&str>, Vec<&str>)] = &[
        (
            vec![
                "label",
                "subject-task",
                "add",
                "triaged",
                "--expected-version",
                stale,
            ],
            vec![
                "update",
                "subject-task",
                "--add-label",
                "triaged",
                "--expected-version",
                stale,
            ],
        ),
        (
            vec![
                "assign",
                "subject-task",
                "alice",
                "--expected-version",
                stale,
            ],
            vec![
                "update",
                "subject-task",
                "--assignee",
                "alice",
                "--expected-version",
                stale,
            ],
        ),
        (
            vec![
                "depend",
                "add",
                "subject-task",
                "--blocked-by",
                "blocker-task",
                "--expected-version",
                stale,
            ],
            vec![
                "update",
                "subject-task",
                "--add-blocked-by",
                "blocker-task",
                "--expected-version",
                stale,
            ],
        ),
        (
            vec![
                "set",
                "subject-task",
                "priority",
                "high",
                "--expected-version",
                stale,
            ],
            vec![
                "update",
                "subject-task",
                "--priority",
                "high",
                "--expected-version",
                stale,
            ],
        ),
        (
            vec![
                "close",
                "subject-task",
                "--status",
                "done",
                "--expected-version",
                stale,
            ],
            vec![
                "update",
                "subject-task",
                "--status",
                "done",
                "--expected-version",
                stale,
            ],
        ),
    ];
    for (folded, canonical) in cases {
        let (_tmp, folded_root, update_root) = pair();
        for slug in ["subject-task", "blocker-task"] {
            create(&folded_root, slug);
            create(&update_root, slug);
        }
        assert_same_failure(&folded_root, folded, &update_root, canonical);
    }

    let (_tmp, bulk_root, update_root) = pair();
    create(&bulk_root, "subject-task");
    create(&update_root, "subject-task");
    assert_same_failure(
        &bulk_root,
        &["bulk", "status:", "--add-label", "triaged"],
        &update_root,
        &["update", "--query", "status:", "--add-label", "triaged"],
    );
}

#[test]
fn canonical_targets_and_patch_fields_are_mutually_exclusive() {
    let (_tmp, root, _) = pair();
    create(&root, "subject-task");
    let both_targets = run(
        &root,
        &[
            "update",
            "subject-task",
            "--query",
            "status:open",
            "--priority",
            "high",
        ],
    );
    assert_eq!(both_targets.status.code(), Some(1), "{both_targets:?}");
    assert!(String::from_utf8_lossy(&both_targets.stderr).contains("cannot be used with '--query"));

    let patch = root.join("patch.yaml");
    std::fs::write(&patch, "slug: subject-task\npriority: high\n").unwrap();
    let patch_s = patch.to_string_lossy();
    let mixed = run(
        &root,
        &["update", "--patch-file", &patch_s, "--priority", "high"],
    );
    assert_eq!(mixed.status.code(), Some(1), "{mixed:?}");
    let stderr = String::from_utf8_lossy(&mixed.stderr);
    assert!(
        stderr.contains("--patch-file") && stderr.contains("--priority"),
        "{stderr}"
    );
}
