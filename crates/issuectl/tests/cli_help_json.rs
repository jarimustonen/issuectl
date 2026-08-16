//! Black-box coverage for machine-readable clap help.

use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .args(args)
        .output()
        .expect("spawn issuectl")
}

#[test]
fn root_help_json_is_a_single_structured_document() {
    let output = run(&["--help", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");

    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["path"], serde_json::json!(["issuectl"]));
    assert!(document["subcommands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command["name"] == "new"));
    assert!(!document["examples"].as_array().unwrap().is_empty());
}

#[test]
fn subcommand_help_json_includes_values_and_global_json_flag() {
    let output = run(&["new", "--help", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");

    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["path"], serde_json::json!(["issuectl", "new"]));
    let flags = document["flags"].as_array().unwrap();
    let issue_type = flags.iter().find(|flag| flag["long"] == "--type").unwrap();
    assert!(issue_type["possible_values"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "bug"));
    assert!(flags
        .iter()
        .any(|flag| flag["long"] == "--json" && flag["global"] == true));
}

#[test]
fn nested_and_aliased_help_json_follow_claps_resolved_command() {
    let nested = run(&["body", "set", "--help", "--json"]);
    assert_eq!(nested.status.code(), Some(0), "{nested:?}");
    let nested: serde_json::Value = serde_json::from_slice(&nested.stdout).unwrap();
    assert_eq!(
        nested["path"],
        serde_json::json!(["issuectl", "body", "set"])
    );

    let alias = run(&["ls", "--help", "--json"]);
    assert_eq!(alias.status.code(), Some(0), "{alias:?}");
    let alias: serde_json::Value = serde_json::from_slice(&alias.stdout).unwrap();
    assert_eq!(alias["path"], serde_json::json!(["issuectl", "list"]));
}

#[test]
fn invalid_help_invocation_remains_a_json_usage_error() {
    let output = run(&["no-such-command", "--help", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "usage-error");
}
