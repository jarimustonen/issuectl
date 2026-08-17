use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const ISSUE_CLAUDE_TEMPLATE: &str = include_str!("../templates/issue-skill.md");
const ISSUE_CODEX_TEMPLATE: &str = include_str!("../templates/issue-prompt.md");
const ISSUE_NEW_TEMPLATE: &str = include_str!("../templates/issue-new-skill.md");
const ISSUE_NEW_CODEX_TEMPLATE: &str = include_str!("../templates/issue-new-prompt.md");
const ISSUE_INTAKE_TEMPLATE: &str = include_str!("../templates/issue-intake-skill.md");
const ISSUE_INTAKE_CODEX_TEMPLATE: &str = include_str!("../templates/issue-intake-prompt.md");
pub const ISSUES_AGENTS_TEMPLATE: &str = include_str!("../templates/issues-agents.md");

/// One install destination for a bundled companion skill. `agent` is always a
/// value accepted by `skill install --agent`; pi.dev is deliberately absent
/// because it is a derived mirror of a Claude install, not an independently
/// selectable format (inspect that mirror with `skill pi-status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillInstallTarget {
    pub agent: String,
    pub label: String,
    pub path: String,
}

/// A bundled companion skill that `issuectl skill install` can write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillCatalogEntry {
    pub name: String,
    pub description: String,
    pub install_targets: Vec<SkillInstallTarget>,
}

/// Version metadata for a skill bundled in this binary, used by `version` for
/// a one-call skill/CLI drift audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillVersion {
    pub name: String,
    pub cli_version: String,
    pub schema_version: u32,
}

/// Return the version pins of all bundled Claude-format skills.
pub fn skill_versions() -> Vec<SkillVersion> {
    ["issue", "issue-new", "issue-intake"]
        .into_iter()
        .map(|name| SkillVersion {
            name: name.to_string(),
            cli_version: env!("CARGO_PKG_VERSION").to_string(),
            schema_version: 1,
        })
        .collect()
}

fn install_target(agent: Agent, label: &str, path: PathBuf) -> SkillInstallTarget {
    SkillInstallTarget {
        agent: agent.argument().to_string(),
        label: label.to_string(),
        path: path.display().to_string(),
    }
}

/// Return the bundled companion-skill catalog in stable install order.
///
/// The catalog derives names, labels, and paths from the same `Agent` and
/// `IntakeSkill` methods that installation uses. Read-only: it describes the
/// Claude and Codex variants this binary can install, never inspecting or
/// changing the derived pi.dev mirror.
pub fn skill_catalog() -> Vec<SkillCatalogEntry> {
    let root = Path::new("");
    let agents = [Agent::Claude, Agent::Codex];
    let mut catalog = Vec::with_capacity(1 + IntakeSkill::ALL.len());

    catalog.push(SkillCatalogEntry {
        name: Agent::Claude.skill_name().unwrap_or("issue").to_string(),
        description: "Manage issues and epics in issues/.".to_string(),
        install_targets: agents
            .iter()
            .map(|agent| install_target(*agent, agent.label(), agent.install_path(root)))
            .collect(),
    });
    for skill in IntakeSkill::ALL {
        let description = match skill {
            IntakeSkill::IssueNew => {
                "Faithfully file an incoming bug report or feature request into intake."
            }
            IntakeSkill::IssueIntake => {
                "Read and brief the actionable intake queue without applying a disposition."
            }
        };
        catalog.push(SkillCatalogEntry {
            name: skill.slug().to_string(),
            description: description.to_string(),
            install_targets: agents
                .iter()
                .map(|agent| {
                    install_target(
                        *agent,
                        skill.label(*agent),
                        skill.install_path(*agent, root),
                    )
                })
                .collect(),
        });
    }
    catalog
}

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
    /// The value accepted by `skill install --agent` for this concrete format.
    pub fn argument(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

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
/// the deployment hook distributes them the same way it distributes
/// `/issue`. Their bodies live in `crates/issuectl-core/templates/` (source of
/// truth — a `*-skill.md` Claude variant and a `*-prompt.md` Codex variant per
/// skill) and are dogfooded into this repo; the
/// `tests::dogfooded_copies_match_templates` test keeps every copy in sync.
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallOutcome {
    /// File did not exist; we wrote it.
    Created,
    /// File existed and the applicable force flag was supplied; we overwrote it.
    Overwritten,
    /// File already existed and `--force` was not supplied.
    AlreadyExists,
    /// The existing scaffold differs from the bundled scaffold and was preserved.
    RepoAuthoredContentPreserved,
}

#[derive(Debug, Clone, serde::Serialize)]
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
    install_skill_summary_with_scaffold_force(repo_root, agents, force, false, pi_root)
}

