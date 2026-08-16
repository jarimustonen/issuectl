//! Black-box coverage for type-to-epic migration and its CLI escape hatches.

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

fn write_issue(root: &Path, slug: &str, fields: &str) {
    let dir = root.join("issues").join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("item.md"),
        format!(
            "---\ntype: task\ncreated: 2026-05-06\nstatus: open\npriority: normal\n{fields}---\n\n# Title\n"
        ),
    )
    .unwrap();
}

#[test]
fn update_type_epic_migrates_reporter_and_lifts_warning_to_json_envelope() {
    let tmp = fresh_repo();
    let root = tmp.path();
    write_issue(root, "reporter-epic", "reporter: alice\n");

    let out = run(
        root,
        &["--json", "update", "reporter-epic", "--type", "epic"],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(value["data"].get("warnings").is_none(), "{value}");
    assert!(
        value["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().is_some_and(|w| w.contains("migrated reporter"))),
        "{value}"
    );
    let content = std::fs::read_to_string(root.join("issues/reporter-epic/item.md")).unwrap();
    assert!(content.contains("owner: alice"), "{content}");
    assert!(!content.contains("reporter:"), "{content}");

    write_issue(root, "human-warning", "reporter: alice\n");
    let human_warning = run(root, &["update", "human-warning", "--type", "epic"]);
    assert_eq!(human_warning.status.code(), Some(0), "{human_warning:?}");
    assert!(
        String::from_utf8_lossy(&human_warning.stderr)
            .contains("@human-warning: migrated reporter"),
        "{human_warning:?}"
    );

    let human = run(root, &["update", "reporter-epic", "--type", "task"]);
    assert_ne!(
        human.status.code(),
        Some(0),
        "owner must still be rejected for non-epics"
    );
    assert!(
        String::from_utf8_lossy(&human.stderr)
            .contains("issuectl update reporter-epic --no-owner --type task"),
        "{human:?}"
    );
    let remedy = run(
        root,
        &["update", "reporter-epic", "--no-owner", "--type", "task"],
    );
    assert_eq!(remedy.status.code(), Some(0), "{remedy:?}");
}

#[test]
fn update_no_reporter_and_no_assignee_clear_built_in_fields() {
    let tmp = fresh_repo();
    let root = tmp.path();
    write_issue(root, "clear-people", "reporter: alice\nassignee: bob\n");

    let out = run(
        root,
        &["update", "clear-people", "--no-reporter", "--no-assignee"],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let content = std::fs::read_to_string(root.join("issues/clear-people/item.md")).unwrap();
    assert!(!content.contains("reporter:"), "{content}");
    assert!(!content.contains("assignee:"), "{content}");
}
