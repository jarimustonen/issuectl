use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const ISSUE_CLAUDE_TEMPLATE: &str = include_str!("../templates/issue-skill.md");
const ISSUE_CODEX_TEMPLATE: &str = include_str!("../templates/issue-prompt.md");
const ISSUE_NEW_TEMPLATE: &str = include_str!("../templates/issue-new-skill.md");
const ISSUE_NEW_CODEX_TEMPLATE: &str = include_str!("../templates/issue-new-prompt.md");
const ISSUE_INTAKE_TEMPLATE: &str = include_str!("../templates/issue-intake-skill.md");
const ISSUE_INTAKE_CODEX_TEMPLATE: &str = include_str!("../templates/issue-intake-prompt.md");
pub const ISSUES_AGENTS_TEMPLATE: &str = include_str!("../templates/issues-agents.md");

/// Label for a skill copy mirrored into pi.dev's skill corpus.
const PI_SKILL_LABEL: &str = "pi.dev skill";

/// Substitute build-time tokens (currently `{{ISSUECTL_VERSION}}`) in a
/// template body. Used at install time so the on-disk skill is pinned to
/// the issuectl release that wrote it.
pub fn render_template(body: &str) -> String {
    body.replace("{{ISSUECTL_VERSION}}", env!("CARGO_PKG_VERSION"))
}

/// The pi.dev skill-corpus directory under the user's home:
/// `<HOME>/.pi/agent/skills`. The dual-home mirror writes each Claude-format
/// `SKILL.md` to `<pi_skills_root>/<name>/SKILL.md` so the skill is also
/// discoverable under the pi.dev harness (which invokes it as `/skill:name`).
///
/// Returns `None` when `HOME` is unset: the pi mirror is a derived convenience
/// on top of the repo-local Claude install, never a hard requirement, so an
/// unresolvable home simply skips it rather than failing the install. The
/// binary resolves this and threads it into [`install_skill`]; tests pass an
/// explicit root instead of touching the real home.
pub fn pi_skills_root() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    // Guard against an empty or relative `HOME`: joining onto it would resolve
    // `.pi/agent/skills` relative to the process CWD (typically the target
    // repo), silently polluting it. Only an absolute home yields a pi root.
    if !home.is_absolute() {
        return None;
    }
    Some(home.join(".pi/agent/skills"))
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

    /// The `/issue` skill's directory name under a per-skill layout
    /// (`.claude/skills/<name>/SKILL.md` and the pi.dev mirror
    /// `~/.pi/agent/skills/<name>/SKILL.md`). `Some("issue")` for Claude;
    /// Codex ships a flat `.codex/prompts/issue.md` with no per-skill dir and
    /// is not a claude-format consumer, so it returns `None` and never
    /// participates in the pi mirror.
    pub fn skill_name(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("issue"),
            Self::Codex => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code skill",
            Self::Codex => "Codex prompt",
        }
    }
}