/// Like [`install_skill_summary`], with an explicit override for regenerating
/// a diverged, repo-authored `issues/AGENTS.md` scaffold.
pub fn install_skill_summary_with_scaffold_force(
    repo_root: &Path,
    agents: &[Agent],
    force: bool,
    force_scaffold: bool,
    pi_root: Option<&Path>,
) -> Result<Vec<InstallResult>> {
    // The standalone intake skills ship in every selected agent's format,
    // just like `/issue`, so an installer hook can distribute them to every
    // configured agent.
    // Capacity: scaffold + per-agent (/issue + intake skills), plus the pi
    // mirrors (/issue + intake skills) when a Claude install has a pi root.
    let per_agent = 1 + IntakeSkill::ALL.len();
    let pi_slots = if pi_root.is_some() && agents.contains(&Agent::Claude) {
        per_agent
    } else {
        0
    };
    let mut results = Vec::with_capacity(agents.len() * per_agent + 1 + pi_slots);
    results.push(install_issues_scaffold(repo_root, force, force_scaffold)?);
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
    // Vendored filter: mirror ONLY `SKILL.md`, never companion resources,
    // matching dotfile linkers that copy just the skill body into the pi
    // corpus. The Codex prompts are not mirrored (a Codex-only install
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
            // Serialize the whole pi block — mirror writes AND the manifest
            // read-modify-write — against a concurrent install/prune from
            // another repo sharing the same global corpus. Acquisition BLOCKS
            // if a peer holds the lock (the flock discipline the rest of
            // `mutate/` uses): concurrent writers wait and serialize, they do
            // not skip. The `Err` arm therefore fires only on a genuine failure
            // to create/open the lock file (unwritable `$HOME`, read-only fs,
            // permissions) — which, like every other pi failure here, is warned
            // and the mirror skipped rather than failing an install whose
            // repo-local targets are already on disk. Held until the end of this
            // scope so `record_pi_provenance` runs under it (see
            // `acquire_pi_lock`).
            let _pi_lock = match acquire_pi_lock(pi_root) {
                Ok(lock) => lock,
                Err(e) => {
                    eprintln!("  ! pi.dev skill mirror skipped (lock unavailable): {e:#}");
                    return Ok(results);
                }
            };

            // Iterate the single authoritative managed-skill set so the mirror
            // write and the lifecycle layer can never disagree about which
            // skills issuectl ships (see `managed_pi_skills`). Track the names
            // this run actually *wrote* (created or overwrote) so provenance is
            // recorded only for real writes — never inferred from a file that
            // merely happens to exist on disk.
            let mut written: BTreeSet<String> = BTreeSet::new();
            for (name, template) in managed_pi_skills() {
                match install_pi_mirror(pi_root, name, template, force) {
                    Ok(r) => {
                        if matches!(
                            r.outcome,
                            InstallOutcome::Created | InstallOutcome::Overwritten
                        ) {
                            written.insert(name.to_string());
                        }
                        results.push(r);
                    }
                    Err(e) => eprintln!("  ! pi.dev skill mirror skipped for {name}: {e:#}"),
                }
            }

            // Record out-of-band provenance for the copies THIS run wrote. The
            // manifest (`<pi_root>/.issuectl-manifest.json`) is what lets the
            // lifecycle layer (`skill pi-status` / `skill pi-prune`)
            // distinguish issuectl-owned entries from hand-authored ones
            // without touching the byte-identical `SKILL.md` bodies. Like the
            // mirror writes, a manifest failure is warned and skipped so it
            // never fails an install that has already put the repo-local
            // targets on disk.
            if let Err(e) = record_pi_provenance(pi_root, &written) {
                eprintln!("  ! pi.dev skill manifest update skipped: {e:#}");
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
    print_skill_install_summary(repo_root, agents, &results);
    Ok(())
}

/// Print a human-readable summary for completed skill installation results.
pub fn print_skill_install_summary(repo_root: &Path, agents: &[Agent], results: &[InstallResult]) {
    for r in results {
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
    // Gate the hint on the pi block having mirrored the FULL managed set, not
    // on the `pi_root.is_some() && claude` preconditions (see
    // [`pi_hint_should_print`]): the block can be skipped or partial after
    // those hold, and the hint copy claims "the same skills are mirrored".
    if pi_hint_should_print(results) {
        println!(
            "  The same skills are mirrored into ~/.pi/agent/skills for pi.dev (/skill:issue)."
        );
    }
    println!("  Or use `issuectl list` to browse issues from the command line.");
}

/// Whether [`install_skill`] should print the pi.dev "skills mirrored" hint.
///
/// True only when the pi block mirrored the **full** managed set: every
/// [`managed_pi_skills`] entry left a pi-labelled [`InstallResult`] (a mirror
/// that was `Created`, `Overwritten`, or already present as `AlreadyExists`).
/// This is the accurate signal for the hint's copy — "The same skills are
/// mirrored" — which claims the whole set is present:
///
/// * a **skipped** block (lock unavailable → early return, or every write
///   warned-and-skipped) leaves zero pi results → hint off;
/// * a **partial** mirror (some per-skill writes refused, e.g. a symlink out of
///   the corpus) leaves fewer results than the managed set → hint off, and the
///   per-skill warnings already printed to stderr tell the user what was
///   skipped;
/// * a **complete** mirror leaves exactly one pi result per managed skill →
///   hint on.
///
/// Keying off [`PI_SKILL_LABEL`] (never used by the repo-local results) keeps
/// this the single chokepoint that couples the label to the hint decision.
fn pi_hint_should_print(results: &[InstallResult]) -> bool {
    let expected = managed_pi_skills().len();
    let mirrored = results.iter().filter(|r| r.label == PI_SKILL_LABEL).count();
    expected > 0 && mirrored == expected
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
        InstallOutcome::RepoAuthoredContentPreserved => {
            println!(
                "  ~ {display} skipped (repo-authored content preserved; use --force-scaffold to regenerate)"
            );
            return;
        }
    };
    if r.label.is_empty() {
        println!("  ✓ {verb} {display}");
    } else {
        println!("  ✓ {verb} {display} ({})", r.label);
    }
}

fn install_issues_scaffold(
    repo_root: &Path,
    force: bool,
    force_scaffold: bool,
) -> Result<InstallResult> {
    let issues_dir = repo_root.join("issues");
    let agents_md = issues_dir.join("AGENTS.md");

    if !issues_dir.exists() {
        std::fs::create_dir_all(&issues_dir)
            .with_context(|| format!("cannot create {}", issues_dir.display()))?;
    }

    let existing = if agents_md.exists() {
        Some(
            std::fs::read(&agents_md)
                .with_context(|| format!("cannot read {}", agents_md.display()))?,
        )
    } else {
        None
    };
    let outcome = match existing.as_deref() {
        None => {
            std::fs::write(&agents_md, ISSUES_AGENTS_TEMPLATE)
                .with_context(|| format!("cannot write {}", agents_md.display()))?;
            InstallOutcome::Created
        }
        Some(content)
            if force_scaffold || (force && content == ISSUES_AGENTS_TEMPLATE.as_bytes()) =>
        {
            std::fs::write(&agents_md, ISSUES_AGENTS_TEMPLATE)
                .with_context(|| format!("cannot write {}", agents_md.display()))?;
            InstallOutcome::Overwritten
        }
        Some(content) if force && content != ISSUES_AGENTS_TEMPLATE.as_bytes() => {
            InstallOutcome::RepoAuthoredContentPreserved
        }
        Some(_) => InstallOutcome::AlreadyExists,
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
    // Path-traversal gate: refuse to write when the entry dir or its `SKILL.md`
    // is a symlink out of the corpus, so a `--force` mirror never overwrites a
    // file outside `pi_root` (see [`ensure_pi_mirror_target_within_corpus`]).
    // Propagated `Err` is caught by the caller loop, which warns and skips the
    // one mirror — the repo-local Claude install still succeeds.
    ensure_pi_mirror_target_within_corpus(pi_root, name)?;
    install_rendered_file(
        pi_root.join(name).join("SKILL.md"),
        template,
        PI_SKILL_LABEL,
        force,
    )
}

/// Refuse to mirror into a skill-entry path that would let the write escape the
/// corpus root. The mirror target is `<pi_root>/<name>/SKILL.md`; a symlink at
/// the intermediate `<pi_root>/<name>` directory OR at the final `SKILL.md`
/// would make `std::fs::write` follow the link and overwrite a file *outside*
/// the corpus (`is_valid_skill_name` vets only the manifest key, never the
/// on-disk shape of the path it names). Both components are inspected with
/// `symlink_metadata`, which never follows the final component, so a directory
/// symlink is seen as a symlink rather than as its (external) target.
///
/// A not-yet-existing entry is fine — the install creates a real directory and
/// a real file. A pre-existing regular file where the entry dir belongs, or a
/// non-regular `SKILL.md` (symlink, FIFO, device, dir), is refused; the caller
/// warns and skips, exactly as the old `create_dir_all` failure did. Every
/// branch fails CLOSED: a stat error we cannot attribute to `NotFound`
/// propagates rather than being read as "safe".
fn ensure_pi_mirror_target_within_corpus(pi_root: &Path, name: &str) -> Result<()> {
    // Defense in depth at the filesystem boundary: reject a key that is not a
    // single safe path component before any join, even though every current
    // caller passes a hard-coded `managed_pi_skills()` name. Without this the
    // helper's own containment contract would rest entirely on its callers.
    if !is_valid_skill_name(name) {
        anyhow::bail!("invalid pi.dev skill name {name:?}; refusing to write it");
    }
    let dir = pi_root.join(name);
    match dir.symlink_metadata() {
        Ok(m) if m.file_type().is_symlink() => anyhow::bail!(
            "{} is a symlink out of the corpus; refusing to write through it",
            dir.display()
        ),
        Ok(m) if !m.is_dir() => {
            anyhow::bail!("{} exists but is not a directory", dir.display())
        }
        Ok(_) => {}                                              // real directory
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // fresh install
        Err(e) => return Err(e).with_context(|| format!("cannot stat {}", dir.display())),
    }
    let skill_md = dir.join("SKILL.md");
    match skill_md.symlink_metadata() {
        Ok(m) if m.file_type().is_symlink() => anyhow::bail!(
            "{} is a symlink out of the corpus; refusing to overwrite through it",
            skill_md.display()
        ),
        // A pre-existing `SKILL.md` must be a plain regular file. A FIFO would
        // block `std::fs::write` forever; a device/socket would receive the
        // rendered body. `--force` overwrites only a genuine regular file.
        Ok(m) if !m.is_file() => {
            anyhow::bail!("{} exists but is not a regular file", skill_md.display())
        }
        Ok(_) => {}                                              // real regular file
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // will be created
        Err(e) => return Err(e).with_context(|| format!("cannot stat {}", skill_md.display())),
    }
    Ok(())
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

// ── pi.dev corpus lifecycle (provenance · drift · prune) ────────────────────
//
// The pi mirror above writes issuectl's Claude skills into the *global*
// `~/.pi/agent/skills/` corpus, byte-identical to the repo-local Claude copy.
// Those copies are otherwise unmanaged — nothing tracks, verifies, or removes
// them (see the `pidev-pi-skill-lifecycle` issue). This section adds the
// observability layer:
//
//   * an out-of-band **provenance manifest** (`.issuectl-manifest.json` at the
//     corpus root) marking which entries issuectl wrote, so tooling can tell
//     issuectl-owned copies apart from hand-authored ones without disturbing
//     the byte-identical `SKILL.md`;
//   * **drift + orphan detection** ([`pi_status`]) comparing each on-disk copy
//     against what the running binary would write and against the set of
//     skills it still ships; and
//   * a **prune** path ([`pi_prune`]) that removes orphaned issuectl-owned
//     entries and clears stale manifest rows.
//
// Reconciliation policy (the write path, deliberately unchanged): a non-force
// install leaves an existing pi copy in place; `--force` overwrites it
// unconditionally to the running binary's version — *always-on-force*, not
// overwrite-only-if-newer. This matches the repo-local Claude/Codex targets
// exactly (force means force) and avoids a surprising "your --force did
// nothing" outcome or brittle version-ordering at write time. The cost — an
// older binary's `--force` can rewrite the global copy to an older version —
// is handled by making drift *visible* (`pi-status` flags a copy whose
// recorded version differs from the running binary) and *reversible* (re-run
// `skill install --force`, or `skill pi-prune` for orphans), rather than by
// guarding the write.
//
// SYMLINK CONTAINMENT / THREAT MODEL (see the `pi-corpus-symlink-traversal`
// issue). Every path that walks, deletes, or overwrites under `pi_root` refuses
// to follow a directory/`SKILL.md` symlink out of the corpus: the walk gate in
// [`classify_pi_corpus`] (symlinked entry → `Unmanaged`, never read through),
// the delete gate `orphan_is_safely_removable`, and the write gate
// [`ensure_pi_mirror_target_within_corpus`], all via `symlink_metadata` (which
// never follows the final component). `save_pi_manifest` creates its temp file
// with `O_EXCL` so a pre-planted symlink at the temp name is refused too. These
// gates are `symlink_metadata`-then-act (check-then-use): they fully close the
// documented threat — a symlink planted BEFORE issuectl runs (a user footgun or
// a sibling tool's leftover), plus the cross-process advisory flock that
// serializes cooperating issuectl/orchestratectl processes. They do NOT close a
// TOCTOU race against a *hostile* process that swaps a real dir for a symlink in
// the window between the check and the destructive syscall, nor a `--force`
// overwrite through a hard link; a same-UID adversary racing us on the corpus
// is out of scope here. Fully closing that needs descriptor-relative no-follow
// ops (`openat2 RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS`, `unlinkat`, atomic
// `renameat`) — tracked in `pi-corpus-fd-relative-hardening`.

/// The provenance-manifest filename at the root of the pi corpus. Namespaced
/// by tool so the sibling `orchestratectl` corpus writer keeps its own manifest
/// and neither prunes the other's entries.
pub const PI_MANIFEST_FILE: &str = ".issuectl-manifest.json";

/// Current on-disk schema version of [`PiManifest`].
const PI_MANIFEST_VERSION: u32 = 1;

/// The tool name stamped into the manifest — a guard against reading a
/// differently-owned file if the filename convention ever collides.
const PI_MANIFEST_TOOL: &str = "issuectl";

/// Acquire the cross-process advisory lock guarding the pi corpus at `pi_root`,
/// so a `skill install` mirror-write + manifest read-modify-write from one repo
/// serializes against a concurrent `skill install` / `pi-prune` from another
/// (both resolve the same global `~/.pi/agent/skills` root, but run as
/// independent processes with no other coordination). Without it, two installs
/// can each load the manifest, add their own row, and race the atomic rename —
/// the loser's row is lost even though the file itself is never torn.
///
/// Reuses the repo-wide [`crate::mutate::WriteLock`] flock helper rather than
/// hand-rolling a second locking primitive: it creates `<pi_root>/.issuectl/`
/// (a dotfile dir the corpus scanner already ignores — see
/// `is_valid_skill_name`) and takes an exclusive `flock(2)` on `write.lock`
/// there, released when the returned guard drops. `WriteLock::acquire`
/// `create_dir_all`s that dir first, so this succeeds even when `pi_root` does
/// not yet exist (the first install).
///
/// The lock is per open file description, so the guard must be held for the
/// whole read-modify-write and never re-acquired while already held —
/// [`record_pi_provenance`]/[`save_pi_manifest`] run *under* a held lock and so
/// deliberately take no lock of their own (a nested acquire would deadlock on
/// Linux, per the same convention `mutate/` follows).
fn acquire_pi_lock(pi_root: &Path) -> Result<crate::mutate::WriteLock> {
    crate::mutate::WriteLock::acquire(pi_root)
        .with_context(|| format!("cannot acquire pi corpus lock at {}", pi_root.display()))
}

/// The out-of-band provenance manifest issuectl maintains at
/// `<pi_root>/.issuectl-manifest.json`. It records which skill entries under
/// the pi corpus issuectl wrote (and at which version), so the lifecycle layer
/// can distinguish issuectl-owned entries from hand-authored ones.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PiManifest {
    /// Schema version of the manifest file itself.
    pub manifest_version: u32,
    /// Owning tool — always `PI_MANIFEST_TOOL`.
    pub tool: String,
    /// `skill name → entry`, a `BTreeMap` for a stable, diff-friendly order.
    pub skills: BTreeMap<String, PiManifestEntry>,
}

/// One row of a [`PiManifest`]: the issuectl version whose template wrote the
/// pi copy of this skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PiManifestEntry {
    /// The issuectl version pinned into the copy issuectl last wrote.
    pub version: String,
}

impl PiManifest {
    fn empty() -> Self {
        Self {
            manifest_version: PI_MANIFEST_VERSION,
            tool: PI_MANIFEST_TOOL.to_string(),
            skills: BTreeMap::new(),
        }
    }
}

/// The Claude-format skills issuectl ships and mirrors into the pi corpus, as
/// `(name, template-source)` pairs in install order. This is the authoritative
/// "known set" the lifecycle layer compares the corpus against: an entry whose
/// name is here is a *current* skill; one that is issuectl-owned (present in
/// the manifest) but NOT here is an **orphan** (renamed/removed — e.g. the
/// deprecated `/triage-bugs`).
pub fn managed_pi_skills() -> Vec<(&'static str, &'static str)> {
    let mut skills = Vec::with_capacity(1 + IntakeSkill::ALL.len());
    if let Some(name) = Agent::Claude.skill_name() {
        skills.push((name, Agent::Claude.template()));
    }
    for skill in IntakeSkill::ALL {
        skills.push((skill.slug(), skill.template(Agent::Claude)));
    }
    skills
}

/// Extract the issuectl version a skill body was rendered for, from the
/// `` This skill was installed for `issuectl X` `` marker every template
/// carries. Returns `None` when the marker is absent (e.g. a hand-authored
/// skill that never went through [`render_template`]). Display/diagnostic only
/// — provenance/ownership is tracked in the manifest at write time, never
/// inferred from this mutable content.
pub fn pinned_version(body: &str) -> Option<String> {
    let marker = "This skill was installed for `issuectl ";
    let start = body.find(marker)? + marker.len();
    let rest = &body[start..];
    let end = rest.find('`')?;
    Some(rest[..end].to_string())
}

/// Whether `name` is a safe single-component skill directory name — the only
/// shape a manifest key or corpus dir may take. Rejects anything that could
/// escape `pi_root` when joined: path separators, `.`/`..`, absolute paths,
/// and dotfiles. This is the containment gate that keeps a corrupt or tampered
/// manifest from steering a delete outside the corpus.
fn is_valid_skill_name(name: &str) -> bool {
    if name.is_empty() || name.starts_with('.') {
        return false;
    }
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(c)), None) if !c.is_empty()
    )
}

