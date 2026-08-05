use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const ISSUE_CLAUDE_TEMPLATE: &str = include_str!("../templates/issue-skill.md");
const ISSUE_CODEX_TEMPLATE: &str = include_str!("../templates/issue-prompt.md");
pub const ISSUES_AGENTS_TEMPLATE: &str = include_str!("../templates/issues-agents.md");

/// Substitute build-time tokens (currently `{{ISSUECTL_VERSION}}`) in a
/// template body. Used at install time so the on-disk skill is pinned to
/// the issuectl release that wrote it.
pub fn render_template(body: &str) -> String {
    body.replace("{{ISSUECTL_VERSION}}", env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            other => anyhow::bail!("unknown agent {other:?}; expected claude or codex"),
        }
    }

    pub fn template(self) -> &'static str {
        match self {
            Self::Claude => ISSUE_CLAUDE_TEMPLATE,
            Self::Codex => ISSUE_CODEX_TEMPLATE,
        }
    }

    pub fn install_path(self, repo_root: &Path) -> PathBuf {
        match self {
            Self::Claude => repo_root.join(".claude/skills/issue/SKILL.md"),
            Self::Codex => repo_root.join(".codex/prompts/issue.md"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code skill",
            Self::Codex => "Codex prompt",
        }
    }
}

/// Outcome of installing a single skill-related file. `init` and other
/// orchestrators consume this to report per-file status without
/// re-running file-existence checks of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOutcome {
    /// File did not exist; we wrote it.
    Created,
    /// File existed and `--force` was supplied; we overwrote it.
    Overwritten,
    /// File already existed and `--force` was not supplied.
    AlreadyExists,
}

#[derive(Debug, Clone)]
pub struct InstallResult {
    pub path: PathBuf,
    pub label: String,
    pub outcome: InstallOutcome,
}

/// Install the issues/AGENTS.md scaffold and one or more agent skill
/// files. Returns one [`InstallResult`] per file touched (or considered)
/// in the order: scaffold, then each agent in input order.
pub fn install_skill_summary(
    repo_root: &Path,
    agents: &[Agent],
    force: bool,
) -> Result<Vec<InstallResult>> {
    let mut results = Vec::with_capacity(agents.len() + 1);
    results.push(install_issues_scaffold(repo_root, force)?);
    for agent in agents {
        results.push(install_agent_template(repo_root, *agent, force)?);
    }
    Ok(results)
}

/// Install the issues/AGENTS.md scaffold and one or more agent skill files.
pub fn install_skill(repo_root: &Path, agents: &[Agent], force: bool) -> Result<()> {
    let results = install_skill_summary(repo_root, agents, force)?;
    for r in &results {
        print_install_result(repo_root, r);
    }

    println!();
    if agents.contains(&Agent::Claude) {
        println!("  Use /issue in Claude Code to create, search, update, and close issues.");
    }
    if agents.contains(&Agent::Codex) {
        println!("  Use /issue in Codex CLI (or invoke the prompt) to manage issues.");
    }
    println!("  Or use `issuectl list` to browse issues from the command line.");
    Ok(())
}

/// Print the template that would be installed for the given agent to stdout.
pub fn print_skill(agent: Agent) -> Result<()> {
    print!("{}", render_template(agent.template()));
    Ok(())
}

/// Render a single [`InstallResult`] in the `~ already exists` / `✓ Created`
/// style shared by `skill install` and `init`.
pub fn print_install_result(repo_root: &Path, r: &InstallResult) {
    let display = r
        .path
        .strip_prefix(repo_root)
        .unwrap_or(&r.path)
        .display()
        .to_string();
    let verb = match r.outcome {
        InstallOutcome::Created => "Created",
        InstallOutcome::Overwritten => "Overwrote",
        InstallOutcome::AlreadyExists => {
            println!("  ~ {display} already exists (use --force to overwrite)");
            return;
        }
    };
    if r.label.is_empty() {
        println!("  ✓ {verb} {display}");
    } else {
        println!("  ✓ {verb} {display} ({})", r.label);
    }
}

fn install_issues_scaffold(repo_root: &Path, force: bool) -> Result<InstallResult> {
    let issues_dir = repo_root.join("issues");
    let agents_md = issues_dir.join("AGENTS.md");

    if !issues_dir.exists() {
        std::fs::create_dir_all(&issues_dir)
            .with_context(|| format!("cannot create {}", issues_dir.display()))?;
    }

    let existed = agents_md.exists();
    let outcome = if force || !existed {
        std::fs::write(&agents_md, ISSUES_AGENTS_TEMPLATE)
            .with_context(|| format!("cannot write {}", agents_md.display()))?;
        if existed {
            InstallOutcome::Overwritten
        } else {
            InstallOutcome::Created
        }
    } else {
        InstallOutcome::AlreadyExists
    };
    Ok(InstallResult {
        path: agents_md,
        label: String::new(),
        outcome,
    })
}

