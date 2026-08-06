use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const ISSUE_CLAUDE_TEMPLATE: &str = include_str!("../templates/issue-skill.md");
const ISSUE_CODEX_TEMPLATE: &str = include_str!("../templates/issue-prompt.md");
const ISSUE_NEW_TEMPLATE: &str = include_str!("../templates/issue-new-skill.md");
const ISSUE_INTAKE_TEMPLATE: &str = include_str!("../templates/issue-intake-skill.md");
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

/// The standalone intake-flow skills that ship alongside `/issue`. Unlike
/// [`Agent`] (which ships `/issue` as a Claude skill *and* a Codex prompt),
/// these are **Claude-only** — they orchestrate the `/worktree-*` family and
/// have no Codex variant. They are installed whenever [`Agent::Claude`] is
/// among the selected agents, so Jari's fleet-apply hook distributes them the
/// same way it distributes `/issue`. Their bodies live in
/// `crates/issuectl-core/templates/` (source of truth) and are dogfooded into
/// this repo's `.claude/skills/`; the
/// [`tests::dogfooded_copies_match_templates`] test keeps the two in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntakeSkill {
    /// `/issue-new` — the thin filing half of the intake flow.
    IssueNew,
    /// `/issue-intake` — the read-only processing/briefing half.
    IssueIntake,
}

impl IntakeSkill {
    /// Every intake skill, in install order. Ordering is semantic — filing
    /// (`/issue-new`) before processing (`/issue-intake`), matching the flow's
    /// lifecycle — and load-bearing for the install summary; do not reorder
    /// for cosmetic reasons.
    pub const ALL: [IntakeSkill; 2] = [IntakeSkill::IssueNew, IntakeSkill::IssueIntake];