/// Strictly load the provenance manifest:
/// - `Ok(None)` when the file is absent (`NotFound`) — the normal "no manifest
///   yet" case;
/// - `Ok(Some(manifest))` when it parses, is stamped with our tool, and carries
///   a supported schema version — with any structurally-unsafe skill keys
///   dropped (`is_valid_skill_name`);
/// - `Err(..)` when the file is present but unreadable, corrupt (bad JSON),
///   owned by another tool, or an unsupported schema version.
///
/// The write/delete paths use this so they refuse to act on a manifest they
/// can't trust — silently treating a corrupt manifest as empty would drop every
/// provenance row and, for prune, act on a misread view of what issuectl owns.
fn try_load_pi_manifest(pi_root: &Path) -> Result<Option<PiManifest>> {
    let path = pi_root.join(PI_MANIFEST_FILE);
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    let mut manifest: PiManifest = serde_json::from_str(&raw)
        .with_context(|| format!("{} is corrupt (invalid JSON)", path.display()))?;
    if manifest.tool != PI_MANIFEST_TOOL {
        anyhow::bail!(
            "{} is owned by another tool ({:?}), not issuectl",
            path.display(),
            manifest.tool
        );
    }
    if manifest.manifest_version != PI_MANIFEST_VERSION {
        anyhow::bail!(
            "{} has unsupported manifest version {} (this issuectl understands {})",
            path.display(),
            manifest.manifest_version,
            PI_MANIFEST_VERSION
        );
    }
    // Never trust an unsafe key into a filesystem join.
    manifest.skills.retain(|name, _| is_valid_skill_name(name));
    Ok(Some(manifest))
}

/// Lenient load for read-only paths ([`pi_status`]): any failure — absent,
/// unreadable, corrupt, foreign, or unsupported — collapses to an empty
/// manifest so a status readout never crashes. The write/delete paths use
/// [`try_load_pi_manifest`] instead, which refuses to act on an untrusted file.
fn load_pi_manifest(pi_root: &Path) -> PiManifest {
    try_load_pi_manifest(pi_root)
        .ok()
        .flatten()
        .unwrap_or_else(PiManifest::empty)
}

/// Persist the provenance manifest atomically: serialize, write to a
/// same-directory temp file, then rename over the destination. The rename is
/// atomic on POSIX, so a concurrent reader or an interrupted write never sees a
/// torn/empty manifest (which the lenient loader would misread as "no owned
/// entries").
///
/// The temp name mixes the process id with a per-call atomic counter, so it is
/// unique across processes *and* across threads/successive calls within one
/// process. The cross-process lock ([`acquire_pi_lock`]) already serializes the
/// real write paths, but the per-thread uniqueness matters for the in-process
/// concurrency tests: without it, two threads would collide on one
/// `…<pid>.tmp` path and a lock regression would surface as a temp-file race
/// rather than the true lost update it is meant to expose.
///
/// This is not itself a substitute for the lock — an unlocked read-modify-write
/// can still lose a row (the atomic rename only guarantees the file on disk is
/// always a complete manifest, never torn). Runs under a held [`acquire_pi_lock`]
/// on every production path.
fn save_pi_manifest(pi_root: &Path, manifest: &PiManifest) -> Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

    let path = pi_root.join(PI_MANIFEST_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(manifest).context("serialize pi manifest")?;
    let tmp = pi_root.join(format!(
        "{}.{}.{}.tmp",
        PI_MANIFEST_FILE,
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    // The temp name is predictable (`<pid>.<seq>`), so a hostile process could
    // pre-create it as a symlink to a file OUTSIDE the corpus; a plain
    // `std::fs::write` would then follow the link and truncate that target with
    // manifest JSON — another way to escape `pi_root`. Create the temp file
    // exclusively via `create_new` (`O_CREAT|O_EXCL`): POSIX guarantees the open
    // fails with `EEXIST` when the final component already exists *including a
    // symlink*, regardless of its target, so a pre-created (or racing) symlink
    // at this exact name is refused rather than followed. No new dependency
    // needed — this is std-only.
    {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .with_context(|| format!("cannot exclusively create {}", tmp.display()))?;
        f.write_all(format!("{body}\n").as_bytes())
            .with_context(|| format!("cannot write {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &path).with_context(|| {
        // Best-effort cleanup of the temp file on a failed rename.
        let _ = std::fs::remove_file(&tmp);
        format!("cannot atomically replace {}", path.display())
    })?;
    Ok(())
}

/// Record provenance for the skills THIS install run actually wrote (the
/// `written` set, populated only for `Created`/`Overwritten` mirror outcomes),
/// stamping each with the running binary's version — the version we just wrote,
/// known for certain rather than parsed back out of mutable file content.
///
/// Crucially, it never *adopts* a file it did not write: a managed-name copy
/// that already existed and was left in place (a non-force install, or a
/// hand-authored file at a managed path) is claimed **only if it already had a
/// manifest row**. A previously-unowned file stays unowned — so `pi-prune` can
/// never later delete a hand-authored skill it silently appropriated. Existing
/// rows for skills this run did not rewrite are preserved untouched; orphan rows
/// are preserved too (only [`pi_prune`] removes those).
///
/// Uses the strict loader so it refuses to clobber a present-but-corrupt
/// manifest with a freshly-rebuilt one.
fn record_pi_provenance(pi_root: &Path, written: &BTreeSet<String>) -> Result<()> {
    // Nothing to record and no risk of clobber if we wrote nothing this run and
    // there is no manifest yet — but still load to preserve/validate an
    // existing one.
    let mut manifest = try_load_pi_manifest(pi_root)?.unwrap_or_else(PiManifest::empty);
    let running = env!("CARGO_PKG_VERSION");
    for (name, _template) in managed_pi_skills() {
        if written.contains(name) {
            // We (re)wrote this copy just now → stamp the running version.
            manifest.skills.insert(
                name.to_string(),
                PiManifestEntry {
                    version: running.to_string(),
                },
            );
        }
        // Skipped-but-already-owned rows are left as-is; skipped-and-unowned
        // files are deliberately NOT adopted.
    }
    // Only touch disk if we have something to persist (a fresh install always
    // writes at least the scaffold copies; a pure no-op non-force re-run over
    // an unowned corpus writes nothing).
    if written.is_empty() && !pi_root.join(PI_MANIFEST_FILE).exists() {
        return Ok(());
    }
    save_pi_manifest(pi_root, &manifest)
}

/// The lifecycle state of one entry in the pi skill corpus, from issuectl's
/// point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PiSkillState {
    /// Managed and the on-disk copy matches what the running binary writes now.
    UpToDate,
    /// Managed, on-disk copy differs, and the recorded version differs from the
    /// running binary — a different (typically older) binary wrote it. Refresh
    /// with `issuectl skill install --force`.
    Stale,
    /// Managed, on-disk copy differs, but the recorded version equals the
    /// running binary — the copy was hand-edited or corrupted after issuectl
    /// wrote it. `--force` restores it.
    Modified,
    /// In the manifest but the `SKILL.md` is genuinely gone (a `NotFound` stat).
    /// `pi-prune` clears the row.
    Missing,
    /// Manifest entry for a skill the running binary no longer ships (renamed
    /// or removed, e.g. `/triage-bugs`). `pi-prune` removes dir + row.
    Orphan,
    /// A skill dir on disk with no manifest row — hand-authored, written by
    /// another tool, or written by a pre-manifest issuectl. Reported for
    /// visibility; never touched by `pi-prune`.
    Unmanaged,
    /// An issuectl-owned entry whose `SKILL.md` (or its containing dir) could
    /// not be stat'd OR read for a reason OTHER than absence (permission denied,
    /// I/O error, ELOOP, a transient failure). Presence and content are unknown,
    /// so the entry must NOT be treated as `Missing` — that would let `pi-prune`
    /// drop the manifest row for a skill whose file may still exist, losing
    /// provenance. Reported for visibility; never pruned. (An UNOWNED entry with
    /// the same failure has no manifest row at risk and stays `Unmanaged`.)
    Inaccessible,
}

impl PiSkillState {
    /// A short human label for the terminal report.
    pub fn label(self) -> &'static str {
        match self {
            Self::UpToDate => "up-to-date",
            Self::Stale => "stale",
            Self::Modified => "modified",
            Self::Missing => "missing",
            Self::Orphan => "orphan",
            Self::Unmanaged => "unmanaged",
            Self::Inaccessible => "inaccessible",
        }
    }
}

/// The lifecycle status of a single pi-corpus skill entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PiSkillStatus {
    pub name: String,
    pub state: PiSkillState,
    /// Version recorded in the manifest, if issuectl owns this entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_version: Option<String>,
    /// Version pinned inside the on-disk `SKILL.md`, if present and parseable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_disk_version: Option<String>,
    /// The `SKILL.md` path this row describes.
    pub path: String,
}

/// A full pi-corpus status report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PiStatusReport {
    /// The running binary's version — what a `--force` refresh would pin to.
    pub version: String,
    /// The pi corpus root that was inspected.
    pub root: String,
    /// One row per skill entry, sorted by name for a stable report.
    pub skills: Vec<PiSkillStatus>,
}

impl PiStatusReport {
    /// Whether any entry is actionable (drift, orphan, a missing copy, or a
    /// copy that could not be inspected).
    pub fn has_findings(&self) -> bool {
        self.skills.iter().any(|s| {
            matches!(
                s.state,
                PiSkillState::Stale
                    | PiSkillState::Modified
                    | PiSkillState::Missing
                    | PiSkillState::Orphan
                    | PiSkillState::Inaccessible
            )
        })
    }
}

