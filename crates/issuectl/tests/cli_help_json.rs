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
    assert!(document["examples"].as_array().unwrap().len() >= 1);
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