    /// The skill's directory name under `.claude/skills/` (and its `/name`).
    pub fn slug(self) -> &'static str {
        match self {
            Self::IssueNew => "issue-new",
            Self::IssueIntake => "issue-intake",
        }
    }

    pub fn template(self) -> &'static str {
        match self {
            Self::IssueNew => ISSUE_NEW_TEMPLATE,
            Self::IssueIntake => ISSUE_INTAKE_TEMPLATE,
        }
    }

    pub fn install_path(self, repo_root: &Path) -> PathBuf {
        repo_root.join(format!(".claude/skills/{}/SKILL.md", self.slug()))
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::IssueNew => "Claude Code intake filing skill",
            Self::IssueIntake => "Claude Code intake processing skill",
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
    // The standalone intake skills are Claude-only; ship them whenever the
    // Claude agent is selected, so the fleet-apply hook distributes them the
    // same way it distributes `/issue`. A Codex-only install skips them.
    let ships_intake = agents.contains(&Agent::Claude);
    let intake_count = if ships_intake {
        IntakeSkill::ALL.len()
    } else {
        0
    };
    let mut results = Vec::with_capacity(agents.len() + 1 + intake_count);
    results.push(install_issues_scaffold(repo_root, force)?);
    for agent in agents {
        results.push(install_agent_template(repo_root, *agent, force)?);
    }
    if ships_intake {
        for skill in IntakeSkill::ALL {
            results.push(install_intake_skill(repo_root, skill, force)?);
        }
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
        println!(
            "  Use /issue-new to file an intake report and /issue-intake to process the queue."
        );
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
    install_rendered_file(
        agent.install_path(repo_root),
        agent.template(),
        agent.label(),
        force,
    )
}

fn install_intake_skill(
    repo_root: &Path,
    skill: IntakeSkill,
    force: bool,
) -> Result<InstallResult> {
    install_rendered_file(
        skill.install_path(repo_root),
        skill.template(),
        skill.label(),
        force,
    )
}

/// Render `template` (substituting build-time tokens) and write it to `path`,
/// respecting `force` for the overwrite decision. Shared by the `/issue`
/// agent templates and the standalone intake skills so they handle
/// creation, `--force` re-install, and parent-dir creation identically.
fn install_rendered_file(
    path: PathBuf,
    template: &str,
    label: &str,
    force: bool,
) -> Result<InstallResult> {
    let existed = path.exists();

    if !force && existed {
        return Ok(InstallResult {
            path,
            label: label.to_string(),
            outcome: InstallOutcome::AlreadyExists,
        });
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::write(&path, render_template(template))
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(InstallResult {
        path,
        label: label.to_string(),
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

        // Compare one dogfooded copy against its rendered template, tolerating
        // only the pinned `{{ISSUECTL_VERSION}}` (the copy records the release
        // that wrote it, which lags the in-development version). Dev-only: a
        // packaged/standalone crate won't have the repo-root copies, so skip
        // rather than fail outside the workspace.
        let check = |copy_path: PathBuf, template: &str| {
            if !copy_path.exists() {
                return;
            }
            let copy = std::fs::read_to_string(&copy_path).unwrap();
            let expected = template.replace("{{ISSUECTL_VERSION}}", pinned_version(&copy));
            assert_eq!(
                copy,
                expected,
                "{} has drifted from its template; run \
                 `issuectl skill install --agent all --force` to regenerate",
                copy_path.display()
            );
        };

        // `/issue` ships as a Claude skill and a Codex prompt.
        for agent in [Agent::Claude, Agent::Codex] {
            check(agent.install_path(&repo_root), agent.template());
        }
        // The standalone intake skills ship Claude-only, but are dogfooded the
        // same way — deleting/renaming a template or letting a copy drift
        // must fail here just like it does for `/issue`.
        for skill in IntakeSkill::ALL {
            check(skill.install_path(&repo_root), skill.template());
        }
    }

    /// Content guard for the standalone intake skills. `/issue-new` and
    /// `/issue-intake` are now binary-shipped (Claude-only) via
    /// [`IntakeSkill`] — `dogfooded_copies_match_templates` keeps their copies
    /// byte-identical to `templates/`; *this* test pins the load-bearing
    /// filing/processing split and CLI-spelling contract so a template edit
    /// can't quietly break the flow. `/triage-bugs` stays a repo-local-only
    /// deprecation alias (not promoted to a template). Skipped outside the
    /// workspace (a packaged crate has no repo-root copies), matching
    /// `dogfooded_copies_match_templates`.
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

        // Contract guard: the exact `intake` subcommand spellings the docs
        // hand to a downstream agent must not silently drift from the CLI. A
        // wrong spelling (`needs-info` instead of the `need-info` command, or
        // `--source_ref` instead of `--source-ref`) would make the agent's
        // command fail. Cross-check the load-bearing spellings against the
        // dogfooded `/issue` reference skill (which mirrors the CLI surface).
        let issue_ref = read("issue"); // .claude/skills/issue/SKILL.md

        // Every intake verb the reference documents, spelled as the CLI expects.
        for cmd in [
            "file",
            "queue",
            "show",
            "accept",
            "defer",
            "need-info",
            "reject",
            "cannot-reproduce",
            "duplicate",
            "obsolete",
            "retype",
            "reopen",
            "withdraw",
        ] {
            let pattern = format!("intake {cmd}");
            assert!(
                issue_ref.contains(&pattern),
                "/issue reference skill must document `issuectl {pattern}`"
            );
        }

        // The command is `need-info`; the *status* is `needs-info`. The wrong
        // command form (`intake needs-info`) must never appear — it is the most
        // likely spelling slip and it would fail at the CLI.
        for skill in [&issue_intake, &issue_ref, &issue_new] {
            assert!(
                !skill.contains("intake needs-info"),
                "the intake command is `need-info`, not `needs-info`"
            );
        }

        // Flag spelling: kebab-case, not snake_case.
        assert!(
            issue_new.contains("--source-ref") && !issue_new.contains("--source_ref"),
            "issue-new must document `--source-ref` (kebab-case)"
        );

        // The intake statuses and the closed `disposition_reason` enum must
        // stay listed in the reference so `-s/--status` filtering and reason
        // vocabulary don't drift from the schema.
        for token in [
            "untriaged",
            "needs-info",
            "deferred",
            "by-design",
            "out-of-scope",
            "withdrawn",
            "superseded",
        ] {
            assert!(
                issue_ref.contains(token),
                "/issue reference skill must document intake token `{token}`"
            );
        }
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
    fn install_claude_writes_intake_skills() {
        let tmp = tempfile::tempdir().unwrap();
        install_skill(tmp.path(), &[Agent::Claude], false).unwrap();
        for skill in IntakeSkill::ALL {
            assert!(
                skill.install_path(tmp.path()).exists(),
                "{} should be installed with the Claude agent",
                skill.slug()
            );
        }
    }

    #[test]
    fn install_codex_only_skips_intake_skills() {
        let tmp = tempfile::tempdir().unwrap();
        install_skill(tmp.path(), &[Agent::Codex], false).unwrap();
        for skill in IntakeSkill::ALL {
            assert!(
                !skill.install_path(tmp.path()).exists(),
                "{} is Claude-only and must not ship on a Codex-only install",
                skill.slug()
            );
        }
    }

    #[test]
    fn installed_intake_skills_pin_current_version() {
        let tmp = tempfile::tempdir().unwrap();
        install_skill(tmp.path(), &[Agent::Claude], false).unwrap();
        for skill in IntakeSkill::ALL {
            let installed = std::fs::read_to_string(skill.install_path(tmp.path())).unwrap();
            assert!(
                installed.contains(env!("CARGO_PKG_VERSION")),
                "{} must pin the current version",
                skill.slug()
            );
            assert!(
                !installed.contains("{{ISSUECTL_VERSION}}"),
                "{} must not leave the raw version token",
                skill.slug()
            );
        }
    }

    #[test]
    fn intake_skills_are_claude_only() {
        // Guards the Claude-only design decision: these two skills ship no
        // Codex variant (they orchestrate the Claude-side `/worktree-*` family).
        let root = Path::new("/tmp/repo");
        assert!(IntakeSkill::IssueNew
            .install_path(root)
            .ends_with(".claude/skills/issue-new/SKILL.md"));
        assert!(IntakeSkill::IssueIntake
            .install_path(root)
            .ends_with(".claude/skills/issue-intake/SKILL.md"));
        assert_eq!(IntakeSkill::ALL.len(), 2);
    }

    #[test]
    fn install_result_order_is_scaffold_then_agents_then_intake() {
        // The dogfood/print paths rely on install-result order being stable:
        // scaffold, each agent in input order, then the intake skills in
        // `IntakeSkill::ALL` order.
        let tmp = tempfile::tempdir().unwrap();
        let results = install_skill_summary(tmp.path(), &[Agent::Claude], false).unwrap();
        assert_eq!(results.len(), 4);
        assert!(results[0].path.ends_with("issues/AGENTS.md"));
        assert!(results[1].path.ends_with(".claude/skills/issue/SKILL.md"));
        assert!(results[2]
            .path
            .ends_with(".claude/skills/issue-new/SKILL.md"));
        assert!(results[3]
            .path
            .ends_with(".claude/skills/issue-intake/SKILL.md"));
    }

    #[test]
    fn force_reinstall_reports_mixed_outcomes() {
        // A `--force` install where the `/issue` skill already exists but the
        // intake skills do not must report `Overwritten` for the former and
        // `Created` for the latter in the same call.
        let tmp = tempfile::tempdir().unwrap();
        let issue = Agent::Claude.install_path(tmp.path());
        std::fs::create_dir_all(issue.parent().unwrap()).unwrap();
        std::fs::write(&issue, "pre-existing").unwrap();

        let results = install_skill_summary(tmp.path(), &[Agent::Claude], true).unwrap();
        let outcome = |needle: &str| {
            results
                .iter()
                .find(|r| r.path.ends_with(needle))
                .map(|r| r.outcome.clone())
                .unwrap_or_else(|| panic!("no result for {needle}"))
        };
        assert_eq!(
            outcome(".claude/skills/issue/SKILL.md"),
            InstallOutcome::Overwritten
        );
        assert_eq!(
            outcome(".claude/skills/issue-new/SKILL.md"),
            InstallOutcome::Created
        );
        assert_eq!(
            outcome(".claude/skills/issue-intake/SKILL.md"),
            InstallOutcome::Created
        );
    }

    #[test]
    fn intake_skill_reinstall_respects_force() {
        let tmp = tempfile::tempdir().unwrap();
        let path = IntakeSkill::IssueNew.install_path(tmp.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "user content").unwrap();

        // Without --force the user's copy is preserved.
        install_skill(tmp.path(), &[Agent::Claude], false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user content");

        // With --force it is regenerated from the template.
        install_skill(tmp.path(), &[Agent::Claude], true).unwrap();
        assert_ne!(std::fs::read_to_string(&path).unwrap(), "user content");
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
