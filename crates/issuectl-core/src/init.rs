//! `issuectl init` — one-command bootstrap for a fresh repo.
//!
//! Runs the existing first-time-setup chain in sequence with sensible
//! defaults: schema scaffold, `.issuectl/AGENTS.md`, the `/issue` skill
//! for one or more agents, and (opt-in) the pre-commit hook and YAML
//! merge driver. Each step is idempotent — re-running on an already
//! initialized repo reports each step as `already_exists` and exits 0.
//!
//! The orchestration here is intentionally thin: per-step work lives
//! in the modules that own each artifact (`schema`, `agents`, `skill`,
//! `hooks`, `merge_driver`). This file only sequences them and shapes
//! the report.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{agents, hooks, merge_driver, schema, skill};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitOptions {
    pub agent: AgentSelection,
    pub with_hooks: bool,
    pub with_merge_driver: bool,
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSelection {
    Claude,
    Codex,
    All,
}

impl AgentSelection {
    fn agents(self) -> &'static [skill::Agent] {
        match self {
            Self::Claude => &[skill::Agent::Claude],
            Self::Codex => &[skill::Agent::Codex],
            Self::All => &[skill::Agent::Claude, skill::Agent::Codex],
        }
    }
}

/// Per-step status reported to humans and serialized in `--json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// Artifact was newly created.
    Created,
    /// Artifact existed and was rewritten in place under `--force`.
    Overwritten,
    /// Artifact already existed and was left untouched.
    AlreadyExists,
    /// Step touched some artifacts and left others alone — see
    /// `artifacts[]` for per-file detail.
    Mixed,
    /// Step was not requested (opt-in flag absent).
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactReport {
    pub path: String,
    pub status: ArtifactStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    Created,
    Overwritten,
    /// Used for `.issuectl/AGENTS.md` when only the schema-derived
    /// managed block was rewritten and the user prose was preserved.
    ManagedRefreshed,
    AlreadyExists,
}

impl ArtifactStatus {
    fn human_verb(self) -> &'static str {
        match self {
            Self::Created => "Created",
            Self::Overwritten => "Overwrote",
            Self::ManagedRefreshed => "Refreshed managed block in",
            Self::AlreadyExists => "Already exists:",
        }
    }
}

/// Side effect that isn't a path write — currently `git config`
/// invocations. Surfaces in JSON so automation tools can verify
/// without scraping notes.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    GitConfig {
        key: String,
        value: String,
        scope: &'static str,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    /// Stable machine name: `schema`, `agents`, `skill`, `hooks`,
    /// `merge_driver`. Distinct from the human label rendered by
    /// `print_human` so the JSON contract isn't coupled to wording.
    pub step: &'static str,
    pub status: StepStatus,
    /// Per-file outcomes (empty when the step doesn't write files,
    /// e.g. the merge-driver step which only mutates git config).
    #[serde(default)]
    pub artifacts: Vec<ArtifactReport>,
    /// Non-path effects (git config writes, etc.).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Free-form follow-up actions for the user (e.g. "add this line
    /// to .gitattributes"). Always machine-parseable as a list of
    /// short strings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next_steps: Vec<String>,
    /// Optional human-readable hint. Not load-bearing for automation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Step-specific structured detail (schema source for the agents
    /// step, etc.). Empty for steps without metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