/// The standalone intake-flow skills that ship alongside `/issue`. Like
/// [`Agent`] (which ships `/issue` as a Claude skill *and* a Codex prompt),
/// each of these ships in **both** formats: a Claude skill under
/// `.claude/skills/` and a Codex prompt under `.codex/prompts/` (frontmatter
/// stripped, body identical). They are installed once per selected agent, so
/// Jari's fleet-apply hook distributes them the same way it distributes
/// `/issue`. Their bodies live in `crates/issuectl-core/templates/` (source of
/// truth — a `*-skill.md` Claude variant and a `*-prompt.md` Codex variant per
/// skill) and are dogfooded into this repo; the
/// [`tests::dogfooded_copies_match_templates`] test keeps every copy in sync.
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

    /// The rendered body for this skill in the given agent's format. The
    /// Codex variant is the Claude one with its YAML frontmatter stripped
    /// (body byte-identical), mirroring how `/issue` ships both.
    pub fn template(self, agent: Agent) -> &'static str {
        match (self, agent) {
            (Self::IssueNew, Agent::Claude) => ISSUE_NEW_TEMPLATE,
            (Self::IssueNew, Agent::Codex) => ISSUE_NEW_CODEX_TEMPLATE,
            (Self::IssueIntake, Agent::Claude) => ISSUE_INTAKE_TEMPLATE,
            (Self::IssueIntake, Agent::Codex) => ISSUE_INTAKE_CODEX_TEMPLATE,
        }
    }

    /// Where this skill installs for the given agent: a Claude skill under
    /// `.claude/skills/<slug>/SKILL.md`, or a Codex prompt under
    /// `.codex/prompts/<slug>.md` (matching the `/issue` Codex convention).
    pub fn install_path(self, agent: Agent, repo_root: &Path) -> PathBuf {
        match agent {
            Agent::Claude => repo_root.join(format!(".claude/skills/{}/SKILL.md", self.slug())),
            Agent::Codex => repo_root.join(format!(".codex/prompts/{}.md", self.slug())),
        }
    }

    pub fn label(self, agent: Agent) -> &'static str {
        match (self, agent) {
            (Self::IssueNew, Agent::Claude) => "Claude Code intake filing skill",
            (Self::IssueNew, Agent::Codex) => "Codex intake filing prompt",
            (Self::IssueIntake, Agent::Claude) => "Claude Code intake processing skill",
            (Self::IssueIntake, Agent::Codex) => "Codex intake processing prompt",
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
/// in a stable order: scaffold first, then each agent's `/issue` template in
/// input order, then each agent's intake skills (in [`IntakeSkill::ALL`]
/// order) — again in input order. Callers (the CLI summary printer) rely on
/// this ordering.
///
/// When `pi_root` is `Some` **and** the Claude layout is being installed, each
/// Claude-format `SKILL.md` is additionally mirrored into
/// `<pi_root>/<name>/SKILL.md` (the pi.dev dual-home). The pi mirrors are
/// appended after the primary results, in a stable order: `/issue`, then the
/// intake skills in [`IntakeSkill::ALL`] order. `pi_root` is `None` for a
/// Codex-only selection or when `HOME` is unresolvable (see
/// [`pi_skills_root`]).
pub fn install_skill_summary(
    repo_root: &Path,
    agents: &[Agent],
    force: bool,
    pi_root: Option<&Path>,
) -> Result<Vec<InstallResult>> {
    // The standalone intake skills ship in every selected agent's format —
    // a Claude skill for `--agent claude`, a Codex prompt for `--agent codex`
    // — the same way `/issue` does, so the fleet-apply hook distributes them
    // to both fleets.
    // Capacity: scaffold + per-agent (/issue + intake skills), plus the pi
    // mirrors (/issue + intake skills) when a Claude install has a pi root.
    let per_agent = 1 + IntakeSkill::ALL.len();
    let pi_slots = if pi_root.is_some() && agents.contains(&Agent::Claude) {
        per_agent
    } else {
        0
    };
    let mut results = Vec::with_capacity(agents.len() * per_agent + 1 + pi_slots);
    results.push(install_issues_scaffold(repo_root, force)?);
    for agent in agents {
        results.push(install_agent_template(repo_root, *agent, force)?);
    }
    for agent in agents {
        for skill in IntakeSkill::ALL {
            results.push(install_intake_skill(repo_root, skill, *agent, force)?);
        }
    }

    // Dual-home into pi.dev's skill dir. Whenever the Claude layout is
    // installed, mirror the SAME claude-format `SKILL.md` into
    // `<pi_root>/<name>/SKILL.md` so the skill is discoverable under the
    // pi.dev harness (pi loads it and invokes `/skill:name`; bare `/name`
    // cross-references resolve via pi's injected available-skills list, so no
    // link rewrite is needed — only the target). This is an ADDITIONAL target
    // that never alters the repo-local Claude write.
    //
    // Vendored filter: mirror ONLY `SKILL.md`, never companion resources —
    // matching homebase `dotfiles link`, which copies just the skill body into
    // the pi corpus. The Codex prompts are not mirrored (a Codex-only install
    // has no Claude `SKILL.md` to mirror, and `pi_root` is `None` there
    // regardless). Each pi copy is written independently via
    // [`install_rendered_file`], so it never gates the repo-local install: a
    // present pi copy with a deleted Claude skill still lets a plain install
    // repair the Claude side (the pi copy is simply left in place unless
    // `--force`).
    //
    // Because pi is a derived convenience and NOT a hard requirement, a failed
    // pi write (unwritable `$HOME`, read-only fs, quota) must never fail the
    // whole install — by the time we get here the repo-local Claude/Codex
    // targets are already on disk, and aborting would also skip the remaining
    // `init` steps (hooks, merge driver). So each pi mirror error is warned to
    // stderr and skipped, and the repo-local install still reports success.
    if let Some(pi_root) = pi_root {
        if agents.contains(&Agent::Claude) {
            let mut mirror = |name: &str, template: &str| match install_pi_mirror(
                pi_root, name, template, force,
            ) {
                Ok(r) => results.push(r),
                Err(e) => eprintln!("  ! pi.dev skill mirror skipped for {name}: {e:#}"),
            };
            if let Some(name) = Agent::Claude.skill_name() {
                mirror(name, Agent::Claude.template());
            }
            for skill in IntakeSkill::ALL {
                mirror(skill.slug(), skill.template(Agent::Claude));
            }
        }
    }
    Ok(results)
}

/// Install the issues/AGENTS.md scaffold and one or more agent skill files.
/// See [`install_skill_summary`] for the `pi_root` dual-home semantics.
pub fn install_skill(
    repo_root: &Path,
    agents: &[Agent],
    force: bool,
    pi_root: Option<&Path>,
) -> Result<()> {
    let results = install_skill_summary(repo_root, agents, force, pi_root)?;
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
    // Both agents now ship the intake skills, so print the hint once for any
    // selection rather than duplicating it per agent.
    if !agents.is_empty() {
        println!(
            "  Use /issue-new to file an intake report and /issue-intake to process the queue."
        );
    }
    // The pi mirror only fires for a Claude install with a resolved home.
    if pi_root.is_some() && agents.contains(&Agent::Claude) {
        println!(
            "  The same skills are mirrored into ~/.pi/agent/skills for pi.dev (/skill:issue)."
        );
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
    agent: Agent,
    force: bool,
) -> Result<InstallResult> {
    install_rendered_file(
        skill.install_path(agent, repo_root),
        skill.template(agent),
        skill.label(agent),
        force,
    )
}

/// Mirror one Claude-format skill body into pi.dev's skill corpus at
/// `<pi_root>/<name>/SKILL.md`. Byte-identical to the repo-local Claude
/// `SKILL.md` (same template, same version substitution); `force` governs the
/// overwrite the same way as every other install target.
fn install_pi_mirror(
    pi_root: &Path,
    name: &str,
    template: &str,
    force: bool,
) -> Result<InstallResult> {
    install_rendered_file(
        pi_root.join(name).join("SKILL.md"),
        template,
        PI_SKILL_LABEL,
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
        install_skill(tmp.path(), &[Agent::Claude], false, None).unwrap();
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

        // Decide the packaged-crate exception ONCE, at the repository level:
        // a packaged/standalone crate won't have the repo-root copies, so skip
        // the whole test. Deciding this per-copy (skipping any missing file)
        // would turn a deleted or never-installed dogfood copy into a silent
        // pass — precisely the drift this test exists to catch. Use the
        // `/issue` Claude copy as the workspace sentinel.
        if !Agent::Claude.install_path(&repo_root).exists() {
            return; // packaged/standalone crate — no repo-root copies
        }

        // Compare one required dogfooded copy against its rendered template,
        // tolerating only the pinned `{{ISSUECTL_VERSION}}` (the copy records
        // the release that wrote it, which lags the in-development version).
        // A missing copy is a failure, not a skip.
        let check = |copy_path: PathBuf, template: &str| {
            assert!(
                copy_path.is_file(),
                "required dogfooded copy is missing: {} — run \
                 `issuectl skill install --agent all --force`",
                copy_path.display()
            );
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
        // The standalone intake skills ship in both formats too — a Claude
        // skill and a Codex prompt each — and are dogfooded the same way.
        // Deleting/renaming a template or letting any copy drift must fail
        // here just like it does for `/issue`.
        for skill in IntakeSkill::ALL {
            for agent in [Agent::Claude, Agent::Codex] {
                check(skill.install_path(agent, &repo_root), skill.template(agent));
            }
        }
    }

    /// Content guard for the standalone intake skills. `/issue-new` and
    /// `/issue-intake` are binary-shipped via [`IntakeSkill`] in both agent
    /// formats (a Claude skill and a Codex prompt) —
    /// `dogfooded_copies_match_templates` keeps every copy byte-identical to
    /// `templates/` (which transitively covers the Codex prompts, since their
    /// bodies equal the Claude ones this test checks); *this* test pins the
    /// load-bearing filing/processing split and CLI-spelling contract so a
    /// template edit can't quietly break the flow. `/triage-bugs` stays a
    /// repo-local-only
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
        install_skill(tmp.path(), &[Agent::Claude], false, None).unwrap();
        assert!(tmp.path().join(".claude/skills/issue/SKILL.md").exists());
        assert!(!tmp.path().join(".codex/prompts/issue.md").exists());
        assert!(tmp.path().join("issues/AGENTS.md").exists());
    }

    #[test]
    fn install_writes_codex_only() {
        let tmp = tempfile::tempdir().unwrap();
        install_skill(tmp.path(), &[Agent::Codex], false, None).unwrap();
        assert!(!tmp.path().join(".claude/skills/issue/SKILL.md").exists());
        assert!(tmp.path().join(".codex/prompts/issue.md").exists());
    }

    #[test]
    fn install_claude_writes_intake_skills() {
        let tmp = tempfile::tempdir().unwrap();
        install_skill(tmp.path(), &[Agent::Claude], false, None).unwrap();
        for skill in IntakeSkill::ALL {
            assert!(
                skill.install_path(Agent::Claude, tmp.path()).exists(),
                "{} should be installed with the Claude agent",
                skill.slug()
            );
        }
    }

    #[test]
    fn install_codex_writes_intake_skills() {
        // The intake skills now ship a Codex prompt too, so a Codex-only
        // install writes them under `.codex/prompts/<slug>.md`.
        let tmp = tempfile::tempdir().unwrap();
        install_skill(tmp.path(), &[Agent::Codex], false, None).unwrap();
        for skill in IntakeSkill::ALL {
            let path = skill.install_path(Agent::Codex, tmp.path());
            assert!(
                path.exists(),
                "{} should ship as a Codex prompt on a Codex install",
                skill.slug()
            );
            assert!(
                path.ends_with(format!(".codex/prompts/{}.md", skill.slug())),
                "{} Codex prompt must land under .codex/prompts/",
                skill.slug()
            );
            // The installed prompt is the rendered Codex template verbatim:
            // frontmatter stripped and the version token substituted.
            let installed = std::fs::read_to_string(&path).unwrap();
            assert_eq!(installed, render_template(skill.template(Agent::Codex)));
            assert!(
                !installed.starts_with("---\n"),
                "{} Codex prompt must not carry YAML frontmatter",
                skill.slug()
            );
        }
    }

    #[test]
    fn installed_intake_skills_pin_current_version() {
        let tmp = tempfile::tempdir().unwrap();
        install_skill(tmp.path(), &[Agent::Claude], false, None).unwrap();
        for skill in IntakeSkill::ALL {
            let installed =
                std::fs::read_to_string(skill.install_path(Agent::Claude, tmp.path())).unwrap();
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
    fn intake_skills_install_paths_per_agent() {
        // Each intake skill ships in both formats: a Claude skill under
        // `.claude/skills/<slug>/SKILL.md` and a Codex prompt under
        // `.codex/prompts/<slug>.md` (matching the `/issue` convention).
        let root = Path::new("/tmp/repo");
        assert!(IntakeSkill::IssueNew
            .install_path(Agent::Claude, root)
            .ends_with(".claude/skills/issue-new/SKILL.md"));
        assert!(IntakeSkill::IssueNew
            .install_path(Agent::Codex, root)
            .ends_with(".codex/prompts/issue-new.md"));
        assert!(IntakeSkill::IssueIntake
            .install_path(Agent::Claude, root)
            .ends_with(".claude/skills/issue-intake/SKILL.md"));
        assert!(IntakeSkill::IssueIntake
            .install_path(Agent::Codex, root)
            .ends_with(".codex/prompts/issue-intake.md"));
        assert_eq!(IntakeSkill::ALL.len(), 2);
    }

    #[test]
    fn intake_codex_prompt_strips_frontmatter() {
        // The Codex prompt is the Claude skill with its YAML frontmatter
        // removed; the body must be identical (same as `/issue`).
        for skill in IntakeSkill::ALL {
            let claude = skill.template(Agent::Claude);
            let codex = skill.template(Agent::Codex);
            assert!(
                claude.starts_with("---\n"),
                "{} Claude template must carry YAML frontmatter",
                skill.slug()
            );
            assert!(
                !codex.starts_with("---\n"),
                "{} Codex prompt must strip the frontmatter",
                skill.slug()
            );
            // The Claude body after the closing `---` equals the Codex prompt.
            // Anchor on the *opening* `---` before splitting on the closing
            // fence, so a `\n---\n` inside the body (e.g. a markdown horizontal
            // rule) can't be mistaken for the frontmatter delimiter.
            let body = claude
                .strip_prefix("---\n")
                .and_then(|rest| rest.split_once("\n---\n"))
                .expect("well-formed frontmatter")
                .1;
            assert_eq!(body, codex, "{} bodies must match", skill.slug());
        }
    }

    #[test]
    fn install_result_order_is_scaffold_then_agents_then_intake() {
        // The dogfood/print paths rely on install-result order being stable:
        // scaffold, each agent in input order, then the intake skills in
        // `IntakeSkill::ALL` order.
        let tmp = tempfile::tempdir().unwrap();
        let results = install_skill_summary(tmp.path(), &[Agent::Claude], false, None).unwrap();
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
    fn install_result_order_is_stable_for_multiple_agents() {
        // Pin the full order for `[Claude, Codex]`: scaffold, each agent's
        // `/issue` in input order, then each agent's intake skills in
        // `IntakeSkill::ALL` order — again in input order.
        let tmp = tempfile::tempdir().unwrap();
        let results =
            install_skill_summary(tmp.path(), &[Agent::Claude, Agent::Codex], false, None).unwrap();
        let expected = [
            "issues/AGENTS.md",
            ".claude/skills/issue/SKILL.md",
            ".codex/prompts/issue.md",
            ".claude/skills/issue-new/SKILL.md",
            ".claude/skills/issue-intake/SKILL.md",
            ".codex/prompts/issue-new.md",
            ".codex/prompts/issue-intake.md",
        ];
        assert_eq!(results.len(), expected.len());
        for (r, tail) in results.iter().zip(expected) {
            assert!(
                r.path.ends_with(tail),
                "expected {tail}, got {}",
                r.path.display()
            );
        }
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

        let results = install_skill_summary(tmp.path(), &[Agent::Claude], true, None).unwrap();
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
        // Exercised for both agent formats so the Codex prompt path gets the
        // same create / preserve / overwrite coverage as the Claude skill.
        for agent in [Agent::Claude, Agent::Codex] {
            let tmp = tempfile::tempdir().unwrap();
            let path = IntakeSkill::IssueNew.install_path(agent, tmp.path());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "user content").unwrap();

            // Without --force the user's copy is preserved.
            install_skill(tmp.path(), &[agent], false, None).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "user content");

            // With --force it is regenerated from the template.
            install_skill(tmp.path(), &[agent], true, None).unwrap();
            assert_ne!(std::fs::read_to_string(&path).unwrap(), "user content");
        }
    }

    #[test]
    fn install_writes_both_with_all() {
        let tmp = tempfile::tempdir().unwrap();
        install_skill(tmp.path(), &[Agent::Claude, Agent::Codex], false, None).unwrap();
        // All six copies land: `/issue` plus both intake skills, per agent.
        for p in [
            ".claude/skills/issue/SKILL.md",
            ".codex/prompts/issue.md",
            ".claude/skills/issue-new/SKILL.md",
            ".claude/skills/issue-intake/SKILL.md",
            ".codex/prompts/issue-new.md",
            ".codex/prompts/issue-intake.md",
        ] {
            assert!(tmp.path().join(p).exists(), "{p} should be installed");
        }
    }

    #[test]
    fn install_without_force_does_not_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".claude/skills/issue/SKILL.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "user content").unwrap();

        install_skill(tmp.path(), &[Agent::Claude], false, None).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user content");
    }

    #[test]
    fn install_with_force_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".claude/skills/issue/SKILL.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "user content").unwrap();

        install_skill(tmp.path(), &[Agent::Claude], true, None).unwrap();
        assert_ne!(std::fs::read_to_string(&path).unwrap(), "user content");
    }

    // ── pi.dev dual-home ────────────────────────────────────────────────────

    /// A Claude install with a pi root mirrors every Claude `SKILL.md` into
    /// `<pi_root>/<name>/SKILL.md` byte-for-byte, and leaves the repo-local
    /// Claude path untouched (regression guard for the primary target).
    #[test]
    fn install_dual_homes_claude_skills_into_pi() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();

        for (claude_rel, name) in [
            (".claude/skills/issue/SKILL.md", "issue"),
            (".claude/skills/issue-new/SKILL.md", "issue-new"),
            (".claude/skills/issue-intake/SKILL.md", "issue-intake"),
        ] {
            // Claude path unchanged.
            let claude_path = repo.path().join(claude_rel);
            assert!(claude_path.exists(), "{claude_rel} must still be installed");

            // pi mirror present and byte-identical to the Claude copy.
            let pi_path = pi.path().join(name).join("SKILL.md");
            assert!(
                pi_path.exists(),
                "{name} must be mirrored into the pi corpus"
            );
            assert_eq!(
                std::fs::read_to_string(&pi_path).unwrap(),
                std::fs::read_to_string(&claude_path).unwrap(),
                "{name} pi mirror must be byte-identical to the Claude SKILL.md"
            );
        }
    }

    /// Vendored filter: ONLY `SKILL.md` is mirrored. The Codex prompts and the
    /// `issues/AGENTS.md` scaffold are never written into the pi corpus.
    #[test]
    fn pi_mirror_only_carries_skill_md() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(
            repo.path(),
            &[Agent::Claude, Agent::Codex],
            false,
            Some(pi.path()),
        )
        .unwrap();

        // Only three per-skill dirs, each holding exactly one SKILL.md.
        let mut names: Vec<String> = std::fs::read_dir(pi.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["issue", "issue-intake", "issue-new"]);
        for name in &names {
            let entries: Vec<_> = std::fs::read_dir(pi.path().join(name))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            assert_eq!(
                entries,
                ["SKILL.md"],
                "{name} pi dir must hold only SKILL.md"
            );
        }
    }

    /// The pi mirror is Claude-format only: a Codex-only install writes no pi
    /// copies even when a pi root is supplied.
    #[test]
    fn codex_only_install_skips_pi_mirror() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Codex], false, Some(pi.path())).unwrap();
        assert!(
            std::fs::read_dir(pi.path()).unwrap().next().is_none(),
            "a Codex-only install must not write into the pi corpus"
        );
    }

    /// `pi_root = None` (HOME unset) installs the Claude skills but writes no
    /// pi mirror — the mirror is a derived convenience, never required.
    #[test]
    fn no_pi_root_installs_claude_without_mirror() {
        let repo = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, None).unwrap();
        assert!(repo.path().join(".claude/skills/issue/SKILL.md").exists());
    }

    /// Idempotency: a second non-force install leaves the pi mirror in place
    /// (reported `AlreadyExists`); `--force` refreshes it. And a present pi
    /// copy never blocks repairing a deleted Claude skill — the pi write is
    /// independent, so the Claude side is recreated while pi is left as-is.
    #[test]
    fn pi_mirror_is_idempotent_and_never_gates_claude_repair() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        let pi_issue = pi.path().join("issue/SKILL.md");
        let claude_issue = repo.path().join(".claude/skills/issue/SKILL.md");

        // First install writes both.
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        assert!(pi_issue.exists() && claude_issue.exists());

        // Second non-force install is idempotent: the pi copy is reported
        // AlreadyExists, not rewritten or errored.
        let results =
            install_skill_summary(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        let pi_outcome = results
            .iter()
            .find(|r| r.path == pi_issue)
            .expect("pi issue mirror present in summary");
        assert_eq!(pi_outcome.outcome, InstallOutcome::AlreadyExists);
        assert_eq!(pi_outcome.label, PI_SKILL_LABEL);

        // Delete the Claude skill but keep the pi copy. A plain (non-force)
        // install must repair the Claude side without being blocked by the
        // pre-existing pi mirror.
        std::fs::remove_file(&claude_issue).unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        assert!(
            claude_issue.exists(),
            "the deleted Claude skill must be repaired even with a present pi mirror"
        );
        assert!(pi_issue.exists(), "the pi mirror must remain in place");
    }

    /// When both the Claude skill AND its pi mirror are missing, a plain
    /// (non-force) install recreates both.
    #[test]
    fn install_recreates_both_when_both_missing() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        let claude_issue = repo.path().join(".claude/skills/issue/SKILL.md");
        let pi_issue = pi.path().join("issue/SKILL.md");
        std::fs::remove_file(&claude_issue).unwrap();
        std::fs::remove_file(&pi_issue).unwrap();

        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        assert!(claude_issue.exists(), "Claude skill must be recreated");
        assert!(pi_issue.exists(), "pi mirror must be recreated");
    }

    /// A failed pi mirror write (unwritable pi root) is non-fatal: the
    /// repo-local Claude install still succeeds and `install_skill` returns
    /// `Ok`. Simulated by planting a regular file where the pi skill dir would
    /// go, so `create_dir_all` on `<pi_root>/issue/` fails.
    #[test]
    fn pi_mirror_failure_is_non_fatal() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        // A file at `<pi_root>/issue` blocks creating the `issue/` skill dir.
        std::fs::write(pi.path().join("issue"), "not a dir").unwrap();

        // Must not error even though the pi mirror for `/issue` cannot be
        // written.
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();

        // The repo-local Claude skills are all installed regardless.
        assert!(repo.path().join(".claude/skills/issue/SKILL.md").exists());
        for skill in IntakeSkill::ALL {
            assert!(skill.install_path(Agent::Claude, repo.path()).exists());
        }
        // The blocked pi entry is absent from the summary (skipped, not
        // errored); the other pi mirrors that CAN be written still land.
        let results =
            install_skill_summary(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        assert!(
            !results
                .iter()
                .any(|r| r.path == pi.path().join("issue/SKILL.md")),
            "the un-writable pi mirror must be skipped, not reported"
        );
        assert!(
            results
                .iter()
                .any(|r| r.path == pi.path().join("issue-new/SKILL.md")),
            "writable pi mirrors must still be installed"
        );
    }

    /// `--force` refreshes a stale pi mirror to the current template.
    #[test]
    fn force_refreshes_pi_mirror() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        let pi_issue = pi.path().join("issue/SKILL.md");
        std::fs::create_dir_all(pi_issue.parent().unwrap()).unwrap();
        std::fs::write(&pi_issue, "stale pi content").unwrap();

        install_skill(repo.path(), &[Agent::Claude], true, Some(pi.path())).unwrap();
        assert_eq!(
            std::fs::read_to_string(&pi_issue).unwrap(),
            render_template(Agent::Claude.template()),
            "--force must refresh the pi mirror to the current template"
        );
    }

    /// The dual-home summary appends pi mirrors after the primary results, in
    /// a stable order: `/issue`, then the intake skills in `IntakeSkill::ALL`
    /// order.
    #[test]
    fn pi_mirrors_appended_in_stable_order() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        let results =
            install_skill_summary(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        // scaffold + issue + 2 intake + 3 pi mirrors.
        assert_eq!(results.len(), 7);
        assert!(results[4].path.ends_with("issue/SKILL.md"));
        assert!(results[5].path.ends_with("issue-new/SKILL.md"));
        assert!(results[6].path.ends_with("issue-intake/SKILL.md"));
        for r in &results[4..] {
            assert!(
                r.path.starts_with(pi.path()),
                "pi mirrors live under pi_root"
            );
        }
    }

    /// The pi mirror pins the running binary's version, same as every other
    /// installed copy.
    #[test]
    fn pi_mirror_pins_current_version() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        let body = std::fs::read_to_string(pi.path().join("issue/SKILL.md")).unwrap();
        assert!(body.contains(env!("CARGO_PKG_VERSION")));
        assert!(!body.contains("{{ISSUECTL_VERSION}}"));
    }

    /// `pi_skills_root()` roots the corpus at `<HOME>/.pi/agent/skills` and
    /// yields `None` when `HOME` is unset.
    #[test]
    fn pi_skills_root_resolves_from_home() {
        // Serialized via a process-global env mutation guard is unnecessary
        // here: we snapshot, mutate, assert, and restore within the test, and
        // no other test reads HOME.
        let saved = std::env::var_os("HOME");
        std::env::set_var("HOME", "/home/example");
        let root = pi_skills_root().unwrap();
        assert!(root.ends_with(".pi/agent/skills"));
        assert!(root.starts_with("/home/example"));
        // Empty or relative HOME must yield None (never a CWD-relative path).
        std::env::set_var("HOME", "");
        assert!(pi_skills_root().is_none(), "empty HOME must yield None");
        std::env::set_var("HOME", "relative/home");
        assert!(pi_skills_root().is_none(), "relative HOME must yield None");
        std::env::remove_var("HOME");
        assert!(pi_skills_root().is_none());
        match saved {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
