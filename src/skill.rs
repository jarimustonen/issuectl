use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const ISSUE_CLAUDE_TEMPLATE: &str = include_str!("../templates/issue-skill.md");
const ISSUE_CODEX_TEMPLATE: &str = include_str!("../templates/issue-prompt.md");
const ISSUES_AGENTS_TEMPLATE: &str = include_str!("../templates/issues-agents.md");

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

/// Install the issues/AGENTS.md scaffold and one or more agent skill files.
pub fn install_skill(repo_root: &Path, agents: &[Agent], force: bool) -> Result<()> {
    install_issues_scaffold(repo_root, force)?;

    for agent in agents {
        install_agent_template(repo_root, *agent, force)?;
    }

    println!();
    if agents.iter().any(|a| *a == Agent::Claude) {
        println!("  Use /issue in Claude Code to create, search, update, and close issues.");
    }
    if agents.iter().any(|a| *a == Agent::Codex) {
        println!("  Use /issue in Codex CLI (or invoke the prompt) to manage issues.");
    }
    println!("  Or use `issuectl list` to browse issues from the command line.");
    Ok(())
}

/// Print the template that would be installed for the given agent to stdout.
pub fn print_skill(agent: Agent) -> Result<()> {
    print!("{}", agent.template());
    Ok(())
}

fn install_issues_scaffold(repo_root: &Path, force: bool) -> Result<()> {
    let issues_dir = repo_root.join("issues");
    let agents_md = issues_dir.join("AGENTS.md");

    if !issues_dir.exists() {
        std::fs::create_dir_all(issues_dir.join("open"))
            .with_context(|| format!("cannot create {}", issues_dir.join("open").display()))?;
        std::fs::create_dir_all(issues_dir.join("closed"))
            .with_context(|| format!("cannot create {}", issues_dir.join("closed").display()))?;
    }

    if force || !agents_md.exists() {
        std::fs::write(&agents_md, ISSUES_AGENTS_TEMPLATE)
            .with_context(|| format!("cannot write {}", agents_md.display()))?;
        println!("  ✓ Created issues/AGENTS.md");
    } else {
        println!("  ~ issues/AGENTS.md already exists (use --force to overwrite)");
    }

    if !issues_dir.join("open").exists() {
        std::fs::create_dir_all(issues_dir.join("open"))?;
    }
    if !issues_dir.join("closed").exists() {
        std::fs::create_dir_all(issues_dir.join("closed"))?;
    }
    Ok(())
}

fn install_agent_template(repo_root: &Path, agent: Agent, force: bool) -> Result<()> {
    let path = agent.install_path(repo_root);
    let display = path
        .strip_prefix(repo_root)
        .unwrap_or(&path)
        .display()
        .to_string();

    if !force && path.exists() {
        println!(
            "  ~ {display} already exists (use --force to overwrite)"
        );
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::write(&path, agent.template())
        .with_context(|| format!("cannot write {}", path.display()))?;
    println!("  ✓ Created {display} ({})", agent.label());
    Ok(())
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
        assert!(
            Agent::Claude
                .install_path(root)
                .ends_with(".claude/skills/issue/SKILL.md")
        );
        assert!(
            Agent::Codex
                .install_path(root)
                .ends_with(".codex/prompts/issue.md")
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