pub fn run(root: &Path, opts: InitOptions, json: bool) -> Result<()> {
    let mut reports: Vec<StepReport> = Vec::new();

    // Detect schema source BEFORE step 1, so the agents step can
    // truthfully report whether the project had its own schema or
    // whether init bootstrapped the default. Detecting after step 1
    // (which writes the schema) would always report `project`.
    let schema_source = agents::detect_schema_source(root);

    // 1. Schema bootstrap (issues/.schema.yaml). `--force` resets a
    // malformed scaffold; without `--force` it's a noop on existing.
    let schema_path = schema::schema_path(root);
    let existed_before = schema_path.exists();
    let wrote = schema::write_default(root, opts.force)
        .context("initializing issues/.schema.yaml")?;
    let schema_status = if wrote && existed_before {
        ArtifactStatus::Overwritten
    } else if wrote {
        ArtifactStatus::Created
    } else {
        ArtifactStatus::AlreadyExists
    };
    reports.push(StepReport {
        step: "schema",
        status: aggregate(&[schema_status]),
        artifacts: vec![ArtifactReport {
            path: rel(root, &schema_path),
            status: schema_status,
        }],
        effects: vec![],
        next_steps: vec![],
        message: None,
        details: None,
    });

    // Preflight schema *load* before any further mutation, so a
    // malformed `issues/.schema.yaml` fails fast with a clear error
    // instead of bombing in step 2 after we've already mutated the
    // filesystem.
    let _schema = schema::load(root).context(
        "loading issues/.schema.yaml — fix or remove the malformed schema and re-run",
    )?;

    // 2. .issuectl/AGENTS.md.
    let agents_outcome = agents::ensure_default_written(root, opts.force)
        .context("initializing .issuectl/AGENTS.md")?;
    let agents_status = match agents_outcome.outcome {
        agents::EnsureOutcome::Created => ArtifactStatus::Created,
        agents::EnsureOutcome::ManagedRefreshed => ArtifactStatus::ManagedRefreshed,
        agents::EnsureOutcome::AlreadyExists => ArtifactStatus::AlreadyExists,
    };
    reports.push(StepReport {
        step: "agents",
        status: aggregate(&[agents_status]),
        artifacts: vec![ArtifactReport {
            path: rel(root, &agents_outcome.path),
            status: agents_status,
        }],
        effects: vec![],
        next_steps: vec![],
        message: None,
        details: Some(serde_json::json!({
            "schema_source": schema_source.as_str(),
        })),
    });

    // 3. Skill (one report covering all selected agents + scaffold).
    let skill_targets = opts.agent.agents();
    let skill_results = skill::install_skill_summary(root, skill_targets, opts.force)
        .context("installing skill templates")?;
    let artifacts: Vec<ArtifactReport> = skill_results
        .iter()
        .map(|r| ArtifactReport {
            path: rel(root, &r.path),
            status: match r.outcome {
                skill::InstallOutcome::Created => ArtifactStatus::Created,
                skill::InstallOutcome::Overwritten => ArtifactStatus::Overwritten,
                skill::InstallOutcome::AlreadyExists => ArtifactStatus::AlreadyExists,
            },
        })
        .collect();
    let per_artifact: Vec<ArtifactStatus> = artifacts.iter().map(|a| a.status).collect();
    reports.push(StepReport {
        step: "skill",
        status: aggregate(&per_artifact),
        artifacts,
        effects: vec![],
        next_steps: vec![],
        message: None,
        details: None,
    });

    // 4. Hooks (opt-in).
    if opts.with_hooks {
        let outcome = hooks::install_hook(root, opts.force)
            .context("installing pre-commit hook")?;
        let hook_path = root.join(".githooks/pre-commit");
        let status = match outcome {
            hooks::InstallOutcome::Installed => ArtifactStatus::Created,
            hooks::InstallOutcome::AlreadyInstalled => ArtifactStatus::AlreadyExists,
        };
        reports.push(StepReport {
            step: "hooks",
            status: aggregate(&[status]),
            artifacts: vec![ArtifactReport {
                path: rel(root, &hook_path),
                status,
            }],
            effects: vec![Effect::GitConfig {
                key: "core.hooksPath".to_string(),
                value: ".githooks".to_string(),
                scope: "local",
            }],
            next_steps: vec![],
            message: Some(
                "Bypass with `git commit --no-verify` or `ISSUECTL_SKIP_DOCTOR=1`."
                    .to_string(),
            ),
            details: None,
        });
    } else {
        reports.push(StepReport {
            step: "hooks",
            status: StepStatus::Skipped,
            artifacts: vec![],
            effects: vec![],
            next_steps: vec![],
            message: Some("opt in with --with-hooks".to_string()),
            details: None,
        });
    }

    // 5. Merge driver (opt-in).
    if opts.with_merge_driver {
        let outcome = merge_driver::install_config(root, opts.force)
            .context("configuring merge.issuectl-yaml.driver")?;
        let status = match outcome {
            merge_driver::InstallOutcome::Configured => ArtifactStatus::Created,
            merge_driver::InstallOutcome::AlreadyConfigured => ArtifactStatus::AlreadyExists,
        };
        reports.push(StepReport {
            step: "merge_driver",
            status: aggregate(&[status]),
            artifacts: vec![],
            effects: vec![Effect::GitConfig {
                key: "merge.issuectl-yaml.driver".to_string(),
                value: "(see git config --get merge.issuectl-yaml.driver)".to_string(),
                scope: "local",
            }],
            next_steps: vec![
                "Add `issues/**/item.md merge=issuectl-yaml` to .gitattributes and commit it; \
                 the driver is configured but inactive without that line."
                    .to_string(),
            ],
            message: None,
            details: None,
        });
    } else {
        reports.push(StepReport {
            step: "merge_driver",
            status: StepStatus::Skipped,
            artifacts: vec![],
            effects: vec![],
            next_steps: vec![],
            message: Some("opt in with --with-merge-driver".to_string()),
            details: None,
        });
    }

    if json {
        let payload = serde_json::json!({ "steps": reports });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_human(&reports);
    }
    Ok(())
}

