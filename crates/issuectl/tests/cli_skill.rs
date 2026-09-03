//! Black-box coverage for the §15 companion-skill catalog and installer.

use std::path::Path;
use std::process::{Command, Output};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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

const DOGFOOD_PATHS: [&str; 9] = [
    ".claude/skills/issue/SKILL.md",
    ".claude/skills/issue-new/SKILL.md",
    ".claude/skills/issue-intake/SKILL.md",
    ".pi/agent/skills/issue/SKILL.md",
    ".pi/agent/skills/issue-new/SKILL.md",
    ".pi/agent/skills/issue-intake/SKILL.md",
    ".codex/prompts/issue.md",
    ".codex/prompts/issue-new.md",
    ".codex/prompts/issue-intake.md",
];

fn installed_paths(root: &Path) -> [&Path; 9] {
    DOGFOOD_PATHS.map(Path::new).map(|path| {
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

#[cfg(unix)]
#[test]
fn release_bump_hook_regenerates_every_dogfood_copy_in_isolation() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let contract = std::fs::read_to_string(source_root.join("OSS-RELEASE.md")).unwrap();
    let frontmatter = contract
        .strip_prefix("---\n")
        .unwrap()
        .split_once("\n---\n")
        .unwrap()
        .0;
    let contract: serde_yaml::Value = serde_yaml::from_str(frontmatter).unwrap();
    assert_eq!(
        contract["release"]["bump_hook"], "scripts/release-bump-hook.sh",
        "the approved release contract must make regeneration part of the engine-owned bump"
    );

    let checkout = tempfile::tempdir().unwrap();
    std::fs::write(
        checkout.path().join("Cargo.toml"),
        format!(
            "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"{}\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
    let script_dir = checkout.path().join("scripts");
    std::fs::create_dir(&script_dir).unwrap();
    let hook = script_dir.join("release-bump-hook.sh");
    std::fs::copy(source_root.join("scripts/release-bump-hook.sh"), &hook).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Start from stale tracked artifacts, as a Shipshape bump does before the hook.
    for relative in DOGFOOD_PATHS {
        let path = checkout.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "stale pre-bump copy\n").unwrap();
    }
    let scaffold = checkout.path().join("issues/AGENTS.md");
    std::fs::create_dir_all(scaffold.parent().unwrap()).unwrap();
    std::fs::write(&scaffold, "repo-authored scaffold\n").unwrap();

    // Intercept Cargo rather than giving the production hook a binary bypass.
    // This exercises its real `cargo run` branch and delegates only the arguments
    // after `--` to the test-built issuectl.
    let fake_bin = checkout.path().join("fake-bin");
    std::fs::create_dir(&fake_bin).unwrap();
    let fake_cargo = fake_bin.join("cargo");
    std::fs::write(
        &fake_cargo,
        "#!/bin/sh\n{\n  printf '%s\\n%s\\n%s\\n' \"$PWD\" \"$HOME\" \"$CARGO_TARGET_DIR\"\n  printf '%s\\n' \"$@\"\n} >> \"$HOOK_ENV_LOG\"\nwhile [ \"$#\" -gt 0 ] && [ \"$1\" != -- ]; do shift; done\n[ \"$#\" -gt 0 ] || exit 64\nshift\nexec \"$WRAPPED_ISSUECTL\" \"$@\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake_cargo, std::fs::Permissions::from_mode(0o755)).unwrap();

    let operator_home = tempfile::tempdir().unwrap();
    let mut global_markers = DOGFOOD_PATHS.to_vec();
    global_markers.push(".pi/agent/skills/.issuectl-manifest.json");
    for relative in &global_markers {
        let path = operator_home.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "do not touch\n").unwrap();
    }
    let env_log = checkout.path().join("hook-environment");
    let path = std::env::join_paths(
        std::iter::once(fake_bin.clone())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let output = Command::new(&hook)
        .current_dir(operator_home.path())
        .env("PATH", path)
        .env("HOME", operator_home.path())
        .env("ISSUECTL_RELEASE_HOOK_BIN", "/must/not/be/honored")
        .env("WRAPPED_ISSUECTL", env!("CARGO_BIN_EXE_issuectl"))
        .env("HOOK_ENV_LOG", &env_log)
        .output()
        .expect("run release bump hook");
    assert_success(&output);

    let hook_environment = std::fs::read_to_string(env_log).unwrap();
    let lines: Vec<_> = hook_environment.lines().collect();
    assert_eq!(Path::new(lines[0]), checkout.path());
    let isolated_home = Path::new(lines[1]);
    let isolated_target = Path::new(lines[2]);
    assert_ne!(isolated_home, operator_home.path());
    assert_ne!(isolated_target, operator_home.path());
    assert_ne!(isolated_home, isolated_target);
    assert_eq!(isolated_home.parent(), isolated_target.parent());
    assert_eq!(
        &lines[3..],
        &[
            "run",
            "--locked",
            "--quiet",
            "-p",
            "issuectl",
            "--bin",
            "issuectl",
            "--",
            "skill",
            "install",
            "--agent",
            "all",
            "--target",
            checkout.path().to_str().unwrap(),
            "--force"
        ]
    );
    assert!(
        !isolated_home.exists() && !isolated_target.exists(),
        "the hook must remove its disposable HOME and build target"
    );

    for relative in global_markers {
        assert_eq!(
            std::fs::read_to_string(operator_home.path().join(relative)).unwrap(),
            "do not touch\n",
            "the release hook must not mutate operator-global agent installations"
        );
    }
    assert_eq!(
        std::fs::read_to_string(scaffold).unwrap(),
        "repo-authored scaffold\n",
        "the normal force install must preserve issues/AGENTS.md"
    );

    // Byte equality with the checked-in copies is the immediate post-bump
    // dogfood invariant. The test binary stands in for the freshly bumped binary.
    for relative in DOGFOOD_PATHS {
        let regenerated = std::fs::read(checkout.path().join(relative)).unwrap();
        let tracked = std::fs::read(source_root.join(relative)).unwrap();
        assert_eq!(regenerated, tracked, "{relative} was not regenerated");
        assert_ne!(regenerated, b"stale pre-bump copy\n");
    }
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
