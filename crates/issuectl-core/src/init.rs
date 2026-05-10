//! `issuectl init` — one-command bootstrap for a fresh repo.
//!
//! Runs the existing first-time-setup chain in sequence with sensible
//! defaults: schema scaffold, `.issuectl/AGENTS.md`, the `/issue` skill
//! for one or more agents, and (opt-in) the pre-commit hook and YAML
//! merge driver. Each step is idempotent — re-running on an already
//! initialized repo reports each step as "already exists" and exits 0.
//!
//! The orchestration here is intentionally thin: per-step work lives
//! in the modules that own each artifact (`schema`, `agents`, `skill`,
//! `hooks`, `merge_driver`). This file only sequences them and shapes
//! the report.

use std::path::Path;

use anyhow::Result;
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
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "all" => Ok(Self::All),
            other => anyhow::bail!("unknown agent {other:?}; expected claude, codex, or all"),
        }
    }

    fn agents(self) -> Vec<skill::Agent> {
        match self {
            Self::Claude => vec![skill::Agent::Claude],
            Self::Codex => vec![skill::Agent::Codex],
            Self::All => vec![skill::Agent::Claude, skill::Agent::Codex],
        }
    }
}

/// Per-step status reported to humans and serialized in `--json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// Artifact was newly created (or rewritten with `--force`).
    Created,
    /// Artifact already existed and was left untouched.
    AlreadyExists,
    /// Step was not requested (opt-in flag absent).
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    /// Stable machine name: `schema`, `agents`, `skill`, `hooks`,
    /// `merge_driver`. Plural-aware: `skill` is one report covering
    /// both Claude and Codex; per-file paths live in `paths`.
    pub step: &'static str,
    pub status: StepStatus,
    /// Repo-relative paths affected by this step (or empty for steps
    /// that don't write a single file, e.g. `merge_driver`).
    pub paths: Vec<String>,
    /// Optional human-readable detail (schema source, hook bypass
    /// hint, etc.).
    pub note: Option<String>,
}

pub fn run(root: &Path, opts: InitOptions, json: bool) -> Result<()> {
    let mut reports: Vec<StepReport> = Vec::new();

    // 1. Schema bootstrap (issues/.schema.yaml).
    let schema_path = schema::schema_path(root);
    let wrote = schema::ensure_default_written(root)?;
    reports.push(StepReport {
        step: "schema",
        status: if wrote {
            StepStatus::Created
        } else {
            StepStatus::AlreadyExists
        },
        paths: vec![rel(root, &schema_path)],
        note: None,
    });

    // 2. .issuectl/AGENTS.md.
    let agents_outcome = agents::ensure_default_written(root, opts.force)?;
    reports.push(StepReport {
        step: "agents",
        status: if agents_outcome.wrote {
            StepStatus::Created
        } else {
            StepStatus::AlreadyExists
        },
        paths: vec![rel(root, &agents_outcome.path)],
        note: Some(format!(
            "schema_source={}",
            agents_outcome.schema_source.as_str()
        )),
    });

    // 3. Skill (one report covering all selected agents + scaffold).
    let skill_targets = opts.agent.agents();
    let skill_results = skill::install_skill_summary(root, &skill_targets, opts.force)?;
    let skill_paths: Vec<String> = skill_results.iter().map(|r| rel(root, &r.path)).collect();
    let any_created = skill_results
        .iter()
        .any(|r| matches!(r.outcome, skill::InstallOutcome::Created));
    reports.push(StepReport {
        step: "skill",
        status: if any_created {
            StepStatus::Created
        } else {
            StepStatus::AlreadyExists
        },
        paths: skill_paths,
        note: None,
    });

    // 4. Hooks (opt-in).
    if opts.with_hooks {
        let outcome = hooks::install_quiet(root, opts.force)?;
        reports.push(StepReport {
            step: "hooks",
            status: match outcome {
                hooks::InstallOutcome::Installed => StepStatus::Created,
                hooks::InstallOutcome::AlreadyInstalled => StepStatus::AlreadyExists,
            },
            paths: vec![rel(root, &root.join(".githooks/pre-commit"))],
            note: Some("core.hooksPath = .githooks".to_string()),
        });
    } else {
        reports.push(StepReport {
            step: "hooks",
            status: StepStatus::Skipped,
            paths: vec![],
            note: Some("opt in with --with-hooks".to_string()),
        });
    }

    // 5. Merge driver (opt-in).
    if opts.with_merge_driver {
        let outcome = merge_driver::install_quiet(root)?;
        reports.push(StepReport {
            step: "merge_driver",
            status: match outcome {
                merge_driver::InstallOutcome::Configured => StepStatus::Created,
                merge_driver::InstallOutcome::AlreadyConfigured => StepStatus::AlreadyExists,
            },
            paths: vec![],
            note: Some(
                "git config merge.issuectl-yaml.driver set; \
                 add `issues/**/item.md merge=issuectl-yaml` to .gitattributes"
                    .to_string(),
            ),
        });
    } else {
        reports.push(StepReport {
            step: "merge_driver",
            status: StepStatus::Skipped,
            paths: vec![],
            note: Some("opt in with --with-merge-driver".to_string()),
        });
    }

    if json {
        let payload = serde_json::json!({
            "steps": reports,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_human(&reports);
    }
    Ok(())
}

fn print_human(reports: &[StepReport]) {
    println!("Initializing issuectl in this repo:");
    for r in reports {
        let glyph = match r.status {
            StepStatus::Created => "✓",
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
            StepStatus::AlreadyExists => "already exists",
            StepStatus::Skipped => "skipped",
        };
        println!("  {glyph} {label} — {verb}");
        for p in &r.paths {
            println!("      {p}");
        }
        if let Some(note) = &r.note {
            // Only show notes that add information for non-skipped
            // steps; skipped-step notes are surfaced as the verb already.
            if !matches!(r.status, StepStatus::Skipped) {
                println!("      ({note})");
            }
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
        // Opt-in steps must be off by default.
        assert!(!tmp.path().join(".githooks/pre-commit").exists());
    }

    #[test]
    fn init_is_idempotent() {
        let tmp = fresh_git_repo();
        run(tmp.path(), opts(), false).unwrap();
        // Second run must not error and must not change a thing.
        let before =
            std::fs::read_to_string(tmp.path().join(".issuectl/AGENTS.md")).unwrap();
        run(tmp.path(), opts(), false).unwrap();
        let after = std::fs::read_to_string(tmp.path().join(".issuectl/AGENTS.md")).unwrap();
        assert_eq!(before, after);
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
    fn with_merge_driver_sets_git_config() {
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
}
