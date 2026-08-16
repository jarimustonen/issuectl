//! Black-box coverage for the CLI JSON contract and drift-audit endpoint.

use std::process::Command;

#[test]
fn version_json_reports_schema_and_bundled_skills_inside_envelope() {
    let out = Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .args(["version", "--json"])
        .output()
        .expect("spawn issuectl");
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["warnings"], serde_json::json!([]));
    let data = &value["data"];
    assert!(data["version"].as_str().is_some());
    assert_eq!(data["supported_schemas"], serde_json::json!([1]));
    assert_eq!(
        data["skills"]
            .as_array()
            .unwrap()
            .iter()
            .map(|skill| skill["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["issue", "issue-new", "issue-intake"]
    );
}