/// Roll up a list of per-artifact statuses into the step's aggregate.
/// Empty input returns `AlreadyExists` (caller never has zero
/// artifacts and zero effects in current code, but the conservative
/// default keeps the report well-formed).
fn aggregate(per_artifact: &[ArtifactStatus]) -> StepStatus {
    if per_artifact.is_empty() {
        return StepStatus::AlreadyExists;
    }
    let any_created = per_artifact.iter().any(|s| {
        matches!(
            s,
            ArtifactStatus::Created | ArtifactStatus::Overwritten | ArtifactStatus::ManagedRefreshed
        )
    });
    let any_existing = per_artifact
        .iter()
        .any(|s| matches!(s, ArtifactStatus::AlreadyExists));
    match (any_created, any_existing) {
        (true, true) => StepStatus::Mixed,
        (true, false) => {
            // Promote `Overwritten` to the step level only when every
            // artifact was overwritten; otherwise call the step
            // `Created` since something was newly written.
            if per_artifact
                .iter()
                .all(|s| matches!(s, ArtifactStatus::Overwritten))
            {
                StepStatus::Overwritten
            } else {
                StepStatus::Created
            }
        }
        (false, _) => StepStatus::AlreadyExists,
    }
}

fn print_human(reports: &[StepReport]) {
    println!("Initializing issuectl in this repo:");
    for r in reports {
        let glyph = match r.status {
            StepStatus::Created | StepStatus::Overwritten | StepStatus::Mixed => "✓",
            StepStatus::AlreadyExists => "~",
            StepStatus::Skipped => "·",
        };
        let label = match r.step {
            "schema" => "schema",
            "agents" => ".issuectl/AGENTS.md",
            "skill" => "skill",
            "hooks" => "pre-commit hook",
            "merge_driver" => "merge driver",
            other => other,
        };
        let verb = match r.status {
            StepStatus::Created => "created",
            StepStatus::Overwritten => "overwrote",
            StepStatus::Mixed => "updated (mixed)",
            StepStatus::AlreadyExists => "already exists",
            StepStatus::Skipped => "skipped",
        };
        println!("  {glyph} {label} — {verb}");
        for a in &r.artifacts {
            println!("      {} {}", a.status.human_verb(), a.path);
        }
        if let Some(msg) = &r.message {
            println!("      {msg}");
        }
        for ns in &r.next_steps {
            println!("      → next: {ns}");
        }
    }
    println!();
    println!("  Use /issue in your AI agent or `issuectl list` from the CLI.");
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_git_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let st = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(st.success());
        std::fs::create_dir_all(tmp.path().join("issues")).unwrap();
        tmp
    }

    fn opts() -> InitOptions {
        InitOptions {
            agent: AgentSelection::All,
            with_hooks: false,
            with_merge_driver: false,
            force: false,
        }
    }

    #[test]
    fn init_creates_baseline_artifacts() {
        let tmp = fresh_git_repo();
        run(tmp.path(), opts(), false).unwrap();
        assert!(tmp.path().join("issues/.schema.yaml").is_file());
        assert!(tmp.path().join(".issuectl/AGENTS.md").is_file());
        assert!(tmp.path().join("issues/AGENTS.md").is_file());
        assert!(tmp.path().join(".claude/skills/issue/SKILL.md").is_file());
        assert!(tmp.path().join(".codex/prompts/issue.md").is_file());
        assert!(!tmp.path().join(".githooks/pre-commit").exists());
    }

    #[test]
    fn init_is_idempotent_for_every_artifact() {
        let tmp = fresh_git_repo();
        let mut o = opts();
        o.with_hooks = true;
        o.with_merge_driver = true;
        run(tmp.path(), o, false).unwrap();

        let snapshot = |p: &Path| std::fs::read_to_string(p).unwrap();
        let before = [
            snapshot(&tmp.path().join("issues/.schema.yaml")),
            snapshot(&tmp.path().join(".issuectl/AGENTS.md")),
            snapshot(&tmp.path().join("issues/AGENTS.md")),
            snapshot(&tmp.path().join(".claude/skills/issue/SKILL.md")),
            snapshot(&tmp.path().join(".codex/prompts/issue.md")),
            snapshot(&tmp.path().join(".githooks/pre-commit")),
        ];

        run(tmp.path(), o, false).unwrap();

        let after = [
            snapshot(&tmp.path().join("issues/.schema.yaml")),
            snapshot(&tmp.path().join(".issuectl/AGENTS.md")),
            snapshot(&tmp.path().join("issues/AGENTS.md")),
            snapshot(&tmp.path().join(".claude/skills/issue/SKILL.md")),
            snapshot(&tmp.path().join(".codex/prompts/issue.md")),
            snapshot(&tmp.path().join(".githooks/pre-commit")),
        ];

        assert_eq!(before, after, "rerun must not rewrite any artifact");
    }

    #[test]
    fn force_preserves_user_prose_in_agents_md() {
        let tmp = fresh_git_repo();
        run(tmp.path(), opts(), false).unwrap();

        // Hand-edit the prose preamble (above the managed block).
        let agents_path = tmp.path().join(".issuectl/AGENTS.md");
        let original = std::fs::read_to_string(&agents_path).unwrap();
        let edited = original.replacen(
            "# Agents policy",
            "# Agents policy — TEAM-CUSTOM HEADER",
            1,
        );
        assert_ne!(edited, original);
        std::fs::write(&agents_path, &edited).unwrap();

        let mut o = opts();
        o.force = true;
        run(tmp.path(), o, false).unwrap();

        let after = std::fs::read_to_string(&agents_path).unwrap();
        assert!(
            after.contains("TEAM-CUSTOM HEADER"),
            "user prose must survive --force; got:\n{after}"
        );
    }

    #[test]
    fn with_hooks_installs_pre_commit() {
        let tmp = fresh_git_repo();
        let mut o = opts();
        o.with_hooks = true;
        run(tmp.path(), o, false).unwrap();
        assert!(tmp.path().join(".githooks/pre-commit").is_file());
    }

    #[test]
    fn with_merge_driver_sets_git_config_to_full_invocation() {
        let tmp = fresh_git_repo();
        let mut o = opts();
        o.with_merge_driver = true;
        run(tmp.path(), o, false).unwrap();
        let out = std::process::Command::new("git")
            .args(["config", "--local", "--get", "merge.issuectl-yaml.driver"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        assert!(out.status.success());
        let val = String::from_utf8_lossy(&out.stdout);
        assert!(val.contains("merge-driver"), "got: {val}");
        assert!(val.contains("--base %O"), "got: {val}");
        assert!(val.contains("--ours %A"), "got: {val}");
        assert!(val.contains("--theirs %B"), "got: {val}");
        assert!(val.contains("--output %A"), "got: {val}");
    }

    #[test]
    fn agent_selection_claude_only_skips_codex_file() {
        let tmp = fresh_git_repo();
        let mut o = opts();
        o.agent = AgentSelection::Claude;
        run(tmp.path(), o, false).unwrap();
        assert!(tmp.path().join(".claude/skills/issue/SKILL.md").is_file());
        assert!(!tmp.path().join(".codex/prompts/issue.md").exists());
    }

    #[test]
    fn malformed_schema_aborts_before_further_mutation() {
        let tmp = fresh_git_repo();
        std::fs::write(tmp.path().join("issues/.schema.yaml"), "not: [valid").unwrap();
        let err = run(tmp.path(), opts(), false).unwrap_err();
        let chain: Vec<String> = err.chain().map(|c| c.to_string()).collect();
        assert!(
            chain.iter().any(|c| c.contains("malformed schema")),
            "expected step-context error; got chain: {chain:?}"
        );
        // Subsequent steps must not have run.
        assert!(!tmp.path().join(".issuectl/AGENTS.md").exists());
    }

    #[test]
    fn merge_driver_refuses_existing_different_value_without_force() {
        let tmp = fresh_git_repo();
        let st = std::process::Command::new("git")
            .args([
                "config",
                "--local",
                "merge.issuectl-yaml.driver",
                "/path/to/wrapper $@",
            ])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(st.success());

        let mut o = opts();
        o.with_merge_driver = true;
        let err = run(tmp.path(), o, false).unwrap_err();
        assert!(
            err.to_string().contains("already set")
                || err.chain().any(|c| c.to_string().contains("already set")),
            "expected refusal; got: {err}"
        );

        // With --force it overwrites.
        o.force = true;
        run(tmp.path(), o, false).unwrap();
    }

    #[test]
    fn second_skill_install_partial_state_reports_mixed() {
        // First install Claude only, then re-run with --agent all to
        // ensure the second run's `skill` step is `mixed` (Claude
        // already exists; Codex newly created).
        let tmp = fresh_git_repo();
        let mut o = opts();
        o.agent = AgentSelection::Claude;
        run(tmp.path(), o, false).unwrap();

        // Round 2 — capture status by re-implementing the dispatch
        // since `run` prints rather than returns the report.
        // (Direct API check: install_skill_summary on the second run.)
        let results = skill::install_skill_summary(
            tmp.path(),
            &[skill::Agent::Claude, skill::Agent::Codex],
            false,
        )
        .unwrap();
        let any_existing = results
            .iter()
            .any(|r| matches!(r.outcome, skill::InstallOutcome::AlreadyExists));
        let any_created = results
            .iter()
            .any(|r| matches!(r.outcome, skill::InstallOutcome::Created));
        assert!(
            any_existing && any_created,
            "expected mixed outcomes; got: {results:?}"
        );
    }
}
