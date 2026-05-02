use std::path::Path;

const ISSUE_SKILL_TEMPLATE: &str = include_str!("../templates/issue-skill.md");
const ISSUES_AGENTS_TEMPLATE: &str = include_str!("../templates/issues-agents.md");

/// Install the issue skill and issues documentation into the current repo.
pub fn install_skill(repo_root: &Path, force: bool) -> anyhow::Result<()> {
    let issues_dir = repo_root.join("issues");
    let claude_dir = repo_root.join(".claude").join("skills").join("issue");

    if force || !issues_dir.join("AGENTS.md").exists() {
        if !issues_dir.exists() {
            std::fs::create_dir_all(issues_dir.join("open"))?;
            std::fs::create_dir_all(issues_dir.join("closed"))?;
        }
        std::fs::write(issues_dir.join("AGENTS.md"), ISSUES_AGENTS_TEMPLATE)?;
        println!("  ✓ Created issues/AGENTS.md");
        if !issues_dir.join("open").exists() {
            std::fs::create_dir_all(issues_dir.join("open"))?;
        }
        if !issues_dir.join("closed").exists() {
            std::fs::create_dir_all(issues_dir.join("closed"))?;
        }
    } else {
        println!("  ~ issues/AGENTS.md already exists (use --force to overwrite)");
    }

    if force || !claude_dir.join("SKILL.md").exists() {
        std::fs::create_dir_all(&claude_dir)?;
        std::fs::write(claude_dir.join("SKILL.md"), ISSUE_SKILL_TEMPLATE)?;
        println!("  ✓ Created .claude/skills/issue/SKILL.md");
    } else {
        println!("  ~ .claude/skills/issue/SKILL.md already exists (use --force to overwrite)");
    }

    println!();
    println!("  Use /issue in Claude Code to create, search, update, and close issues.");
    println!("  Or use `issuectl list` to browse issues from the command line.");

    Ok(())
}