/// Directory names of the current skill entries physically present under
/// `pi_root` (each `<name>/` holding a mirrored skill). The manifest file, any
/// stray non-directory entries, and any name that is not a safe single
/// component (`is_valid_skill_name`, which also excludes dotfiles) are
/// ignored.
fn on_disk_skill_dirs(pi_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(pi_root) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_valid_skill_name(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Classify every entry in the corpus — manifest rows plus on-disk skill dirs —
/// against `manifest`, the caller-supplied snapshot. Splitting this out (rather
/// than re-reading the manifest inside [`pi_status`]) lets [`pi_prune`] act on
/// exactly the same snapshot it will mutate, so a read-then-classify race can't
/// open between the two. Read-only.
fn classify_pi_corpus(pi_root: &Path, manifest: &PiManifest) -> Vec<PiSkillStatus> {
    let managed: BTreeMap<&str, &str> = managed_pi_skills().into_iter().collect();
    let running = env!("CARGO_PKG_VERSION");

    // Union of every name we might have something to say about, name-sorted.
    let mut names: BTreeSet<String> = manifest.skills.keys().cloned().collect();
    names.extend(on_disk_skill_dirs(pi_root));

    let mut skills = Vec::with_capacity(names.len());
    for name in names {
        let recorded = manifest.skills.get(&name);
        let managed_template = managed.get(name.as_str()).copied();
        let dir = pi_root.join(&name);
        let skill_md = dir.join("SKILL.md");

        // Containment gate on the WALK path (mirrors the prune/install gates):
        // never resolve `skill_md` THROUGH a symlinked `<pi_root>/<name>`.
        // `symlink_metadata(skill_md)` does not follow the final component but
        // *does* follow an intermediate directory symlink, so a
        // `<pi_root>/x -> /external/dir` would make status read
        // `/external/dir/SKILL.md` — an out-of-corpus read (info disclosure) and
        // a misclassification. A symlinked entry dir is reported `Unmanaged`
        // (visible, but never read through and never prune-eligible — prune only
        // ever acts on `Orphan`/`Missing`), so classification and prune agree
        // that a symlinked entry is off-limits without following it.
        let dir_meta = dir.symlink_metadata();
        if dir_meta
            .as_ref()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            skills.push(PiSkillStatus {
                name,
                state: PiSkillState::Unmanaged,
                recorded_version: recorded.map(|e| e.version.clone()),
                on_disk_version: None,
                path: skill_md.to_string_lossy().into_owned(),
            });
            continue;
        }

        // Presence via `symlink_metadata` (does NOT follow symlinks). Only a
        // genuine `NotFound` counts as absent; a stat (of the entry dir OR the
        // `SKILL.md`) or a content read that fails for ANY other reason
        // (permission, I/O, ELOOP, a transient failure) leaves presence/content
        // *unknown*. Folding such an entry into `Missing` would let `pi_prune`
        // drop the manifest row for a skill whose file may still exist, losing
        // provenance — so an OWNED entry we could not fully inspect is surfaced
        // as `Inaccessible` and never pruned. The dir stat is folded in too (its
        // own symlink check above uses `unwrap_or(false)`, which would otherwise
        // let a non-`NotFound` dir-stat error fall through to a possibly-racing
        // child stat). Content is read only for a plain regular file.
        let dir_inaccessible =
            matches!(&dir_meta, Err(e) if e.kind() != std::io::ErrorKind::NotFound);
        let meta = skill_md.symlink_metadata();
        let mut inaccessible =
            dir_inaccessible || matches!(&meta, Err(e) if e.kind() != std::io::ErrorKind::NotFound);
        let present = meta.is_ok();
        let is_regular = meta.as_ref().map(|m| m.is_file()).unwrap_or(false);
        // Read raw bytes so a genuine I/O/permission failure (→ `Inaccessible`)
        // is told apart from non-UTF-8 content (real drift, compared as bytes).
        let on_disk = if is_regular {
            match std::fs::read(&skill_md) {
                Ok(bytes) => Some(bytes),
                Err(_) => {
                    inaccessible = true;
                    None
                }
            }
        } else {
            None
        };
        let on_disk_version = on_disk
            .as_deref()
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(pinned_version);

        let state = if inaccessible && recorded.is_some() {
            // An OWNED entry whose stat/read failed with something other than
            // `NotFound`: presence and content are unknown. Never fold this into
            // `Missing` (which prune clears) — report it as `Inaccessible` and
            // leave the entry alone. An UNOWNED inaccessible entry has no
            // manifest row at risk, so it falls through to `Unmanaged` below
            // (unchanged behavior).
            PiSkillState::Inaccessible
        } else if recorded.is_some() && !present {
            // Owned but the copy is truly gone (a genuine `NotFound`; checked
            // before orphan, so a retired skill whose file already vanished
            // reads as Missing).
            PiSkillState::Missing
        } else if recorded.is_some() && managed_template.is_none() {
            // issuectl-owned but no longer a shipped skill → orphan.
            PiSkillState::Orphan
        } else if let Some(template) = managed_template.filter(|_| recorded.is_some()) {
            // Managed + issuectl-owned + present: compare against what we'd write
            // now. A non-regular file (symlink/dir) or divergent content is
            // drift; the recorded version splits hand-modification from an
            // other-binary copy.
            if is_regular && on_disk.as_deref() == Some(render_template(template).as_bytes()) {
                PiSkillState::UpToDate
            } else if recorded.map(|e| e.version.as_str()) == Some(running) {
                PiSkillState::Modified
            } else {
                PiSkillState::Stale
            }
        } else {
            // On disk (or a stray dir) but not recorded as issuectl-owned:
            // hand-authored, another tool, or a pre-manifest install. Never our
            // business to prune.
            PiSkillState::Unmanaged
        };

        skills.push(PiSkillStatus {
            name,
            state,
            recorded_version: recorded.map(|e| e.version.clone()),
            on_disk_version,
            path: skill_md.to_string_lossy().into_owned(),
        });
    }
    skills
}

/// Inspect the pi skill corpus rooted at `pi_root` and classify every entry
/// into a [`PiSkillState`]. Read-only: never writes. Uses the lenient manifest
/// loader so a status readout never fails on a corrupt manifest (it just shows
/// entries as `unmanaged`); the mutating [`pi_prune`] is stricter.
pub fn pi_status(pi_root: &Path) -> Result<PiStatusReport> {
    let manifest = load_pi_manifest(pi_root);
    Ok(PiStatusReport {
        version: env!("CARGO_PKG_VERSION").to_string(),
        root: pi_root.to_string_lossy().into_owned(),
        skills: classify_pi_corpus(pi_root, &manifest),
    })
}

/// What a [`pi_prune`] pass did (or, in dry-run, would do) to one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PiPruneKind {
    /// An orphaned issuectl-owned skill dir (skill no longer shipped): its
    /// `SKILL.md` is removed, the now-empty dir dropped, and the manifest row
    /// cleared.
    Orphan,
    /// A manifest row whose `SKILL.md` is already gone: the row is cleared.
    Missing,
}

/// One entry a prune pass removed (or would remove in dry-run).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PiPruneItem {
    pub name: String,
    pub kind: PiPruneKind,
    /// The `SKILL.md` path removed (orphan) or the missing path whose row was
    /// cleared.
    pub path: String,
}

/// The outcome of a [`pi_prune`] pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PiPruneOutcome {
    /// `false` for a dry run OR an apply that changed nothing; `true` only when
    /// changes actually landed on disk.
    pub applied: bool,
    /// Entries removed, or — in dry-run — that would be removed.
    pub removed: Vec<PiPruneItem>,
    /// Orphan entries deliberately left alone for safety: a symlinked or
    /// non-regular `SKILL.md`, a dir that also holds sibling files a user added,
    /// or an entry whose file could not be removed. The user must resolve these
    /// by hand — prune never force-deletes them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<PiPruneItem>,
}

