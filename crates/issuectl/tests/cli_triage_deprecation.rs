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
fn triage_listing_points_to_read_only_doctor() {
    let tmp = fresh_repo();
    seed_inbox(tmp.path(), "listed-draft");
    let value = json(&run(tmp.path(), &["--json", "triage"]));
    assert_eq!(
        value["warnings"][0]["replacement_argv"],
        serde_json::json!(["issuectl", "doctor"])
    );
}

#[test]
fn text_deprecation_warning_stays_on_stderr() {
    let tmp = fresh_repo();
    seed_inbox(tmp.path(), "text-draft");
    let out = run(tmp.path(), &["triage"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("warning: "));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("warning:"));
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
        assert_eq!(first["data"]["hits"][0]["status"], "untracked");
        assert_eq!(first["data"]["filings"].as_array().unwrap().len(), 1);
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
        assert!(body.contains("source_ref: scan-todos:source.rs:"), "{body}");
        assert!(!tmp.path().join("issues/inbox").exists());

        // Moving the unchanged marker to another line must keep the same
        // content-derived source identity and deduplicate the retry.
        std::fs::write(
            tmp.path().join("source.rs"),
            "// inserted above\n// TODO(issue:) follow up\n",
        )
        .unwrap();
        let second = json(&run(tmp.path(), &["--json", "scan-todos", flag]));
        assert_eq!(
            second["data"]["filings"][0]["deduplicated"], true,
            "line movement must not create a second intake item"
        );
        let items_after = std::fs::read_dir(tmp.path().join("issues"))
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().join("item.md").is_file())
            .count();
        assert_eq!(items_after, 1, "stable source-ref must deduplicate retries");
    }
}

#[test]
fn scan_todos_json_reports_nonfatal_filing_failures_structurally() {
    let tmp = fresh_repo();
    std::fs::write(
        tmp.path().join("issues/.schema.yaml"),
        "version: 1\nfields:\n  provenance:\n    enum: [chat]\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("source.rs"), "// TODO(issue:) follow up\n").unwrap();

    let value = json(&run(tmp.path(), &["--json", "scan-todos", "--file-intake"]));
    assert!(value["data"]["filings"][0]["error"].is_string());
    assert!(value["warnings"][0]
        .as_str()
        .unwrap()
        .contains("could not file intake item"));
    assert_eq!(
        std::fs::read_dir(tmp.path().join("issues"))
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().join("item.md").is_file())
            .count(),
        0
    );
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
    let item = tmp.path().join("issues/inbox/stranded-draft/item.md");
    let mut body = std::fs::read_to_string(&item).unwrap();
    body.push_str("\n## Notes\n\nLegacy draft note.\n");
    std::fs::write(&item, body).unwrap();

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
    let migrated = tmp.path().join("issues/stranded-draft/item.md");
    assert!(migrated.is_file());
    let migrated_body = std::fs::read_to_string(migrated).unwrap();
    assert!(migrated_body.contains("## Comments"), "{migrated_body}");
    assert!(!migrated_body.contains("## Notes"), "{migrated_body}");
    assert!(!tmp.path().join("issues/inbox").exists());
}
