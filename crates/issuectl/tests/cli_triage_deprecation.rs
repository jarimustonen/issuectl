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
        .env_remove("ISSUECTL_NO_DEPRECATION_WARNINGS")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .current_dir(root)
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .expect("spawn issuectl")
}

fn seed_inbox(root: &Path, slug: &str) {
    let dir = root.join("issues/inbox").join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("item.md"),
        format!(
            "---\ncreated: 2026-01-01\nupdated: 2026-01-01\nslug: {slug}\ntype: task\nstatus: open\npriority: normal\n---\n\n# Draft\n"
        ),
    )
    .unwrap();
}

fn json(out: &Output) -> serde_json::Value {
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    serde_json::from_slice(&out.stdout).expect("JSON stdout")
}

#[test]
fn deprecated_triage_still_promotes_and_emits_structured_warning() {
    let tmp = fresh_repo();
    seed_inbox(tmp.path(), "old-draft");

    let value = json(&run(tmp.path(), &["--json", "triage", "old-draft"]));
    assert_eq!(value["data"]["slug"], "old-draft");
    assert_eq!(value["warnings"][0]["id"], "triage-command");
    assert_eq!(value["warnings"][0]["removal_version"], "0.18.0");
    assert_eq!(
        value["warnings"][0]["replacement_argv"],
        serde_json::json!(["issuectl", "doctor", "--fix"])
    );
    assert!(tmp.path().join("issues/old-draft/item.md").is_file());
}

#[test]
fn deprecation_warning_can_be_suppressed() {
    let tmp = fresh_repo();
    seed_inbox(tmp.path(), "quiet-draft");
    let out = Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env("ISSUECTL_NO_DEPRECATION_WARNINGS", "1")
        .current_dir(tmp.path())
        .args(["--root", tmp.path().to_str().unwrap(), "--json", "triage"])
        .output()
        .unwrap();
    let value = json(&out);
    assert_eq!(value["warnings"], serde_json::json!([]));
}

#[test]
fn canonical_help_hides_legacy_inbox_surface() {
    let tmp = fresh_repo();
    let top = run(tmp.path(), &["--help"]);
    let text = String::from_utf8_lossy(&top.stdout);
    assert!(!text.contains("\n  triage "), "{text}");

    let create = run(tmp.path(), &["create", "--help"]);
    let text = String::from_utf8_lossy(&create.stdout);
    assert!(!text.contains("--inbox"), "{text}");

    let scan = run(tmp.path(), &["scan-todos", "--help"]);
    let text = String::from_utf8_lossy(&scan.stdout);
    assert!(text.contains("--file-intake"), "{text}");
    assert!(!text.contains("--create-inbox"), "{text}");
}

#[test]
fn scan_todos_files_via_intake_and_old_flag_is_a_deprecated_alias() {
    for flag in ["--file-intake", "--create-inbox"] {
        let tmp = fresh_repo();
        std::fs::write(tmp.path().join("source.rs"), "// TODO(issue:) follow up\n").unwrap();

        let first = json(&run(tmp.path(), &["--json", "scan-todos", flag]));
        assert_eq!(first["data"][0]["status"], "untracked");
        if flag == "--create-inbox" {
            assert_eq!(first["warnings"][0]["id"], "scan-todos-create-inbox");
        } else {
            assert_eq!(first["warnings"], serde_json::json!([]));
        }

        let items: Vec<_> = std::fs::read_dir(tmp.path().join("issues"))
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().join("item.md").is_file())
            .collect();
        assert_eq!(items.len(), 1);
        let body = std::fs::read_to_string(items[0].path().join("item.md")).unwrap();
        assert!(body.contains("status: untriaged"), "{body}");
        assert!(body.contains("provenance: scan-todos"), "{body}");
        assert!(body.contains("source_ref: source.rs:1"), "{body}");
        assert!(!tmp.path().join("issues/inbox").exists());

        let second = run(tmp.path(), &["scan-todos", flag]);
        assert_eq!(second.status.code(), Some(0), "{second:?}");
        let items_after = std::fs::read_dir(tmp.path().join("issues"))
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().join("item.md").is_file())
            .count();
        assert_eq!(items_after, 1, "stable source-ref must deduplicate retries");
    }
}

#[test]
fn create_inbox_remains_compatible_with_a_warning() {
    let tmp = fresh_repo();
    let value = json(&run(
        tmp.path(),
        &[
            "--json",
            "create",
            "--type",
            "task",
            "--title",
            "Compatibility draft",
            "--slug",
            "compatibility-draft",
            "--inbox",
        ],
    ));
    assert_eq!(value["warnings"][0]["id"], "create-inbox");
    assert!(tmp
        .path()
        .join("issues/inbox/compatibility-draft/item.md")
        .is_file());
}

#[test]
fn doctor_reports_and_migrates_stranded_inbox_drafts() {
    let tmp = fresh_repo();
    seed_inbox(tmp.path(), "stranded-draft");

    let report = json(&run(tmp.path(), &["--json", "doctor"]));
    assert_eq!(report["data"]["inbox_drafts"][0]["slug"], "stranded-draft");
    assert!(tmp
        .path()
        .join("issues/inbox/stranded-draft/item.md")
        .is_file());

    let fixed = json(&run(tmp.path(), &["--json", "doctor", "--fix"]));
    assert_eq!(
        fixed["data"]["apply_outcome"]["inbox_drafts_migrated"][0]["slug"],
        "stranded-draft"
    );
    assert!(tmp.path().join("issues/stranded-draft/item.md").is_file());
    assert!(!tmp.path().join("issues/inbox").exists());
}