/// Whether an orphan entry is safe to auto-remove. It is safe only when the
/// `SKILL.md` (if it exists) is a plain **regular file** — never a symlink
/// (whose target could be anywhere) or a directory — **and** the entry dir
/// contains nothing but that `SKILL.md`. Claude's skill format permits bundled
/// reference files, so a dir with siblings is left for the user rather than
/// having its `SKILL.md` silently torn out.
fn orphan_is_safely_removable(dir: &Path, skill_md: &Path) -> bool {
    // Path-traversal gate (checked FIRST, before the `skill_md`/`read_dir`
    // inspection below): the entry dir must be a REAL directory physically
    // under the corpus, never a symlink. `is_valid_skill_name` proves only that
    // the manifest *key* is a single safe component — it says nothing about the
    // on-disk shape of `<pi_root>/<name>`. If that dir has been replaced by a
    // symlink (`triage-bugs -> /external/dir`), the `skill_md.symlink_metadata`
    // and `read_dir(dir)` below — and the eventual `remove_file(skill_md)` in
    // `pi_prune` — all resolve THROUGH the link to an arbitrary target outside
    // the corpus, turning prune into an arbitrary-delete. `symlink_metadata`
    // does not follow the final component, so a directory symlink is reported
    // as a symlink here and refused (reported in `skipped`, never deleted).
    match dir.symlink_metadata() {
        Ok(m) if m.file_type().is_symlink() => return false, // dir symlink → never follow
        Ok(m) if !m.is_dir() => return false,                // not a real dir → refuse
        Ok(_) => {}                                          // genuine directory → inspect
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true, // no dir → row-only drop is safe
        Err(_) => return false, // unreadable → refuse to act blind
    }
    match skill_md.symlink_metadata() {
        Ok(m) if !m.is_file() => return false, // symlink or dir at SKILL.md → refuse
        Ok(_) => {}
        // Fail CLOSED on a stat error we can't attribute to absence: only a
        // genuine `NotFound` means "no SKILL.md, the stray dir/row is safe to
        // drop". EACCES/EIO/ELOOP etc. leave the type unknown, so refuse rather
        // than delete a file we could not inspect.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return false,
    }
    match std::fs::read_dir(dir) {
        // A sibling-file check gates the delete, so it must fail CLOSED: an
        // unreadable entry (`Err`) is NOT proof the dir holds only `SKILL.md`.
        // `flatten()` would silently drop such an `Err` and could let prune tear
        // `SKILL.md` out of a dir that actually has siblings — treat any `Err`
        // entry as "not only SKILL.md" and refuse.
        Ok(entries) => entries
            .map(|e| e.map(|e| e.file_name()))
            .all(|e| matches!(e, Ok(n) if n.to_str() == Some("SKILL.md"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true, // no dir at all
        Err(_) => false, // can't inspect the dir → refuse to delete blindly
    }
}

/// Prune the pi corpus: remove orphaned issuectl-owned entries (skills the
/// running binary no longer ships) and clear manifest rows whose copy is gone.
/// Dry-run when `apply` is false (reports what it *would* do, touches nothing).
///
/// Safety properties (this deletes files under the user's `$HOME`):
/// - **Only issuectl-owned entries** are ever touched. `Unmanaged` dirs
///   (hand-authored or another tool's) are left strictly alone, and current
///   skills (`UpToDate`/`Stale`/`Modified`) are refreshed via `skill install
///   --force`, never deleted.
/// - It **refuses to act on an untrusted manifest** (corrupt, foreign, or an
///   unsupported version) via the strict loader — acting on the empty view a
///   lenient load would produce could drop provenance or misjudge ownership.
/// - Manifest keys are validated to safe single path components at load time
///   (`is_valid_skill_name`), so a tampered key like `../../x` can never steer
///   a delete outside the corpus.
/// - An orphan removal drops **only** a regular-file `SKILL.md` and then the dir
///   *if it is now empty*; a symlinked/odd `SKILL.md` or a dir with sibling
///   files is reported in `skipped`, not deleted (see
///   `orphan_is_safely_removable`).
pub fn pi_prune(pi_root: &Path, apply: bool) -> Result<PiPruneOutcome> {
    // Hold the corpus lock across the whole load → classify → delete → save so a
    // concurrent `skill install` (or a second prune) from another repo cannot
    // interleave its manifest read-modify-write with ours and lose a row. Unlike
    // the best-effort install path, prune is an explicit user command, so a lock
    // we cannot acquire is a hard error rather than a silent skip.
    let _pi_lock = acquire_pi_lock(pi_root)?;

    // Deletion gate: refuse to act on a manifest we cannot fully trust. An
    // absent manifest means nothing is owned, so nothing to prune.
    let mut manifest = match try_load_pi_manifest(pi_root)? {
        Some(m) => m,
        None => {
            return Ok(PiPruneOutcome {
                applied: false,
                removed: Vec::new(),
                skipped: Vec::new(),
            })
        }
    };

    // Classify against the same snapshot we will mutate (no intervening reload).
    let statuses = classify_pi_corpus(pi_root, &manifest);
    let mut removed = Vec::new();
    let mut skipped = Vec::new();
    let mut dirty = false;

    for entry in &statuses {
        match entry.state {
            PiSkillState::Orphan => {
                let dir = pi_root.join(&entry.name);
                let skill_md = dir.join("SKILL.md");
                let item = PiPruneItem {
                    name: entry.name.clone(),
                    kind: PiPruneKind::Orphan,
                    path: skill_md.to_string_lossy().into_owned(),
                };
                if !orphan_is_safely_removable(&dir, &skill_md) {
                    skipped.push(item);
                    continue;
                }
                if apply {
                    // Remove the file. A genuine `NotFound` is fine (already
                    // gone); any OTHER error is a hard failure — leave the row so
                    // the manifest keeps reflecting reality, and report it as
                    // skipped. (No preceding `exists()` probe: that both follows
                    // symlinks and races the unlink; matching the `remove_file`
                    // result directly is the fail-closed form.)
                    match std::fs::remove_file(&skill_md) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) => {
                            skipped.push(item);
                            continue;
                        }
                    }
                    // Drop the dir only if now empty — never recursively. Do NOT
                    // clear the row (or report the entry removed) if this hard-
                    // fails: a non-empty/permission/raced dir means the on-disk
                    // state does not match "fully pruned", so keep the row (the
                    // now-fileless dir reclassifies as `Missing` next pass and is
                    // cleared then). `NotFound` is fine — nothing left to drop.
                    match std::fs::remove_dir(&dir) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) => {
                            skipped.push(item);
                            continue;
                        }
                    }
                    manifest.skills.remove(&entry.name);
                    dirty = true;
                }
                removed.push(item);
            }
            PiSkillState::Missing => {
                if apply {
                    manifest.skills.remove(&entry.name);
                    dirty = true;
                }
                removed.push(PiPruneItem {
                    name: entry.name.clone(),
                    kind: PiPruneKind::Missing,
                    path: entry.path.clone(),
                });
            }
            _ => {}
        }
    }

    if apply && dirty {
        save_pi_manifest(pi_root, &manifest)?;
    }
    Ok(PiPruneOutcome {
        applied: apply && dirty,
        removed,
        skipped,
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
    fn force_preserves_diverged_issues_scaffold() {
        let tmp = tempfile::tempdir().unwrap();
        let scaffold = tmp.path().join("issues/AGENTS.md");
        std::fs::create_dir_all(scaffold.parent().unwrap()).unwrap();
        std::fs::write(&scaffold, "# Repo policy\n").unwrap();

        let results = install_skill_summary(tmp.path(), &[Agent::Claude], true, None).unwrap();

        assert_eq!(
            std::fs::read_to_string(&scaffold).unwrap(),
            "# Repo policy\n"
        );
        assert_eq!(
            results[0].outcome,
            InstallOutcome::RepoAuthoredContentPreserved
        );
        assert_eq!(
            serde_json::to_value(&results).unwrap()[0]["outcome"],
            "repo_authored_content_preserved"
        );
    }

    #[test]
    fn force_refreshes_identical_issues_scaffold() {
        let tmp = tempfile::tempdir().unwrap();
        let scaffold = tmp.path().join("issues/AGENTS.md");
        std::fs::create_dir_all(scaffold.parent().unwrap()).unwrap();
        std::fs::write(&scaffold, ISSUES_AGENTS_TEMPLATE).unwrap();

        let results = install_skill_summary(tmp.path(), &[Agent::Claude], true, None).unwrap();

        assert_eq!(
            std::fs::read(&scaffold).unwrap(),
            ISSUES_AGENTS_TEMPLATE.as_bytes()
        );
        assert_eq!(results[0].outcome, InstallOutcome::Overwritten);
    }

    #[test]
    fn install_creates_missing_issues_scaffold() {
        let tmp = tempfile::tempdir().unwrap();

        let results = install_skill_summary(tmp.path(), &[Agent::Claude], true, None).unwrap();

        assert_eq!(
            std::fs::read(tmp.path().join("issues/AGENTS.md")).unwrap(),
            ISSUES_AGENTS_TEMPLATE.as_bytes()
        );
        assert_eq!(results[0].outcome, InstallOutcome::Created);
    }

    #[test]
    fn force_scaffold_regenerates_diverged_issues_scaffold() {
        let tmp = tempfile::tempdir().unwrap();
        let scaffold = tmp.path().join("issues/AGENTS.md");
        std::fs::create_dir_all(scaffold.parent().unwrap()).unwrap();
        std::fs::write(&scaffold, "# Repo policy\n").unwrap();

        let results = install_skill_summary_with_scaffold_force(
            tmp.path(),
            &[Agent::Claude],
            false,
            true,
            None,
        )
        .unwrap();

        assert_eq!(
            std::fs::read(&scaffold).unwrap(),
            ISSUES_AGENTS_TEMPLATE.as_bytes()
        );
        assert_eq!(results[0].outcome, InstallOutcome::Overwritten);
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

    // ── pi.dev "skills mirrored" hint gate ──────────────────────────────────

    /// Directly exercises the hint predicate — the branch `install_skill`
    /// actually flips on. Guards against a regression that re-gates the print on
    /// the `pi_root.is_some() && claude` preconditions (which the summary-level
    /// tests below would NOT catch, since the summary is unchanged).
    #[test]
    fn pi_hint_predicate_requires_the_full_managed_set() {
        let pi_result = |name: &str| InstallResult {
            path: PathBuf::from(name),
            label: PI_SKILL_LABEL.to_string(),
            outcome: InstallOutcome::Created,
        };
        let expected = managed_pi_skills().len();
        assert!(expected > 0, "the managed pi set must be non-empty");

        // Complete: one pi result per managed skill → hint on.
        let full: Vec<InstallResult> = managed_pi_skills()
            .iter()
            .map(|(name, _)| pi_result(name))
            .collect();
        assert!(pi_hint_should_print(&full));

        // Skipped: no pi results at all → hint off.
        assert!(!pi_hint_should_print(&[]));

        // Partial: one short of the full set → hint off (the copy would
        // over-claim "the same skills are mirrored").
        let partial = &full[..expected - 1];
        assert!(!pi_hint_should_print(partial));

        // A repo-local result never carries the pi label, so it never counts.
        let repo_local = InstallResult {
            path: PathBuf::from(".claude/skills/issue/SKILL.md"),
            label: Agent::Claude.label().to_string(),
            outcome: InstallOutcome::Created,
        };
        assert!(!pi_hint_should_print(std::slice::from_ref(&repo_local)));
    }

    /// A normal Claude+pi install mirrors every managed skill, so the summary
    /// carries one pi-labelled result per skill and the hint predicate is true.
    #[test]
    fn install_summary_signals_hint_when_full_mirror_runs() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        let results =
            install_skill_summary(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        assert_eq!(
            results.iter().filter(|r| r.label == PI_SKILL_LABEL).count(),
            managed_pi_skills().len(),
            "a full pi mirror must leave one pi-labelled result per managed skill"
        );
        assert!(pi_hint_should_print(&results), "hint must fire");
    }

    /// Regression for `pi-mirror-hint-accuracy`: when the whole pi block is
    /// skipped after the preconditions hold — here every mirror write is
    /// refused because each entry dir is a symlink out of the corpus — the
    /// summary carries NO pi-labelled result, so the hint predicate is false.
    /// The repo-local Claude install still succeeds and nothing escapes the
    /// corpus into the symlinked external dirs.
    #[cfg(unix)]
    #[test]
    fn install_summary_omits_hint_when_block_skipped() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let managed = managed_pi_skills();
        assert!(!managed.is_empty(), "test requires a managed pi skill");
        // Block every managed pi mirror: point each entry dir at an external
        // target so `ensure_pi_mirror_target_within_corpus` refuses the write
        // and the caller warns-and-skips it, leaving no pi result.
        for (name, _) in &managed {
            let external = outside.path().join(name);
            std::fs::create_dir_all(&external).unwrap();
            std::os::unix::fs::symlink(&external, pi.path().join(name)).unwrap();
        }

        let results =
            install_skill_summary(repo.path(), &[Agent::Claude], true, Some(pi.path())).unwrap();
        assert!(
            !pi_hint_should_print(&results),
            "a fully skipped pi block must leave the hint off"
        );
        assert!(
            repo.path().join(".claude/skills/issue/SKILL.md").exists(),
            "the repo-local Claude install must still succeed"
        );
        // The refusal must be a no-op on the external targets — nothing written
        // through the symlink into the corpus-escape dirs.
        for (name, _) in &managed {
            assert!(
                std::fs::read_dir(outside.path().join(name))
                    .unwrap()
                    .next()
                    .is_none(),
                "{name}: no file may be written through the corpus-escape symlink"
            );
        }
    }

    /// Partial mirror: one managed skill is blocked (symlink out of the corpus)
    /// while the rest mirror cleanly. The summary then carries fewer pi results
    /// than the managed set, so the hint stays off — otherwise the "the same
    /// skills are mirrored" copy would over-claim while a stderr warning names
    /// the skipped one.
    #[cfg(unix)]
    #[test]
    fn install_summary_omits_hint_on_partial_mirror() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let managed = managed_pi_skills();
        assert!(
            managed.len() >= 2,
            "test needs a skill to block and one to keep"
        );
        // Block exactly one managed skill; leave the others writable.
        let (blocked, _) = managed[0];
        let external = outside.path().join(blocked);
        std::fs::create_dir_all(&external).unwrap();
        std::os::unix::fs::symlink(&external, pi.path().join(blocked)).unwrap();

        let results =
            install_skill_summary(repo.path(), &[Agent::Claude], true, Some(pi.path())).unwrap();
        let mirrored = results.iter().filter(|r| r.label == PI_SKILL_LABEL).count();
        assert_eq!(
            mirrored,
            managed.len() - 1,
            "exactly the unblocked skills should mirror"
        );
        assert!(
            !pi_hint_should_print(&results),
            "a partial mirror must leave the hint off"
        );
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

        // Only three per-skill *dirs*, each holding exactly one SKILL.md. The
        // corpus root also carries the out-of-band provenance manifest (a file,
        // not a mirrored skill) and the `.issuectl` advisory-lock dir — both
        // dotfiles the corpus scanner ignores. Filter to genuine skill dirs
        // exactly as production does (`is_valid_skill_name`, which excludes
        // dotfiles) before comparing.
        let mut names: Vec<String> = std::fs::read_dir(pi.path())
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| e.file_type().unwrap().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| is_valid_skill_name(name))
            .collect();
        names.sort();
        assert_eq!(names, ["issue", "issue-intake", "issue-new"]);
        // The manifest is present at the root but is never a skill dir.
        assert!(pi.path().join(PI_MANIFEST_FILE).is_file());
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

    // ── pi.dev corpus lifecycle ─────────────────────────────────────────────

    /// Helper: find the status row for `name`, or panic.
    fn row<'a>(report: &'a PiStatusReport, name: &str) -> &'a PiSkillStatus {
        report
            .skills
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no status row for {name}"))
    }

    /// `pinned_version` extracts the embedded version from a rendered body and
    /// returns `None` for a body without the marker.
    #[test]
    fn pinned_version_reads_the_install_marker() {
        let body = render_template(Agent::Claude.template());
        assert_eq!(
            pinned_version(&body).as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(pinned_version("no marker here"), None);
    }

    /// The managed set is exactly the three Claude skills issuectl mirrors.
    #[test]
    fn managed_pi_skills_lists_the_shipped_claude_skills() {
        let names: Vec<&str> = managed_pi_skills().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["issue", "issue-new", "issue-intake"]);
    }

    #[test]
    fn skill_catalog_lists_every_shipped_skill_and_install_target() {
        let root = Path::new("");
        let catalog = skill_catalog();
        let mut expected_names = vec![Agent::Claude.skill_name().unwrap().to_string()];
        expected_names.extend(
            IntakeSkill::ALL
                .iter()
                .map(|skill| skill.slug().to_string()),
        );
        assert_eq!(
            catalog.iter().map(|skill| &skill.name).collect::<Vec<_>>(),
            expected_names.iter().collect::<Vec<_>>()
        );

        for (entry, intake_skill) in catalog.iter().skip(1).zip(IntakeSkill::ALL) {
            assert!(!entry.description.is_empty());
            for (target, agent) in entry
                .install_targets
                .iter()
                .zip([Agent::Claude, Agent::Codex])
            {
                assert_eq!(target.agent, agent.argument());
                assert_eq!(target.label, intake_skill.label(agent));
                assert_eq!(
                    target.path,
                    intake_skill.install_path(agent, root).display().to_string()
                );
            }
        }

        let issue = &catalog[0];
        for (target, agent) in issue
            .install_targets
            .iter()
            .zip([Agent::Claude, Agent::Codex])
        {
            assert_eq!(target.agent, agent.argument());
            assert_eq!(target.label, agent.label());
            assert_eq!(target.path, agent.install_path(root).display().to_string());
        }
    }

    /// A fresh Claude install writes a provenance manifest that records every
    /// mirrored skill at the running binary's version, and stamps the owning
    /// tool + schema version.
    #[test]
    fn install_writes_provenance_manifest() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();

        let manifest = load_pi_manifest(pi.path());
        assert_eq!(manifest.tool, PI_MANIFEST_TOOL);
        assert_eq!(manifest.manifest_version, PI_MANIFEST_VERSION);
        let mut names: Vec<&String> = manifest.skills.keys().collect();
        names.sort();
        assert_eq!(names, ["issue", "issue-intake", "issue-new"]);
        for entry in manifest.skills.values() {
            assert_eq!(entry.version, env!("CARGO_PKG_VERSION"));
        }
    }

    /// A Codex-only install writes neither mirror nor manifest.
    #[test]
    fn codex_only_install_writes_no_manifest() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Codex], false, Some(pi.path())).unwrap();
        assert!(!pi.path().join(PI_MANIFEST_FILE).exists());
    }

    /// After a clean install every mirrored skill reports `UpToDate`.
    #[test]
    fn pi_status_reports_up_to_date_after_install() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();

        let report = pi_status(pi.path()).unwrap();
        assert_eq!(report.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(report.skills.len(), 3);
        for s in &report.skills {
            assert_eq!(
                s.state,
                PiSkillState::UpToDate,
                "{} should be up to date",
                s.name
            );
        }
        assert!(!report.has_findings());
    }

    /// A copy an older binary wrote (recorded version differs, content differs)
    /// is `Stale`; prune leaves it alone (it is a current skill, refreshed via
    /// `--force`, not deleted).
    #[test]
    fn pi_status_flags_stale_older_copy() {
        let pi = tempfile::tempdir().unwrap();
        // Simulate an older binary's mirror: a divergent body pinned to 0.0.1.
        let issue = pi.path().join("issue/SKILL.md");
        std::fs::create_dir_all(issue.parent().unwrap()).unwrap();
        std::fs::write(
            &issue,
            "old body\nThis skill was installed for `issuectl 0.0.1`.\n",
        )
        .unwrap();
        let mut manifest = PiManifest::empty();
        manifest.skills.insert(
            "issue".into(),
            PiManifestEntry {
                version: "0.0.1".into(),
            },
        );
        save_pi_manifest(pi.path(), &manifest).unwrap();

        let report = pi_status(pi.path()).unwrap();
        let issue_row = row(&report, "issue");
        assert_eq!(issue_row.state, PiSkillState::Stale);
        assert_eq!(issue_row.recorded_version.as_deref(), Some("0.0.1"));
        assert_eq!(issue_row.on_disk_version.as_deref(), Some("0.0.1"));
        assert!(report.has_findings());

        // Prune must not touch a current skill, however stale.
        let outcome = pi_prune(pi.path(), true).unwrap();
        assert!(outcome.removed.is_empty());
        assert!(issue.exists(), "a stale current skill must never be pruned");
    }

    /// A copy the current binary wrote but that was then hand-edited (recorded
    /// version == running, content differs) is `Modified`.
    #[test]
    fn pi_status_flags_hand_modified_copy() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        // Corrupt the on-disk copy but keep the manifest at the running version.
        std::fs::write(pi.path().join("issue/SKILL.md"), "tampered").unwrap();

        let report = pi_status(pi.path()).unwrap();
        assert_eq!(row(&report, "issue").state, PiSkillState::Modified);
    }

    /// An issuectl-owned entry for a skill the binary no longer ships is an
    /// `Orphan`; prune removes its SKILL.md, drops the empty dir, and clears
    /// the manifest row — dry-run first, then applied.
    #[test]
    fn pi_prune_removes_orphans() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();

        // Plant an orphan: a manifest row + on-disk copy for a retired skill.
        let orphan = pi.path().join("triage-bugs/SKILL.md");
        std::fs::create_dir_all(orphan.parent().unwrap()).unwrap();
        std::fs::write(&orphan, "retired skill body").unwrap();
        let mut manifest = load_pi_manifest(pi.path());
        manifest.skills.insert(
            "triage-bugs".into(),
            PiManifestEntry {
                version: "0.9.0".into(),
            },
        );
        save_pi_manifest(pi.path(), &manifest).unwrap();

        // It classifies as an orphan.
        let report = pi_status(pi.path()).unwrap();
        assert_eq!(row(&report, "triage-bugs").state, PiSkillState::Orphan);

        // Dry run reports it but changes nothing.
        let dry = pi_prune(pi.path(), false).unwrap();
        assert!(!dry.applied);
        assert_eq!(dry.removed.len(), 1);
        assert_eq!(dry.removed[0].name, "triage-bugs");
        assert!(orphan.exists(), "dry run must not delete anything");
        assert!(load_pi_manifest(pi.path())
            .skills
            .contains_key("triage-bugs"));

        // Applied: the copy, the now-empty dir, and the manifest row all go.
        let applied = pi_prune(pi.path(), true).unwrap();
        assert!(applied.applied);
        assert_eq!(applied.removed.len(), 1);
        assert!(!orphan.exists());
        assert!(
            !pi.path().join("triage-bugs").exists(),
            "empty orphan dir removed"
        );
        assert!(!load_pi_manifest(pi.path())
            .skills
            .contains_key("triage-bugs"));
        // The current skills are untouched.
        assert!(pi.path().join("issue/SKILL.md").exists());
    }

    /// Prune never deletes an unmanaged (hand-authored) entry, even one whose
    /// name is not a shipped skill — only manifest-owned orphans are removed.
    #[test]
    fn pi_prune_leaves_unmanaged_entries_alone() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();

        // Hand-authored skill dir: on disk, NOT in the manifest.
        let hand = pi.path().join("my-own-skill/SKILL.md");
        std::fs::create_dir_all(hand.parent().unwrap()).unwrap();
        std::fs::write(&hand, "hand-authored").unwrap();

        let report = pi_status(pi.path()).unwrap();
        assert_eq!(row(&report, "my-own-skill").state, PiSkillState::Unmanaged);

        let outcome = pi_prune(pi.path(), true).unwrap();
        assert!(outcome.removed.is_empty());
        assert!(hand.exists(), "an unmanaged entry must never be pruned");
    }

    /// A manifest row whose copy has vanished is `Missing`; prune clears the
    /// row (there is no file to remove).
    #[test]
    fn pi_prune_clears_missing_rows() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        // Delete the on-disk copy but leave the manifest row.
        std::fs::remove_file(pi.path().join("issue/SKILL.md")).unwrap();

        let report = pi_status(pi.path()).unwrap();
        assert_eq!(row(&report, "issue").state, PiSkillState::Missing);

        let outcome = pi_prune(pi.path(), true).unwrap();
        assert_eq!(outcome.removed.len(), 1);
        assert_eq!(outcome.removed[0].kind, PiPruneKind::Missing);
        assert!(!load_pi_manifest(pi.path()).skills.contains_key("issue"));
    }

    /// REGRESSION (`pi-corpus-metadata-error-misclass`): an owned entry whose
    /// `SKILL.md` stat fails for a reason OTHER than `NotFound` must NOT be
    /// misclassified as `Missing` and pruned — its provenance row must survive.
    /// Here the manifest owns `issue` but `<pi>/issue` is a regular file rather
    /// than a directory, so stat of `<pi>/issue/SKILL.md` fails with `ENOTDIR`
    /// (a non-`NotFound` error). The corpus layout is anomalous — issuectl did
    /// not create this shape — so presence of a valid entry cannot be confirmed;
    /// we refuse to prune and surface it for the user to reconcile. `#[cfg(unix)]`
    /// because the `ENOTDIR` error-kind mapping is POSIX-specific.
    #[cfg(unix)]
    #[test]
    fn inaccessible_entry_is_not_pruned_as_missing() {
        let pi = tempfile::tempdir().unwrap();
        // `<pi>/issue` is a FILE, so any stat of `<pi>/issue/SKILL.md` errors
        // with ENOTDIR rather than ENOENT — a non-NotFound stat error.
        std::fs::write(pi.path().join("issue"), "not a directory").unwrap();
        let mut manifest = PiManifest::empty();
        manifest.skills.insert(
            "issue".into(),
            PiManifestEntry {
                version: env!("CARGO_PKG_VERSION").into(),
            },
        );
        save_pi_manifest(pi.path(), &manifest).unwrap();

        // A non-`NotFound` stat error classifies as `Inaccessible`, never
        // `Missing`, and is surfaced as an actionable finding.
        let report = pi_status(pi.path()).unwrap();
        assert_eq!(
            row(&report, "issue").state,
            PiSkillState::Inaccessible,
            "a non-NotFound metadata error must not read as Missing"
        );
        assert!(report.has_findings());

        // Prune must be a complete no-op on the entry: neither removed NOR
        // merely skipped — `Inaccessible` is invisible to prune, so the row
        // (and provenance) survives untouched.
        let outcome = pi_prune(pi.path(), true).unwrap();
        assert!(outcome.removed.is_empty(), "nothing may be removed");
        assert!(outcome.skipped.is_empty(), "nothing may even be skipped");
        assert!(
            load_pi_manifest(pi.path()).skills.contains_key("issue"),
            "the manifest row must survive a metadata-read error"
        );
    }

    /// Companion regression: an owned entry whose `SKILL.md` stats fine as a
    /// regular file but whose CONTENT cannot be read (permission denied) must
    /// also classify `Inaccessible`, not fabricated drift (`Modified`/`Stale`),
    /// and must survive prune. Exercises the read-failure → `Inaccessible` path.
    /// `#[cfg(unix)]` + a runtime probe: mode bits are honored only for a
    /// non-root owner (root — or an unusual filesystem — bypasses permission
    /// checks, so the read would succeed); the probe skips the assertions in
    /// that case rather than depending on a uid syscall crate.
    #[cfg(unix)]
    #[test]
    fn unreadable_content_classifies_inaccessible_not_drift() {
        use std::os::unix::fs::PermissionsExt;
        let pi = tempfile::tempdir().unwrap();
        let dir = pi.path().join("issue");
        std::fs::create_dir_all(&dir).unwrap();
        let skill_md = dir.join("SKILL.md");
        std::fs::write(&skill_md, "some body").unwrap();
        // Write-only (0o200): stat succeeds (regular file) but read → EACCES.
        std::fs::set_permissions(&skill_md, std::fs::Permissions::from_mode(0o200)).unwrap();
        // If the read still succeeds (running as root, or a filesystem that
        // ignores mode bits), the scenario cannot be constructed here — skip.
        if std::fs::read(&skill_md).is_ok() {
            return;
        }
        let mut manifest = PiManifest::empty();
        manifest.skills.insert(
            "issue".into(),
            PiManifestEntry {
                version: env!("CARGO_PKG_VERSION").into(),
            },
        );
        save_pi_manifest(pi.path(), &manifest).unwrap();

        let report = pi_status(pi.path()).unwrap();
        assert_eq!(
            row(&report, "issue").state,
            PiSkillState::Inaccessible,
            "an unreadable regular file must classify Inaccessible, not drift"
        );

        let outcome = pi_prune(pi.path(), true).unwrap();
        assert!(outcome.removed.is_empty() && outcome.skipped.is_empty());
        assert!(load_pi_manifest(pi.path()).skills.contains_key("issue"));

        // Restore permissions so the tempdir cleanup can remove the file.
        std::fs::set_permissions(&skill_md, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    /// A re-install after an orphan prune records provenance truthfully from
    /// the on-disk copies, and a subsequent `--force` refreshes a stale copy
    /// back to `UpToDate` (the documented reconciliation path).
    #[test]
    fn force_reinstall_refreshes_stale_to_up_to_date() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        // Older-binary copy: divergent body + old manifest version.
        let issue = pi.path().join("issue/SKILL.md");
        std::fs::create_dir_all(issue.parent().unwrap()).unwrap();
        std::fs::write(
            &issue,
            "old\nThis skill was installed for `issuectl 0.0.1`.\n",
        )
        .unwrap();
        let mut manifest = PiManifest::empty();
        manifest.skills.insert(
            "issue".into(),
            PiManifestEntry {
                version: "0.0.1".into(),
            },
        );
        save_pi_manifest(pi.path(), &manifest).unwrap();
        assert_eq!(
            row(&pi_status(pi.path()).unwrap(), "issue").state,
            PiSkillState::Stale
        );

        // A --force install refreshes the copy and re-stamps the manifest.
        install_skill(repo.path(), &[Agent::Claude], true, Some(pi.path())).unwrap();
        assert_eq!(
            row(&pi_status(pi.path()).unwrap(), "issue").state,
            PiSkillState::UpToDate
        );
    }

    /// `pi_status` on an empty / nonexistent corpus is an empty, finding-free
    /// report rather than an error.
    #[test]
    fn pi_status_empty_corpus_is_ok() {
        let pi = tempfile::tempdir().unwrap();
        let report = pi_status(pi.path()).unwrap();
        assert!(report.skills.is_empty());
        assert!(!report.has_findings());
    }

    /// A corrupt or foreign-tool manifest is ignored (treated as empty), so a
    /// stray file can't crash status/prune.
    #[test]
    fn foreign_manifest_is_ignored() {
        let pi = tempfile::tempdir().unwrap();
        std::fs::write(pi.path().join(PI_MANIFEST_FILE), "{not json").unwrap();
        assert!(load_pi_manifest(pi.path()).skills.is_empty());
        std::fs::write(
            pi.path().join(PI_MANIFEST_FILE),
            r#"{"manifest_version":1,"tool":"orchestratectl","skills":{"x":{"version":"1"}}}"#,
        )
        .unwrap();
        assert!(
            load_pi_manifest(pi.path()).skills.is_empty(),
            "another tool's manifest must not be read as ours"
        );
    }

    /// Provenance records the running version for skills THIS run wrote, and
    /// never adopts a pre-existing file it did not write. A hand-authored file
    /// at a managed path (`issue`) that a non-force install leaves in place must
    /// NOT gain a manifest row — otherwise a later prune could delete it.
    #[test]
    fn install_never_adopts_a_preexisting_unowned_file() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        // Hand-authored `issue` copy already in the corpus, with an old marker.
        let issue = pi.path().join("issue/SKILL.md");
        std::fs::create_dir_all(issue.parent().unwrap()).unwrap();
        std::fs::write(
            &issue,
            "hand body\nThis skill was installed for `issuectl 0.0.1`.\n",
        )
        .unwrap();

        // Non-force install: the mirror leaves `issue` in place but freshly
        // writes the two intake skills.
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();

        let manifest = load_pi_manifest(pi.path());
        assert!(
            !manifest.skills.contains_key("issue"),
            "a pre-existing unowned file must NOT be adopted into the manifest"
        );
        assert!(
            manifest.skills.contains_key("issue-new")
                && manifest.skills.contains_key("issue-intake"),
            "freshly-written skills are recorded"
        );
        // Status shows the un-adopted file as unmanaged (never prune-eligible).
        assert_eq!(
            row(&pi_status(pi.path()).unwrap(), "issue").state,
            PiSkillState::Unmanaged
        );
        let outcome = pi_prune(pi.path(), true).unwrap();
        assert!(outcome.removed.iter().all(|i| i.name != "issue"));
        assert!(issue.exists(), "the hand-authored file must survive prune");
    }

    /// A tampered/corrupt manifest key that would escape the corpus
    /// (`../../outside`, absolute paths, `.`/`..`) is dropped at load time and
    /// never reaches a filesystem join, so prune can't delete outside pi_root.
    #[test]
    fn manifest_path_traversal_keys_are_rejected() {
        for bad in ["../../outside", "/etc/evil", "..", ".", "a/b", ".hidden"] {
            assert!(!is_valid_skill_name(bad), "{bad:?} must be rejected");
        }
        for ok in ["issue", "issue-new", "triage-bugs", "a_b-c"] {
            assert!(is_valid_skill_name(ok), "{ok:?} must be accepted");
        }

        let pi = tempfile::tempdir().unwrap();
        // Plant a would-be victim OUTSIDE the corpus and a manifest that points
        // at it via `../`.
        let outside = pi.path().join("victim");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("SKILL.md"), "precious").unwrap();
        let corpus = pi.path().join("skills");
        std::fs::create_dir_all(&corpus).unwrap();
        std::fs::write(
            corpus.join(PI_MANIFEST_FILE),
            r#"{"manifest_version":1,"tool":"issuectl","skills":{"../victim":{"version":"1"}}}"#,
        )
        .unwrap();

        // The unsafe key is filtered on load, so it is invisible to status/prune.
        assert!(try_load_pi_manifest(&corpus)
            .unwrap()
            .unwrap()
            .skills
            .is_empty());
        let outcome = pi_prune(&corpus, true).unwrap();
        assert!(outcome.removed.is_empty());
        assert!(
            outside.join("SKILL.md").exists(),
            "prune must never touch a path outside the corpus"
        );
    }

    /// Path-traversal (symlink) regression: an issuectl-owned entry whose
    /// `<pi_root>/<name>` directory has been REPLACED by a symlink pointing at a
    /// directory OUTSIDE the corpus must never be followed by prune —
    /// `is_valid_skill_name` vets only the key, not the on-disk shape. The walk
    /// gate in `classify_pi_corpus` reports a symlinked entry dir as `Unmanaged`
    /// (never read through, never prune-eligible), so it is neither removed nor
    /// even reached the deletion gate, and the external file is untouched.
    /// Hermetic: two tempdirs, never the real `~/.pi/`.
    #[cfg(unix)]
    #[test]
    fn pi_prune_refuses_to_follow_directory_symlink_out_of_corpus() {
        let pi = tempfile::tempdir().unwrap();
        // A precious victim OUTSIDE the corpus root, in its own tempdir.
        let outside = tempfile::tempdir().unwrap();
        let external = outside.path().join("external");
        std::fs::create_dir_all(&external).unwrap();
        let victim = external.join("SKILL.md");
        std::fs::write(&victim, "precious external file").unwrap();

        // Replace a would-be orphan entry dir with a symlink to the external
        // dir: `<pi_root>/triage-bugs -> <outside>/external`.
        std::os::unix::fs::symlink(&external, pi.path().join("triage-bugs")).unwrap();

        // Own it in the manifest so it classifies as an Orphan (retired skill).
        let mut manifest = PiManifest::empty();
        manifest.skills.insert(
            "triage-bugs".into(),
            PiManifestEntry {
                version: "0.9.0".into(),
            },
        );
        save_pi_manifest(pi.path(), &manifest).unwrap();

        // Status must classify the symlinked entry as Unmanaged WITHOUT reading
        // through the link (no leaked out-of-corpus version), and never as a
        // prune-eligible Orphan/Missing.
        let report = pi_status(pi.path()).unwrap();
        let tb = row(&report, "triage-bugs");
        assert_eq!(
            tb.state,
            PiSkillState::Unmanaged,
            "a symlinked entry dir must classify Unmanaged, not Orphan/Missing"
        );
        assert!(
            tb.on_disk_version.is_none(),
            "status must not read through the symlink and leak the external file's version"
        );

        // Apply-prune must NOT follow the symlink and delete the external file,
        // and must neither remove nor touch the entry.
        let outcome = pi_prune(pi.path(), true).unwrap();
        assert!(
            victim.exists(),
            "prune must not delete a file outside the corpus via a directory symlink"
        );
        assert!(
            outcome.removed.iter().all(|i| i.name != "triage-bugs"),
            "a symlinked entry dir must never be reported removed"
        );
        // The symlink itself is left in place (prune never touches it).
        assert!(pi.path().join("triage-bugs").symlink_metadata().is_ok());

        // Defense in depth: the deletion gate ALSO refuses a symlinked entry dir
        // directly, independent of the classify-side Unmanaged verdict.
        assert!(
            !orphan_is_safely_removable(
                &pi.path().join("triage-bugs"),
                &pi.path().join("triage-bugs").join("SKILL.md"),
            ),
            "orphan_is_safely_removable must refuse a symlinked entry dir"
        );
    }

    /// Companion to the dir-symlink case: a REAL orphan entry dir whose
    /// `SKILL.md` is a symlink to a file outside the corpus must also be
    /// skipped, never followed by `remove_file`.
    #[cfg(unix)]
    #[test]
    fn pi_prune_refuses_symlinked_skill_md_out_of_corpus() {
        let pi = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("secret");
        std::fs::write(&victim, "secret contents").unwrap();

        // Real dir, but SKILL.md points OUT of the corpus.
        let dir = pi.path().join("triage-bugs");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&victim, dir.join("SKILL.md")).unwrap();

        let mut manifest = PiManifest::empty();
        manifest.skills.insert(
            "triage-bugs".into(),
            PiManifestEntry {
                version: "0.9.0".into(),
            },
        );
        save_pi_manifest(pi.path(), &manifest).unwrap();

        let outcome = pi_prune(pi.path(), true).unwrap();
        assert!(
            victim.exists(),
            "prune must not delete a symlinked SKILL.md's external target"
        );
        assert!(outcome.skipped.iter().any(|i| i.name == "triage-bugs"));
    }

    /// Install-side twin of the prune symlink guard: a `--force` install whose
    /// `<pi_root>/<name>` entry dir is a symlink to an external directory must
    /// NOT write the mirror through the link and overwrite the external
    /// `SKILL.md`. The mirror is skipped (non-fatal); the repo-local Claude
    /// install still succeeds.
    #[cfg(unix)]
    #[test]
    fn install_refuses_to_mirror_through_directory_symlink() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let external = outside.path().join("external");
        std::fs::create_dir_all(&external).unwrap();
        let victim = external.join("SKILL.md");
        std::fs::write(&victim, "external precious").unwrap();

        // `<pi_root>/issue -> <outside>/external`
        std::os::unix::fs::symlink(&external, pi.path().join("issue")).unwrap();

        install_skill(repo.path(), &[Agent::Claude], true, Some(pi.path())).unwrap();
        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "external precious",
            "install --force must not write through a corpus dir symlink"
        );
        // The repo-local Claude side is unaffected, and the writable mirrors
        // (issue-new, issue-intake) still land inside the corpus.
        assert!(repo.path().join(".claude/skills/issue/SKILL.md").exists());
        assert!(pi.path().join("issue-new/SKILL.md").exists());
    }

    /// Install-side twin for a symlinked final `SKILL.md`: a real entry dir
    /// whose `SKILL.md` is a symlink to a file outside the corpus (the classic
    /// `issue/SKILL.md -> ~/.ssh/config`) must not be overwritten through the
    /// link by a `--force` install.
    #[cfg(unix)]
    #[test]
    fn install_refuses_to_overwrite_symlinked_skill_md() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("ssh_config");
        std::fs::write(&victim, "Host *\n  secret\n").unwrap();

        let dir = pi.path().join("issue");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&victim, dir.join("SKILL.md")).unwrap();

        install_skill(repo.path(), &[Agent::Claude], true, Some(pi.path())).unwrap();
        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "Host *\n  secret\n",
            "install --force must not overwrite a symlinked SKILL.md's external target"
        );
    }

    /// The install write gate rejects a pre-existing non-regular `SKILL.md`
    /// (here: a directory) rather than blindly writing over it. The mirror is
    /// skipped (non-fatal — absent from the summary), and the repo-local Claude
    /// install still succeeds. Guards against the fail-open form where a
    /// non-`NotFound` stat outcome was treated as "safe to write".
    #[test]
    fn install_refuses_nonregular_skill_md() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        // A directory where the `issue` mirror's `SKILL.md` file belongs.
        std::fs::create_dir_all(pi.path().join("issue/SKILL.md")).unwrap();

        let results =
            install_skill_summary(repo.path(), &[Agent::Claude], true, Some(pi.path())).unwrap();
        assert!(
            !results
                .iter()
                .any(|r| r.path == pi.path().join("issue/SKILL.md")),
            "a non-regular SKILL.md must be skipped, not written"
        );
        // The directory is left untouched, and the writable mirrors still land.
        assert!(pi.path().join("issue/SKILL.md").is_dir());
        assert!(pi.path().join("issue-new/SKILL.md").is_file());
        assert!(repo.path().join(".claude/skills/issue/SKILL.md").exists());
    }

    /// The deletion gate refuses to act on a corrupt manifest rather than
    /// treating it as empty and clobbering it.
    #[test]
    fn pi_prune_refuses_corrupt_manifest() {
        let pi = tempfile::tempdir().unwrap();
        std::fs::write(pi.path().join(PI_MANIFEST_FILE), "{ not json").unwrap();
        assert!(
            pi_prune(pi.path(), true).is_err(),
            "prune must error on a corrupt manifest, not silently no-op"
        );
        // The corrupt file is left intact (not clobbered with an empty manifest).
        assert_eq!(
            std::fs::read_to_string(pi.path().join(PI_MANIFEST_FILE)).unwrap(),
            "{ not json"
        );
    }

    /// A future/unsupported manifest schema version is refused by the strict
    /// loader (old binaries must not destructively reinterpret a newer file).
    #[test]
    fn unsupported_manifest_version_is_refused() {
        let pi = tempfile::tempdir().unwrap();
        std::fs::write(
            pi.path().join(PI_MANIFEST_FILE),
            r#"{"manifest_version":999,"tool":"issuectl","skills":{}}"#,
        )
        .unwrap();
        assert!(try_load_pi_manifest(pi.path()).is_err());
        assert!(pi_prune(pi.path(), true).is_err());
    }

    /// An orphan dir that also holds sibling files (Claude skills may bundle
    /// references) is left in place: its SKILL.md is NOT torn out, and the
    /// entry is reported under `skipped`.
    #[test]
    fn pi_prune_skips_orphan_with_sibling_files() {
        let pi = tempfile::tempdir().unwrap();
        let dir = pi.path().join("triage-bugs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "retired").unwrap();
        std::fs::write(dir.join("reference.md"), "user reference").unwrap();
        let mut manifest = PiManifest::empty();
        manifest.skills.insert(
            "triage-bugs".into(),
            PiManifestEntry {
                version: "0.9.0".into(),
            },
        );
        save_pi_manifest(pi.path(), &manifest).unwrap();

        let outcome = pi_prune(pi.path(), true).unwrap();
        assert!(
            outcome.removed.is_empty(),
            "must not remove a dir with siblings"
        );
        assert_eq!(outcome.skipped.len(), 1);
        assert_eq!(outcome.skipped[0].name, "triage-bugs");
        assert!(dir.join("SKILL.md").exists(), "SKILL.md must survive");
        assert!(dir.join("reference.md").exists(), "sibling must survive");
        // The manifest row stays so the entry remains visible.
        assert!(load_pi_manifest(pi.path())
            .skills
            .contains_key("triage-bugs"));
    }

    /// A retired skill whose file is already gone reads as `Missing` (checked
    /// before orphan), and prune clears the row.
    #[test]
    fn retired_skill_with_missing_file_is_missing_not_orphan() {
        let pi = tempfile::tempdir().unwrap();
        let mut manifest = PiManifest::empty();
        manifest.skills.insert(
            "triage-bugs".into(),
            PiManifestEntry {
                version: "0.9.0".into(),
            },
        );
        save_pi_manifest(pi.path(), &manifest).unwrap();

        assert_eq!(
            row(&pi_status(pi.path()).unwrap(), "triage-bugs").state,
            PiSkillState::Missing
        );
        let outcome = pi_prune(pi.path(), true).unwrap();
        assert_eq!(outcome.removed.len(), 1);
        assert_eq!(outcome.removed[0].kind, PiPruneKind::Missing);
        assert!(!load_pi_manifest(pi.path())
            .skills
            .contains_key("triage-bugs"));
    }

    /// A present-but-unreadable owned copy (here: SKILL.md replaced by a
    /// directory) is NOT classified `Missing`, so prune never clears its
    /// provenance row for a file that still exists.
    #[test]
    fn present_but_nonregular_copy_is_not_missing() {
        let pi = tempfile::tempdir().unwrap();
        // `issue/SKILL.md` is a directory, not a regular file.
        std::fs::create_dir_all(pi.path().join("issue/SKILL.md")).unwrap();
        let mut manifest = PiManifest::empty();
        manifest.skills.insert(
            "issue".into(),
            PiManifestEntry {
                version: env!("CARGO_PKG_VERSION").into(),
            },
        );
        save_pi_manifest(pi.path(), &manifest).unwrap();

        let state = row(&pi_status(pi.path()).unwrap(), "issue").state;
        assert_ne!(
            state,
            PiSkillState::Missing,
            "a present entry is not Missing"
        );
    }

    /// `applied` is only true when changes actually landed: a `--force` prune of
    /// a clean corpus reports `applied == false`.
    #[test]
    fn prune_applied_is_false_on_noop_force() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        let outcome = pi_prune(pi.path(), true).unwrap();
        assert!(outcome.removed.is_empty());
        assert!(
            !outcome.applied,
            "a no-op --force must not claim changes landed"
        );
    }

    /// The manifest is written atomically (temp + rename): no `*.tmp` residue is
    /// left behind after a successful install.
    #[test]
    fn manifest_write_leaves_no_temp_residue() {
        let repo = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        install_skill(repo.path(), &[Agent::Claude], false, Some(pi.path())).unwrap();
        let residue: Vec<_> = std::fs::read_dir(pi.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(residue.is_empty(), "atomic write must not leave temp files");
        assert!(pi.path().join(PI_MANIFEST_FILE).is_file());
    }

    /// Concurrency regression for `pi-manifest-locking`: many processes sharing
    /// the one global pi corpus each read-modify-write the manifest, and the
    /// loser of an unlocked race silently drops the winner's row (the atomic
    /// temp+rename prevents a *torn* file but not a *lost update*). This drives
    /// the exact production primitives the install/prune paths hold their lock
    /// across — `acquire_pi_lock` → strict load → insert → `save_pi_manifest` —
    /// with every thread inserting its own distinct row, and asserts none are
    /// lost. A `Barrier` releases all threads at once and a short mid-critical
    /// sleep widens the window so the same test reliably FAILS if the lock is
    /// removed (each racer would then clobber the others down to ~one row).
    #[test]
    fn concurrent_manifest_writers_do_not_lose_entries() {
        use std::sync::{Arc, Barrier};

        let pi = tempfile::tempdir().unwrap();
        let pi_root = pi.path().to_path_buf();
        const WRITERS: usize = 8;
        let barrier = Arc::new(Barrier::new(WRITERS));

        let handles: Vec<_> = (0..WRITERS)
            .map(|i| {
                let pi_root = pi_root.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let key = format!("concurrent-skill-{i}");
                    barrier.wait(); // maximise contention: everyone starts together
                                    // The same load → modify → save discipline `record_pi_provenance`
                                    // and `pi_prune` run, but here each writer contributes a
                                    // *distinct* row so a lost update is directly observable.
                    let _lock = acquire_pi_lock(&pi_root).expect("acquire pi lock");
                    let mut manifest = try_load_pi_manifest(&pi_root)
                        .unwrap()
                        .unwrap_or_else(PiManifest::empty);
                    manifest.skills.insert(
                        key,
                        PiManifestEntry {
                            version: format!("0.0.{i}"),
                        },
                    );
                    // Widen the read-modify-write window so an unlocked run would
                    // reliably interleave (and clobber). Under the lock this only
                    // serialises the writers.
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    save_pi_manifest(&pi_root, &manifest).expect("save manifest");
                })
            })
            .collect();

        for h in handles {
            h.join().expect("writer thread panicked");
        }

        // Every writer's row must have survived the concurrent updates.
        let manifest = try_load_pi_manifest(&pi_root)
            .expect("final manifest loads")
            .expect("manifest present after writes");
        assert_eq!(
            manifest.skills.len(),
            WRITERS,
            "every concurrent writer's row must survive; got {:?}",
            manifest.skills.keys().collect::<Vec<_>>()
        );
        for i in 0..WRITERS {
            assert!(
                manifest
                    .skills
                    .contains_key(&format!("concurrent-skill-{i}")),
                "row for writer {i} was lost to a racing writer"
            );
        }
    }

    /// End-to-end companion to `concurrent_manifest_writers_do_not_lose_entries`,
    /// exercising the real lock *sites*: a `skill install` (mirror + provenance
    /// RMW) from one repo running concurrently with a `pi_prune` on the same
    /// corpus. The lock must serialise them so the run always ends with a
    /// well-formed manifest that still owns every shipped skill — never a torn
    /// file, a panic, or a managed row dropped by an interleaved prune. Each
    /// repo gets its own tempdir; the pi corpus is shared (as it is in reality).
    #[test]
    fn concurrent_install_and_prune_keep_manifest_consistent() {
        let pi = tempfile::tempdir().unwrap();
        let pi_root = pi.path().to_path_buf();

        // Seed the corpus so there is a real manifest for prune to load, plus a
        // stale orphan row (a skill this binary no longer ships) whose dir is
        // gone — a `Missing` entry prune will try to clear while install runs.
        let repo0 = tempfile::tempdir().unwrap();
        install_skill(repo0.path(), &[Agent::Claude], false, Some(&pi_root)).unwrap();
        {
            let mut m = load_pi_manifest(&pi_root);
            m.skills.insert(
                "retired-skill".to_string(),
                PiManifestEntry {
                    version: "0.0.1".to_string(),
                },
            );
            save_pi_manifest(&pi_root, &m).unwrap();
        }

        let installer = {
            let pi_root = pi_root.clone();
            std::thread::spawn(move || {
                let repo = tempfile::tempdir().unwrap();
                install_skill_summary(repo.path(), &[Agent::Claude], true, Some(&pi_root)).unwrap();
            })
        };
        let pruner = {
            let pi_root = pi_root.clone();
            std::thread::spawn(move || {
                pi_prune(&pi_root, true).unwrap();
            })
        };
        installer.join().expect("installer thread panicked");
        pruner.join().expect("pruner thread panicked");

        // The manifest must still be trustworthy (strict load succeeds) and must
        // still own every shipped skill — the interleaved prune must not have
        // dropped a row the concurrent install just (re)wrote.
        let manifest = try_load_pi_manifest(&pi_root)
            .expect("manifest stays trusted under concurrency")
            .expect("manifest present");
        for (name, _) in managed_pi_skills() {
            assert!(
                manifest.skills.contains_key(name),
                "shipped skill `{name}` lost from manifest under concurrent install+prune"
            );
        }
        // The load-bearing serializability check: whichever order the lock
        // imposes, `retired-skill` (a Missing orphan row) ends up *gone* — under
        // serial [install, prune] prune clears it; under [prune, install]
        // install preserves only the managed set and never re-adds it. Its
        // survival would mean a stale install save landed after prune's delete —
        // a non-serializable result the lock exists to prevent.
        assert!(
            !manifest.skills.contains_key("retired-skill"),
            "pruned orphan row was resurrected by a racing install save — lock did not serialize"
        );
        // No temp residue from a half-finished atomic write.
        let residue: Vec<_> = std::fs::read_dir(&pi_root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(
            residue.is_empty(),
            "no temp residue after concurrent writes"
        );
    }
}
