//! Black-box coverage for `issuectl config` argument dispatch and output.

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

#[test]
fn config_path_reports_the_schema_path_in_text_and_json() {
    let tmp = fresh_repo();
    let root = tmp.path();
    let expected = root.join("issues/.schema.yaml").to_string_lossy().into_owned();

    let text = run(root, &["config", "path"]);
    assert_eq!(text.status.code(), Some(0), "{text:?}");
    assert_eq!(String::from_utf8(text.stdout).unwrap(), format!("{expected}\n"));

    let json = run(root, &["--json", "config", "path"]);
    assert_eq!(json.status.code(), Some(0), "{json:?}");
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["path"], expected);
    assert!(json.stderr.is_empty());
}

#[test]
fn config_show_reports_effective_values_and_sources() {
    let tmp = fresh_repo();
    let root = tmp.path();
    std::fs::write(
        root.join("issues/.schema.yaml"),
        "version: 1\nfields:\n  priority:\n    required: false\n",
    )
    .unwrap();

    let json = run(root, &["--json", "config", "show"]);
    assert_eq!(json.status.code(), Some(0), "{json:?}");
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["values"]["schema.fields.priority"]["source"], "file");
    assert_eq!(value["values"]["schema.fields.status"]["source"], "default");

    let text = run(root, &["config", "show"]);
    assert_eq!(text.status.code(), Some(0), "{text:?}");
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("schema.fields.priority [file]"));
    assert!(text.contains("schema.fields.status [default]"));
}
