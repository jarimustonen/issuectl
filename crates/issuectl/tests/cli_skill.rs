//! Black-box coverage for the §15 companion-skill catalog and installer.

use std::path::Path;
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .args(args)
        .output()
        .expect("spawn issuectl")
}

fn assert_success(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

fn installed_paths(root: &Path) -> [&Path; 9] {
    [
        Path::new(".claude/skills/issue/SKILL.md"),
        Path::new(".claude/skills/issue-new/SKILL.md"),
        Path::new(".claude/skills/issue-intake/SKILL.md"),
        Path::new(".pi/agent/skills/issue/SKILL.md"),
        Path::new(".pi/agent/skills/issue-new/SKILL.md"),
        Path::new(".pi/agent/skills/issue-intake/SKILL.md"),
        Path::new(".codex/prompts/issue.md"),
        Path::new(".codex/prompts/issue-new.md"),
        Path::new(".codex/prompts/issue-intake.md"),
    ]
    .map(|path| {
        assert!(root.join(path).is_file(), "{} should exist", path.display());
        path
    })
}

#[test]
fn skill_list_reports_complete_machine_contract() {
    let text = run(&["skill", "list"]);
    assert_success(&text);
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("Supported agents: claude, pi, codex"));
    assert!(text.contains("issue  Manage issues and epics in issues/."));
    assert!(text.contains(".pi/agent/skills/<name>/..."));

    let json = run(&["--json", "skill", "list"]);
    assert_success(&json);
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let data = &value["data"];
    assert_eq!(
        data["supported_agents"],
        serde_json::json!(["claude", "pi", "codex"])
    );
    assert_eq!(data["install"]["selection_flag"], "--agent");
    assert_eq!(data["install"]["default"], "all");
    assert_eq!(
        data["install"]["accepted_values"],
        serde_json::json!(["claude", "pi", "codex", "all"])
    );
    assert_eq!(data["install"]["target_flag"], "--target");
    assert_eq!(data["install"]["dry_run_flag"], "--dry-run");
    assert_eq!(data["install"]["force_flag"], "--force");
    assert_eq!(data["install"]["interactive"], false);
    assert_eq!(data["install"]["no_clobber_default"], true);
    assert_eq!(data["install"]["overwrite_requires_force"], true);
    assert_eq!(
        data["install"]["layouts"],
        serde_json::json!([
            {"agent":"claude","path":".claude/skills/<name>/...","form":"agent-skill-tree"},
            {"agent":"pi","path":".pi/agent/skills/<name>/...","form":"agent-skill-tree"},
            {"agent":"codex","path":".codex/prompts/<name>.md","form":"self-contained-prompt"}
        ])
    );
    assert_eq!(
        data["skills"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["issue", "issue-new", "issue-intake"]
    );
}

#[test]
fn default_and_explicit_all_install_every_skill_for_every_agent() {
    for explicit_all in [false, true] {
        let target = tempfile::tempdir().unwrap();
        let target_arg = target.path().to_str().unwrap();
        let mut args = vec!["--json", "skill", "install", "--target", target_arg];
        if explicit_all {
            args.extend(["--agent", "all"]);
        }
        let output = run(&args);
        assert_success(&output);
        installed_paths(target.path());
        assert!(target.path().join("issues/AGENTS.md").is_file());

        for name in ["issue", "issue-new", "issue-intake"] {
            let claude = std::fs::read(
                target
                    .path()
                    .join(format!(".claude/skills/{name}/SKILL.md")),
            )
            .unwrap();
            let pi = std::fs::read(
                target
                    .path()
                    .join(format!(".pi/agent/skills/{name}/SKILL.md")),
            )
            .unwrap();
            assert_eq!(
                claude, pi,
                "pi and Claude Agent Skills must be byte-identical"
            );
            let codex =
                std::fs::read_to_string(target.path().join(format!(".codex/prompts/{name}.md")))
                    .unwrap();
            assert!(
                !codex.starts_with("---\n"),
                "Codex prompt must be self-contained without skill frontmatter"
            );
        }
    }
}

#[test]
fn each_single_agent_selection_writes_only_its_native_layout() {
    for (agent, expected) in [
        ("claude", ".claude/skills/issue/SKILL.md"),
        ("pi", ".pi/agent/skills/issue/SKILL.md"),
        ("codex", ".codex/prompts/issue.md"),
    ] {
        let target = tempfile::tempdir().unwrap();
        let output = run(&[
            "--json",
            "skill",
            "install",
            "issue",
            "--agent",
            agent,
            "--target",
            target.path().to_str().unwrap(),
        ]);
        assert_success(&output);
        assert!(target.path().join(expected).is_file());
        let skill_files = walkdir_count(target.path(), "SKILL.md");
        let prompt_files = walkdir_count(target.path(), "issue.md");
        assert_eq!(
            skill_files + prompt_files,
            1,
            "only one runtime artifact should be installed"
        );
    }
}

fn walkdir_count(root: &Path, filename: &str) -> usize {
    fn visit(path: &Path, filename: &str, count: &mut usize) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, filename, count);
            } else if path.file_name().and_then(|name| name.to_str()) == Some(filename) {
                *count += 1;
            }
        }
    }
    let mut count = 0;
    visit(root, filename, &mut count);
    count
}

#[test]
fn dry_run_reports_plan_without_creating_target() {
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join("not-created");
    let output = run(&[
        "--json",
        "skill",
        "install",
        "--target",
        target.to_str().unwrap(),
        "--dry-run",
    ]);
    assert_success(&output);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data"]["dry_run"], true);
    assert_eq!(
        value["data"]["agents"],
        serde_json::json!(["claude", "pi", "codex"])
    );
    assert!(
        !target.exists(),
        "dry-run must not create its target directory"
    );
}

#[test]
fn collision_is_preserved_without_force_and_overwritten_with_force() {
    let target = tempfile::tempdir().unwrap();
    let path = target.path().join(".pi/agent/skills/issue/SKILL.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "repo-authored\n").unwrap();
    let base = [
        "--json",
        "skill",
        "install",
        "issue",
        "--agent",
        "pi",
        "--target",
        target.path().to_str().unwrap(),
    ];

    let preserved = run(&base);
    assert_success(&preserved);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "repo-authored\n");
    let value: serde_json::Value = serde_json::from_slice(&preserved.stdout).unwrap();
    assert!(value["data"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| {
            row["path"]
                .as_str()
                .unwrap()
                .ends_with(".pi/agent/skills/issue/SKILL.md")
                && row["outcome"] == "already_exists"
        }));

    let mut forced_args = base.to_vec();
    forced_args.push("--force");
    let forced = run(&forced_args);
    assert_success(&forced);
    assert_ne!(std::fs::read_to_string(&path).unwrap(), "repo-authored\n");
}
