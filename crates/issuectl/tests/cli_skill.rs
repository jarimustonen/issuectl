//! Black-box coverage for `issuectl skill list` output and dispatch.

use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .args(args)
        .output()
        .expect("spawn issuectl")
}

#[test]
fn skill_list_reports_the_bundled_catalog_in_text_and_json() {
    let text = run(&["skill", "list"]);
    assert_eq!(text.status.code(), Some(0), "{text:?}");
    assert!(text.stderr.is_empty());
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("issue  Manage issues and epics in issues/."));
    assert!(text.contains("[claude] Claude Code skill  .claude/skills/issue/SKILL.md"));
    assert!(text.contains("[codex ] Codex prompt  .codex/prompts/issue.md"));
    assert!(text.contains("issue-new"));
    assert!(text.contains("issue-intake"));

    let json = run(&["--json", "skill", "list"]);
    assert_eq!(json.status.code(), Some(0), "{json:?}");
    assert!(json.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let skills = value["data"].as_array().expect("catalog is a JSON array");
    assert_eq!(
        skills
            .iter()
            .map(|skill| skill["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["issue", "issue-new", "issue-intake"]
    );
    assert_eq!(skills[0]["install_targets"][0]["agent"], "claude");
    assert_eq!(skills[0]["install_targets"][1]["agent"], "codex");
    assert_eq!(
        skills[0]["install_targets"][1]["path"],
        ".codex/prompts/issue.md"
    );
}