fn install_agent_template(repo_root: &Path, agent: Agent, force: bool) -> Result<InstallResult> {
    let path = agent.install_path(repo_root);
    let existed = path.exists();

    if !force && existed {
        return Ok(InstallResult {
            path,
            label: agent.label().to_string(),
            outcome: InstallOutcome::AlreadyExists,
        });
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::write(&path, render_template(agent.template()))
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(InstallResult {
        path,
        label: agent.label().to_string(),
        outcome: if existed {
            InstallOutcome::Overwritten
        } else {
            InstallOutcome::Created
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_from_str_accepts_known_values() {
        assert_eq!(Agent::from_str("claude").unwrap(), Agent::Claude);
        assert_eq!(Agent::from_str("codex").unwrap(), Agent::Codex);
    }

    #[test]
    fn agent_from_str_rejects_unknown() {
        assert!(Agent::from_str("gpt").is_err());
        assert!(Agent::from_str("").is_err());
    }

    #[test]
    fn install_paths_differ_per_agent() {
        let root = Path::new("/tmp/repo");
        assert!(Agent::Claude
            .install_path(root)
            .ends_with(".claude/skills/issue/SKILL.md"));
        assert!(Agent::Codex
            .install_path(root)
            .ends_with(".codex/prompts/issue.md"));
    }

    #[test]
    fn render_template_substitutes_version() {
        let out = render_template("issuectl {{ISSUECTL_VERSION}} expected");
        assert_eq!(
            out,
            format!("issuectl {} expected", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn installed_skill_pins_current_version() {
        let tmp = tempfile::tempdir().unwrap();
        install_skill(tmp.path(), &[Agent::Claude], false).unwrap();
        let installed =
            std::fs::read_to_string(tmp.path().join(".claude/skills/issue/SKILL.md")).unwrap();
        assert!(installed.contains(env!("CARGO_PKG_VERSION")));
        assert!(!installed.contains("{{ISSUECTL_VERSION}}"));
    }

    /// The repo dogfoods both skill templates into `.claude/` and
    /// `.codex/`. They are the contract consumer-side agents read, so they
    /// must never drift from `templates/`. This test renders each template
    /// and compares it to the committed copy, tolerating only the pinned
    /// `{{ISSUECTL_VERSION}}` (the copy records the release that wrote it,
    /// which lags the in-development version). If it fails, regenerate with
    /// `issuectl skill install --agent all --force`.
    #[test]
    fn dogfooded_copies_match_templates() {
        fn pinned_version(copy: &str) -> &str {
            let marker = "This skill was installed for `issuectl ";
            let start = copy.find(marker).expect("version marker present") + marker.len();
            let rest = &copy[start..];
            let end = rest.find('`').expect("closing backtick after version");
            &rest[..end]
        }

        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for agent in [Agent::Claude, Agent::Codex] {
            let copy_path = agent.install_path(&repo_root);
            // Dev-only guard: a packaged/standalone crate won't have the
            // repo-root copies. Skip rather than fail outside the workspace.
            if !copy_path.exists() {
                continue;
            }
            let copy = std::fs::read_to_string(&copy_path).unwrap();
            let expected = agent
                .template()
                .replace("{{ISSUECTL_VERSION}}", pinned_version(&copy));
            assert_eq!(
                copy,
                expected,
                "{} has drifted from its template; run \
                 `issuectl skill install --agent all --force` to regenerate",
                copy_path.display()
            );
        }
    }

    /// The standalone intake skills (`/issue-new`, `/issue-intake`, and the
    /// `/triage-bugs` deprecation alias) are dogfooded as repo-local
    /// `.claude/skills/*/SKILL.md` files rather than installed by the binary
    /// (they orchestrate the `/worktree-*` family, so they are not pushed to
    /// arbitrary consumer repos). They still need a guard so they don't rot:
    /// this test pins their frontmatter and the load-bearing filing/processing
    /// split. Skipped outside the workspace (a packaged crate has no repo-root
    /// copies), matching `dogfooded_copies_match_templates`.
    #[test]
    fn standalone_intake_skills_are_wellformed() {
        let skills_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(".claude/skills");
        if !skills_dir.join("issue/SKILL.md").exists() {
            return; // packaged/standalone crate — no repo-root copies
        }

        let read = |name: &str| {
            let p = skills_dir.join(name).join("SKILL.md");
            std::fs::read_to_string(&p)
                .unwrap_or_else(|_| panic!("standalone skill missing at {}", p.display()))
        };

        // Every standalone skill carries valid frontmatter naming itself.
        for name in ["issue-new", "issue-intake", "triage-bugs"] {
            let body = read(name);
            let after = body
                .strip_prefix("---\n")
                .unwrap_or_else(|| panic!("{name} must open with YAML frontmatter"));
            let (front, _) = after
                .split_once("\n---")
                .unwrap_or_else(|| panic!("{name} frontmatter must close with ---"));
            assert!(
                front.contains(&format!("name: {name}")),
                "{name} frontmatter must set `name: {name}`"
            );
            assert!(
                front.contains("description:"),
                "{name} frontmatter must carry a description"
            );
        }

        // `/issue-new` is the FILING half: it files (and attaches), and does
        // not process the queue — that is `/issue-intake`'s job.
        let issue_new = read("issue-new");
        assert!(
            issue_new.contains("issuectl intake file"),
            "issue-new must file via `issuectl intake file`"
        );
        assert!(
            issue_new.contains("issuectl attach"),
            "issue-new must attach screenshots via `issuectl attach`"
        );
        assert!(
            !issue_new.contains("issuectl intake queue"),
            "issue-new must NOT process the queue — that belongs to /issue-intake"
        );

        // `/issue-intake` is the PROCESSING half: it reads the queue, drives the
        // read-only analysis engine (never reimplementing it), and documents
        // that it replaces `/triage-bugs`.
        let issue_intake = read("issue-intake");
        assert!(
            issue_intake.contains("issuectl intake queue"),
            "issue-intake must read the queue via `issuectl intake queue`"
        );
        assert!(
            issue_intake.contains("/worktree-bug-analysis"),
            "issue-intake must drive /worktree-bug-analysis as the analysis engine"
        );
        assert!(
            issue_intake.contains("## Triage analysis"),
            "issue-intake must reference the append-only ## Triage analysis section"
        );
        assert!(
            issue_intake.to_lowercase().contains("replaces")
                && issue_intake.contains("/triage-bugs"),
            "issue-intake must document that it replaces /triage-bugs"
        );

        // `/triage-bugs` is a THIN deprecation alias delegating to
        // `/issue-intake`; it must not reimplement any triage logic.
        let triage = read("triage-bugs");
        assert!(
            triage.contains("/issue-intake"),
            "triage-bugs alias must delegate to /issue-intake"
        );
        assert!(
            triage.to_uppercase().contains("DEPRECATED")
                || triage.to_lowercase().contains("renamed"),
            "triage-bugs alias must announce the rename/deprecation"
        );
        assert!(
            !triage.contains("issuectl intake queue"),
            "triage-bugs alias must NOT reimplement the queue read"
        );
        assert!(
            triage.len() < 3000,
            "triage-bugs must stay a thin alias (is {} bytes)",
            triage.len()
        );
    }

    #[test]
    fn templates_differ_between_agents() {
        let claude = Agent::Claude.template();
        let codex = Agent::Codex.template();
        assert_ne!(claude, codex);
        // Claude template carries the YAML frontmatter; Codex strips it
        assert!(claude.starts_with("---\nname: issue"));
        assert!(!codex.starts_with("---\nname:"));
    }

    #[test]
    fn install_writes_claude_only() {
        let tmp = tempfile::tempdir().unwrap();
        install_skill(tmp.path(), &[Agent::Claude], false).unwrap();
        assert!(tmp.path().join(".claude/skills/issue/SKILL.md").exists());
        assert!(!tmp.path().join(".codex/prompts/issue.md").exists());
        assert!(tmp.path().join("issues/AGENTS.md").exists());
    }

    #[test]
    fn install_writes_codex_only() {
        let tmp = tempfile::tempdir().unwrap();
        install_skill(tmp.path(), &[Agent::Codex], false).unwrap();
        assert!(!tmp.path().join(".claude/skills/issue/SKILL.md").exists());
        assert!(tmp.path().join(".codex/prompts/issue.md").exists());
    }

    #[test]
    fn install_writes_both_with_all() {
        let tmp = tempfile::tempdir().unwrap();
        install_skill(tmp.path(), &[Agent::Claude, Agent::Codex], false).unwrap();
        assert!(tmp.path().join(".claude/skills/issue/SKILL.md").exists());
        assert!(tmp.path().join(".codex/prompts/issue.md").exists());
    }

    #[test]
    fn install_without_force_does_not_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".claude/skills/issue/SKILL.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "user content").unwrap();

        install_skill(tmp.path(), &[Agent::Claude], false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user content");
    }

    #[test]
    fn install_with_force_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".claude/skills/issue/SKILL.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "user content").unwrap();

        install_skill(tmp.path(), &[Agent::Claude], true).unwrap();
        assert_ne!(std::fs::read_to_string(&path).unwrap(), "user content");
    }
}
