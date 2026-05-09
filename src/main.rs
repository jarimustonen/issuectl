mod body_sections;
mod canonical;
mod context;
mod docs;
mod doctor;
mod fmt;
mod hooks;
mod item_text;
mod merge_driver;
mod models;
mod mutate;
mod parser;
mod query;
mod repo;
mod schema;
mod server;
mod skill;
mod slug;
mod write;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::builder::PossibleValuesParser;
use clap::{Parser, Subcommand, ValueEnum};

pub(crate) const ISSUE_TYPES: &[&str] = &["bug", "task", "feature", "improvement", "chore", "epic"];
pub(crate) const PRIORITIES: &[&str] = &["normal", "high"];
pub(crate) const ACTIVE_STATUSES: &[&str] = &["open", "in-progress", "testing"];
pub(crate) const CLOSING_STATUSES: &[&str] = &[
    "done",
    "fixed",
    "wontfix",
    "duplicate",
    "cannot-reproduce",
    "obsolete",
];

pub(crate) fn all_statuses() -> Vec<&'static str> {
    ACTIVE_STATUSES
        .iter()
        .chain(CLOSING_STATUSES.iter())
        .copied()
        .collect()
}

pub(crate) fn is_closing_status(status: &str) -> bool {
    CLOSING_STATUSES.contains(&status)
}

/// Public re-export of `normalize_related_refs` for the mutate module
/// so it can validate `add_related` / `remove_related` exactly the way
/// the CLI does, without duplicating the logic.
pub(crate) fn normalize_related_refs_pub(refs: &[String]) -> anyhow::Result<Vec<String>> {
    normalize_related_refs(refs)
}

const TOP_LEVEL_HELP: &str = "\
Examples:
  issuectl ls                              List open issues
  issuectl ls -t bug -p high               Filter by type and priority
  issuectl ls --closed --json              Closed issues as JSON
  issuectl show extremely-quiet-otter          Full details by slug
  issuectl search redirect                 Keyword search
  issuectl new --type bug --title \"...\"    Create a new issue (random slug)
  issuectl update <slug> --status testing  Change status
  issuectl close <slug> --status fixed     Set a closing status (fixed/done/...)
  issuectl doctor                          Health-check the repo
  issuectl doctor --fix                    Migrate legacy numbered issues
  issuectl skill install                   Install /issue skill in current repo
  issuectl serve                           Run a local Trello-style web board
  issuectl docs                            List bundled documentation topics
  issuectl docs kanban                     Print the kanban / web-board doc
";

#[derive(Parser)]
#[command(
    name = "issuectl",
    version,
    about = "Manage markdown-based issues with frontmatter",
    after_help = TOP_LEVEL_HELP
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Output as JSON
    #[arg(global = true, long)]
    json: bool,

    /// Override the repo root (the directory that contains issues/). Useful
    /// for pointing issuectl at an external project from another working
    /// directory. When omitted, issuectl walks up from cwd looking for
    /// issues/ or .git.
    #[arg(global = true, long, value_name = "PATH")]
    root: Option<PathBuf>,
}

static ROOT_OVERRIDE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

fn parse_non_empty(s: &str) -> std::result::Result<String, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        Err("value cannot be empty or whitespace".to_string())
    } else if trimmed.len() != s.len() {
        Err(format!(
            "value has leading or trailing whitespace: {s:?} (use {trimmed:?})"
        ))
    } else {
        Ok(s.to_string())
    }
}

fn parse_custom_field(s: &str) -> std::result::Result<(String, String), String> {
    let (key, value) = s
        .split_once('=')
        .ok_or_else(|| format!("expected key=value, got {s:?}"))?;
    if key.is_empty() || value.is_empty() {
        return Err(format!("expected non-empty key=value, got {s:?}"));
    }
    // Reject leading/trailing whitespace rather than silently trimming
    // — matches `parse_non_empty` and keeps the CLI consistent with
    // `UpdateIssueRequest::validate`'s API-side whitespace check.
    if key.trim() != key || value.trim() != value {
        return Err(format!(
            "field {s:?} has leading or trailing whitespace; remove it"
        ));
    }
    if !mutate::is_valid_custom_field_key(key) {
        return Err(format!(
            "field key {key:?} must be alphanumeric / underscore / hyphen"
        ));
    }
    if let Some(hint) = mutate::reserved_custom_field_hint(key) {
        return Err(format!("field {key:?} is built-in: {hint}"));
    }
    Ok((key.to_string(), value.to_string()))
}

/// Clap value parser for `--clear-field <key>`. Same shape and reserved
/// rules as `parse_custom_field`, but consumes a bare key (no `=value`).
fn parse_custom_field_key(s: &str) -> std::result::Result<String, String> {
    if s.is_empty() {
        return Err(format!("expected a non-empty field key, got {s:?}"));
    }
    if s.trim() != s {
        return Err(format!(
            "field key {s:?} has leading or trailing whitespace; remove it"
        ));
    }
    if !mutate::is_valid_custom_field_key(s) {
        return Err(format!(
            "field key {s:?} must be alphanumeric / underscore / hyphen"
        ));
    }
    if let Some(hint) = mutate::reserved_custom_field_hint(s) {
        return Err(format!("field {s:?} is built-in: {hint}"));
    }
    Ok(s.to_string())
}

/// Clap value parser for any slug-shaped CLI argument. Rejects anything
/// that wouldn't pass [`slug::is_valid`], which closes the path-traversal
/// door for `Show/Update/Close <slug>` and keeps `--epic` / `--related`
/// in line with the canonical slug shape.
fn parse_slug_arg(s: &str) -> std::result::Result<String, String> {
    let s = parse_non_empty(s)?;
    if !slug::is_valid(&s) {
        return Err(format!(
            "{s:?} is not a valid slug (lowercase ASCII, kebab-case, ≥2 segments, no path separators)"
        ));
    }
    Ok(s)
}

#[derive(Subcommand)]
enum Command {
    /// List or query issues by frontmatter fields
    #[command(alias = "ls")]
    List {
        /// Optional query string. Same syntax as `search` and the web
        /// `?q=` filter (e.g. `status:in-progress -label:wontfix`).
        /// When supplied, the implicit "open only" default is disabled
        /// — combine with `--all` / `--closed` or an explicit
        /// `folder:`/`status:` term as needed. Pass leading-hyphen
        /// negations as a single quoted argument: `ls "-label:wontfix"`.
        #[arg(value_parser = parse_non_empty, allow_hyphen_values = true)]
        query: Option<String>,

        /// Filter by assignee (or owner for epics)
        #[arg(short = 'a', long, value_parser = parse_non_empty)]
        assignee: Option<String>,

        /// Filter by type
        #[arg(short = 't', long = "type", value_parser = PossibleValuesParser::new(ISSUE_TYPES))]
        issue_type: Option<String>,

        /// Filter by priority
        #[arg(short = 'p', long, value_parser = PossibleValuesParser::new(PRIORITIES))]
        priority: Option<String>,

        /// Filter by status
        #[arg(short = 's', long, value_parser = PossibleValuesParser::new(all_statuses()))]
        status: Option<String>,

        /// Filter by parent epic slug
        #[arg(short = 'e', long, value_parser = parse_slug_arg)]
        epic: Option<String>,

        /// Filter by label
        #[arg(short = 'l', long, value_parser = parse_non_empty)]
        label: Option<String>,

        /// Include closed issues
        #[arg(long)]
        all: bool,

        /// Show only closed issues
        #[arg(long)]
        closed: bool,
    },

    /// Show full details of a single issue
    Show {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
    },

    /// Search issues by keyword in title, slug, and body
    Search {
        /// Search keyword
        #[arg(value_parser = parse_non_empty)]
        query: String,

        /// Include closed issues
        #[arg(long)]
        all: bool,
    },

    /// Show summary statistics
    Stats,

    /// Create a new issue or epic (random slug auto-generated)
    New {
        /// Item type
        #[arg(short = 't', long = "type", value_parser = PossibleValuesParser::new(ISSUE_TYPES))]
        issue_type: String,

        /// Item title (markdown heading)
        #[arg(long, value_parser = parse_non_empty)]
        title: String,

        /// Override the auto-generated slug (any kebab-case identifier)
        #[arg(long, value_parser = parse_non_empty)]
        slug: Option<String>,

        /// Reporter username (issues only)
        #[arg(long, value_parser = parse_non_empty)]
        reporter: Option<String>,

        /// Assignee username (issues only)
        #[arg(short = 'a', long, value_parser = parse_non_empty)]
        assignee: Option<String>,

        /// Owner username (epics only)
        #[arg(long, value_parser = parse_non_empty)]
        owner: Option<String>,

        /// Priority
        #[arg(short = 'p', long, default_value = "normal", value_parser = PossibleValuesParser::new(PRIORITIES))]
        priority: String,

        /// Parent epic slug
        #[arg(short = 'e', long, value_parser = parse_slug_arg)]
        epic: Option<String>,

        /// Add a label (repeatable)
        #[arg(short = 'l', long = "label", value_parser = parse_non_empty)]
        labels: Vec<String>,

        /// Add a related issue reference like "@extremely-quiet-otter" or bare slug (repeatable)
        #[arg(long = "related", value_parser = parse_non_empty)]
        related: Vec<String>,

        /// Source line for the body (e.g. "frontend/login")
        #[arg(long, value_parser = parse_non_empty)]
        source: Option<String>,

        /// Description body (free text)
        #[arg(long, value_parser = parse_non_empty)]
        description: Option<String>,

        /// Set a custom frontmatter field (repeatable). Format `key=value`.
        /// Use this for fields the schema declares but no built-in flag
        /// covers (e.g. `--field team=payments`). Built-in fields use
        /// their dedicated flags (`--type`, `--priority`, ...).
        #[arg(long = "field", value_parser = parse_custom_field)]
        custom_fields: Vec<(String, String)>,
    },

    /// Update fields of an existing issue or epic
    Update {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// New status (active or closing — frontmatter only, no directory move)
        #[arg(short = 's', long, value_parser = PossibleValuesParser::new(all_statuses()))]
        status: Option<String>,

        /// New assignee (issues)
        #[arg(short = 'a', long, value_parser = parse_non_empty)]
        assignee: Option<String>,

        /// New owner (epics)
        #[arg(long, value_parser = parse_non_empty)]
        owner: Option<String>,

        /// New priority
        #[arg(short = 'p', long, value_parser = PossibleValuesParser::new(PRIORITIES))]
        priority: Option<String>,

        /// Set parent epic slug
        #[arg(short = 'e', long, value_parser = parse_slug_arg)]
        epic: Option<String>,

        /// Remove the parent epic reference
        #[arg(long, conflicts_with = "epic")]
        no_epic: bool,

        /// Add a label (repeatable)
        #[arg(long = "add-label", value_parser = parse_non_empty)]
        add_labels: Vec<String>,

        /// Remove a label (repeatable)
        #[arg(long = "remove-label", value_parser = parse_non_empty)]
        remove_labels: Vec<String>,

        /// Add a related reference like "@<slug>" or bare slug (repeatable)
        #[arg(long = "add-related", value_parser = parse_non_empty)]
        add_related: Vec<String>,

        /// Remove a related reference (repeatable)
        #[arg(long = "remove-related", value_parser = parse_non_empty)]
        remove_related: Vec<String>,

        /// Record a commit, format HASH:summary (repeatable)
        #[arg(long = "add-commit", value_parser = parse_non_empty)]
        add_commits: Vec<String>,

        /// Set a custom frontmatter field (repeatable). Format `key=value`.
        /// Mirrors `new --field`. Built-in fields use their dedicated
        /// flags (`--status`, `--priority`, ...).
        #[arg(long = "field", value_parser = parse_custom_field)]
        custom_fields: Vec<(String, String)>,

        /// Remove a custom frontmatter field (repeatable). Built-in fields
        /// have dedicated removal mechanics (e.g. `--no-epic`); use this
        /// only for keys the schema or client added beyond the built-in
        /// set.
        #[arg(long = "clear-field", value_parser = parse_custom_field_key)]
        clear_fields: Vec<String>,

        /// Optimistic-concurrency token from a prior `show`/`list --json`.
        /// Required when `--json` is in effect (the `--json` channel is the
        /// AI-agent surface, where blind clobber is unacceptable). Optional
        /// for human invocations — `flock` still prevents corruption.
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },

    /// Set a closing status (frontmatter only; flat layout has no directory move)
    Close {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Closing status (default: `fixed` for bugs, `done` otherwise)
        #[arg(short = 's', long, value_parser = PossibleValuesParser::new(CLOSING_STATUSES))]
        status: Option<String>,

        /// Record a commit, format HASH:summary (repeatable)
        #[arg(long = "commit", value_parser = parse_non_empty)]
        commits: Vec<String>,

        /// Optimistic-concurrency token; same semantics as `update`.
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },

    /// Append a timestamped block to an issue's `## Comments` section
    /// (or `## Decisions` / `## Agent Runs` with `--decision` / `--agent-run`)
    Note {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Author of the note (e.g. `alice` or `agent-name`)
        #[arg(long = "as", value_parser = parse_non_empty)]
        author: String,

        /// Note text (one positional argument; quote multi-word input)
        #[arg(value_parser = parse_non_empty)]
        message: String,

        /// Append to the `## Decisions` section instead of `## Comments`.
        #[arg(long, conflicts_with = "agent_run")]
        decision: bool,

        /// Append to the `## Agent Runs` section instead of `## Comments`.
        #[arg(long = "agent-run", conflicts_with = "decision")]
        agent_run: bool,

        /// Plan only: print a unified diff and exit 0 without writing.
        #[arg(long)]
        dry_run: bool,

        /// Optimistic-concurrency token; required with --json
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },

    /// Set a single frontmatter field. Built-in fields (`status`,
    /// `priority`, `assignee`, `owner`, `epic`) use the typed update
    /// path; any other key goes through the schema-validated
    /// `custom_fields` slot. Use `--clear` to remove a (non-status)
    /// field.
    Set {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Field name (e.g. `status`, `priority`, or a schema-declared
        /// custom key like `team`)
        #[arg(value_parser = parse_non_empty)]
        field: String,

        /// New value. Required unless `--clear` is given.
        #[arg(value_parser = parse_non_empty, required_unless_present = "clear")]
        value: Option<String>,

        /// Remove the field instead of setting it. Conflicts with `value`.
        #[arg(long, conflicts_with = "value")]
        clear: bool,

        /// Plan only: print a unified diff and exit 0 without writing.
        #[arg(long)]
        dry_run: bool,

        /// Optimistic-concurrency token; required with --json
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },

    /// Toggle a markdown checklist item in the issue body. Matches a
    /// unique line containing the substring whose stripped text starts
    /// with `- [ ]` or `- [x]`.
    Check {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Substring of the task line to match (must be unique across
        /// the body's checkbox lines).
        #[arg(value_parser = parse_non_empty)]
        task: String,

        /// Plan only: print a unified diff and exit 0 without writing.
        #[arg(long)]
        dry_run: bool,

        /// Optimistic-concurrency token; required with --json
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },

    /// Add or remove a label. Re-running the same call is safe: a
    /// duplicate add is a no-op on labels (the list is deduped) and
    /// removing an absent label is a no-op too. Note that the
    /// `updated:` frontmatter date is still bumped on every call —
    /// idempotency here means "won't error / won't double the
    /// label," not "byte-identical file."
    Label {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Operation
        #[arg(value_enum)]
        op: LabelOp,

        /// Label
        #[arg(value_parser = parse_non_empty)]
        label: String,

        /// Plan only: print a unified diff and exit 0 without writing.
        #[arg(long)]
        dry_run: bool,

        /// Optimistic-concurrency token; required with --json
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },

    /// Apply a multi-field YAML patch in a single transaction.
    /// The file declares `slug:` plus any combination of built-in
    /// fields, `custom_fields:`, label/related list ops, and commits
    /// — all applied under one flock with one schema-validation pass.
    Apply {
        /// Path to the YAML patch file
        #[arg(value_name = "PATCH")]
        patch: PathBuf,

        /// Plan only: print a unified diff and exit 0 without writing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Edit issue body markdown
    Body {
        #[command(subcommand)]
        action: BodyAction,
    },

    /// Health-check the repo and (with --fix) migrate legacy layouts and
    /// numbered issues to the canonical flat slug layout
    Doctor {
        /// Apply migrations and fixes (otherwise read-only report)
        #[arg(long)]
        fix: bool,
    },

    /// Install / uninstall the opt-in pre-commit hook that runs
    /// `issuectl doctor` on staged issue files
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },

    /// Install or preview the /issue skill template (Claude Code or Codex)
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },

    /// Print bundled long-form documentation. Run without an argument to
    /// list available topics.
    Docs {
        /// Topic name (e.g. `kanban`). Omit to list topics.
        topic: Option<String>,
    },

    /// Run a local read-only web board (Trello-style) for the current repo
    Serve {
        /// Port to listen on
        #[arg(long, default_value_t = 7878)]
        port: u16,

        /// Host/interface to bind to (default: 127.0.0.1, local-only)
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Disable the filesystem watcher (no live updates; manual page
        /// reload only). Useful when running over a filesystem where
        /// `notify` is unreliable, or for read-only diagnostics.
        #[arg(long)]
        no_watch: bool,

        /// Number of distinct slugs touched in a single debounce window
        /// above which per-issue events collapse into a single Resync
        /// (e.g. `git checkout` of a feature branch).
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..))]
        watch_bulk_threshold: u32,

        /// Force the polling watcher backend with the given interval in
        /// milliseconds. Use this on network filesystems (NFS/SMB)
        /// where `notify`'s native backend misses events. Default off:
        /// the platform-native backend is used.
        ///
        /// Floor 500ms because the advertised use case is network
        /// filesystems, where aggressive polling is exactly the wrong
        /// default — recursive 50ms stat()s over NFS would make the
        /// board itself the source of filesystem load. Upper bound
        /// 60s prevents typos like `--watch-poll-ms 6000000` from
        /// silently disabling polling. Tests build `WatcherBackend::
        /// Poll` directly without going through clap, so this floor
        /// does not affect test runtime.
        #[arg(long, value_name = "MS",
              conflicts_with = "no_watch",
              value_parser = clap::value_parser!(u64).range(500..=60_000))]
        watch_poll_ms: Option<u64>,

        /// Enable PATCH/POST writes when bound to a non-loopback
        /// address. Default off: non-loopback binds are read-only.
        /// Loopback binds always allow writes.
        #[arg(long)]
        allow_remote_writes: bool,
    },

    /// Normalize `item.md` files: canonical key order, sorted arrays,
    /// trimmed whitespace, ATX headings. Idempotent.
    Fmt {
        /// Specific slugs to format. Default: every flat-layout issue.
        #[arg(value_parser = parse_slug_arg)]
        slugs: Vec<String>,

        /// Don't write — exit non-zero if any file would change. CI mode.
        #[arg(long, conflicts_with = "diff")]
        check: bool,

        /// Don't write — print a unified diff for files that would change.
        #[arg(long, conflicts_with = "check")]
        diff: bool,
    },

    /// Custom git merge driver for `issues/**/*.md`. Invoked by git
    /// after `install-merge-driver` configures the driver. Hidden from
    /// `--help` because end users do not call this directly.
    #[command(hide = true)]
    MergeDriver {
        #[arg(long, value_name = "PATH")]
        base: PathBuf,
        #[arg(long, value_name = "PATH")]
        ours: PathBuf,
        #[arg(long, value_name = "PATH")]
        theirs: PathBuf,
        #[arg(long, value_name = "PATH")]
        output: PathBuf,
    },

    /// Render an agent context bundle for an issue (issue + parent epic +
    /// related/blocking refs + body sections + commits + schema rules).
    /// Read-only — never mutates `issues/`. Output is byte-deterministic
    /// for caching by downstream agents.
    Context {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Write the bundle into `.issuectl/cache/agent/<slug>/` (gitignored)
        /// instead of (or in addition to) printing it.
        #[arg(long)]
        write: bool,
    },

    /// Render a repo-local prompt template against an issue's context
    /// bundle. Templates live at `.issuectl/prompts/<template>.md` and use
    /// `{{key}}` substitution; unknown keys are left intact.
    Prompt {
        /// Template name (e.g. `implement` → `.issuectl/prompts/implement.md`).
        #[arg(value_parser = parse_non_empty)]
        template: String,

        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Write the rendered prompt into
        /// `.issuectl/cache/agent/<slug>/prompts/<template>.md`.
        #[arg(long)]
        write: bool,
    },

    /// Print the `.gitattributes` and `git config` snippets to wire up
    /// the issuectl-yaml merge driver. Pass `--apply` to also run
    /// `git config` for this repo (does not modify `.gitattributes`).
    InstallMergeDriver {
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum LabelOp {
    Add,
    Remove,
}

#[derive(Subcommand)]
enum BodyAction {
    /// Replace the markdown body of an issue. Read content from stdin
    /// (`--stdin`) or a file (`--from-file PATH`). With `--json`,
    /// `--expected-version` is required (D4=B). Without `--json`,
    /// `flock` still prevents corruption but blind clobber is allowed.
    Set {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Read body from stdin
        #[arg(long, conflicts_with = "from_file")]
        stdin: bool,

        /// Read body from this file
        #[arg(long = "from-file", value_name = "PATH", conflicts_with = "stdin")]
        from_file: Option<PathBuf>,

        /// Optimistic-concurrency token; required with --json
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },
}

#[derive(Subcommand)]
enum HooksAction {
    /// Install (or uninstall with --uninstall) the pre-commit hook
    Install {
        /// Remove the hook block + revert `core.hooksPath` instead of
        /// installing
        #[arg(long)]
        uninstall: bool,

        /// Overwrite an existing non-`.githooks` `core.hooksPath`
        /// (e.g. `.husky`). The prior value is stashed and restored
        /// on uninstall.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum SkillAction {
    /// Install the /issue skill template into the current repo. By default
    /// installs the Claude Code skill; use --agent codex for Codex CLI, or
    /// --agent all for both.
    Install {
        /// Which agent's skill format to install
        #[arg(short = 'a', long, default_value = "claude", value_parser = PossibleValuesParser::new(["claude", "codex", "all"]))]
        agent: String,

        /// Overwrite existing files
        #[arg(long)]
        force: bool,
    },
    /// Print the skill template to stdout (preview before installing)
    Print {
        /// Which agent's skill format to print
        #[arg(short = 'a', long, default_value = "claude", value_parser = PossibleValuesParser::new(["claude", "codex"]))]
        agent: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let json_output = cli.json;
    ROOT_OVERRIDE.set(cli.root).ok();

    match cli.command {
        Command::List {
            query,
            assignee,
            issue_type,
            priority,
            status,
            epic,
            label,
            all,
            closed,
        } => cmd_list(
            json_output,
            query,
            assignee,
            issue_type,
            priority,
            status,
            epic,
            label,
            all,
            closed,
        ),
        Command::Show { slug } => cmd_show(json_output, &slug),
        Command::Search { query, all } => cmd_search(json_output, &query, all),
        Command::Stats => cmd_stats(json_output),
        Command::New {
            issue_type,
            title,
            slug,
            reporter,
            assignee,
            owner,
            priority,
            epic,
            labels,
            related,
            source,
            description,
            custom_fields,
        } => cmd_new(
            json_output,
            NewArgs {
                issue_type,
                title,
                slug,
                reporter,
                assignee,
                owner,
                priority,
                epic,
                labels,
                related,
                source,
                description,
                custom_fields,
            },
        ),
        Command::Update {
            slug,
            status,
            assignee,
            owner,
            priority,
            epic,
            no_epic,
            add_labels,
            remove_labels,
            add_related,
            remove_related,
            add_commits,
            custom_fields,
            clear_fields,
            expected_version,
        } => cmd_update(
            json_output,
            UpdateArgs {
                slug,
                status,
                assignee,
                owner,
                priority,
                epic,
                no_epic,
                add_labels,
                remove_labels,
                add_related,
                remove_related,
                add_commits,
                custom_fields,
                clear_fields,
                expected_version,
            },
        ),
        Command::Close {
            slug,
            status,
            commits,
            expected_version,
        } => cmd_close(json_output, &slug, status, commits, expected_version),
        Command::Note {
            slug,
            author,
            message,
            decision,
            agent_run,
            dry_run,
            expected_version,
        } => cmd_note(
            json_output,
            &slug,
            &author,
            &message,
            decision,
            agent_run,
            dry_run,
            expected_version,
        ),
        Command::Set {
            slug,
            field,
            value,
            clear,
            dry_run,
            expected_version,
        } => cmd_set(json_output, &slug, &field, value, clear, dry_run, expected_version),
        Command::Check {
            slug,
            task,
            dry_run,
            expected_version,
        } => cmd_check(json_output, &slug, &task, dry_run, expected_version),
        Command::Label {
            slug,
            op,
            label,
            dry_run,
            expected_version,
        } => cmd_label(json_output, &slug, op, &label, dry_run, expected_version),
        Command::Apply { patch, dry_run } => cmd_apply(json_output, &patch, dry_run),
        Command::Body { action } => match action {
            BodyAction::Set {
                slug,
                stdin,
                from_file,
                expected_version,
            } => cmd_body_set(json_output, &slug, stdin, from_file, expected_version),
        },
        Command::Doctor { fix } => doctor::run(&find_root(), fix, json_output),
        Command::Hooks { action } => match action {
            HooksAction::Install { uninstall, force } => {
                hooks::run(&find_root(), uninstall, force)
            }
        },
        Command::Skill { action } => match action {
            SkillAction::Install { agent, force } => cmd_skill_install(&agent, force),
            SkillAction::Print { agent } => cmd_skill_print(&agent),
        },
        Command::Docs { topic } => docs::run(topic),
        Command::Serve {
            port,
            host,
            no_watch,
            watch_bulk_threshold,
            watch_poll_ms,
            allow_remote_writes,
        } => server::run(
            find_root(),
            host,
            port,
            server::ServeOptions {
                watch_enabled: !no_watch,
                watch_bulk_threshold: watch_bulk_threshold as usize,
                watch_poll_interval: watch_poll_ms.map(std::time::Duration::from_millis),
                allow_remote_writes,
            },
        ),
        Command::Fmt { slugs, check, diff } => cmd_fmt(json_output, slugs, check, diff),
        Command::MergeDriver {
            base,
            ours,
            theirs,
            output,
        } => {
            let code = merge_driver::run(&merge_driver::MergeArgs {
                base,
                ours,
                theirs,
                output,
            })?;
            std::process::exit(code);
        }
        Command::Context { slug, write } => cmd_context(json_output, &slug, write),
        Command::Prompt {
            template,
            slug,
            write,
        } => cmd_prompt(json_output, &template, &slug, write),
        Command::InstallMergeDriver { apply } => {
            let root = find_root();
            merge_driver::install(&root, apply)
        }
    }
}

fn cmd_fmt(json: bool, slugs: Vec<String>, check: bool, diff: bool) -> Result<()> {
    let mode = if check {
        fmt::FormatMode::Check
    } else if diff {
        fmt::FormatMode::Diff
    } else {
        fmt::FormatMode::Write
    };
    let root = find_root();
    let results = fmt::format_repo(&root, &slugs, mode)?;
    let any_changed = results
        .iter()
        .any(|r| r.status == fmt::FormatStatus::Changed);

    if json {
        let entries: Vec<_> = results
            .iter()
            .map(|r| {
                let mut o = serde_json::json!({
                    "path": r.path.to_string_lossy(),
                    "status": match r.status {
                        fmt::FormatStatus::Unchanged => "unchanged",
                        fmt::FormatStatus::Changed => "changed",
                    },
                });
                // Include the diff when --diff requested so JSON
                // consumers don't lose what the human pretty-printer
                // would have shown (M6).
                if let (Some(d), serde_json::Value::Object(map)) = (&r.diff, &mut o) {
                    map.insert("diff".into(), serde_json::Value::String(d.clone()));
                }
                o
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        for r in &results {
            match r.status {
                fmt::FormatStatus::Unchanged => {}
                fmt::FormatStatus::Changed => match mode {
                    fmt::FormatMode::Write => println!("formatted: {}", r.path.display()),
                    fmt::FormatMode::Check => println!("would format: {}", r.path.display()),
                    fmt::FormatMode::Diff => {
                        if let Some(d) = &r.diff {
                            print!("{d}");
                        }
                    }
                },
            }
        }
        if !any_changed && mode != fmt::FormatMode::Diff {
            println!("All {} file(s) already formatted.", results.len());
        }
    }

    if check && any_changed {
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_context(json: bool, slug: &str, write: bool) -> Result<()> {
    let root = find_root();
    let bundle = context::build(&root, slug)?;
    let (filename, content) = if json {
        ("context.json", context::render_json(&bundle)?)
    } else {
        ("context.md", context::render_markdown(&bundle))
    };
    if write {
        let path = context::write_artifact(&root, slug, &[filename], &content)?;
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "path": path.to_string_lossy(),
                    "slug": slug,
                }))?
            );
        } else {
            println!("wrote {}", path.display());
        }
    } else {
        print!("{content}");
    }
    Ok(())
}

fn cmd_prompt(json: bool, template: &str, slug: &str, write: bool) -> Result<()> {
    let root = find_root();
    let bundle = context::build(&root, slug)?;
    let tpl = context::load_template(&root, template)?;
    let rendered = context::render_prompt(&tpl, &bundle);
    if write {
        let segments = context::prompt_cache_segments(template)?;
        let segs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
        let path = context::write_artifact(&root, slug, &segs, &rendered)?;
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "path": path.to_string_lossy(),
                    "slug": slug,
                    "template": template,
                    "rendered": rendered,
                }))?
            );
        } else {
            println!("wrote {}", path.display());
        }
    } else if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "slug": slug,
                "template": template,
                "rendered": rendered,
            }))?
        );
    } else {
        print!("{rendered}");
    }
    Ok(())
}

fn find_root() -> PathBuf {
    if let Some(Some(p)) = ROOT_OVERRIDE.get() {
        if !p.join("issues").is_dir() {
            eprintln!(
                "Error: --root {} does not contain an issues/ directory",
                p.display()
            );
            std::process::exit(1);
        }
        return p.clone();
    }
    repo::find_repo_root(None)
}

fn load() -> Vec<models::Issue> {
    let root = find_root();
    repo::load_issues(&root)
}

// ── Commands ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn cmd_list(
    json: bool,
    query_str: Option<String>,
    assignee: Option<String>,
    issue_type: Option<String>,
    priority: Option<String>,
    status: Option<String>,
    epic: Option<String>,
    label: Option<String>,
    all: bool,
    closed: bool,
) -> Result<()> {
    let mut q = match query_str.as_deref() {
        Some(s) => query::parse(s).context("parsing positional query")?,
        None => query::Query::default(),
    };

    // Translate flag filters into query terms. Flag values are
    // pre-validated by clap (PossibleValuesParser) so they can't
    // smuggle in `:`/`-` syntax that would re-enter the parser.
    if let Some(a) = assignee {
        q.push(query::Term::Field {
            field: query::FieldName::Assignee,
            m: query::FieldMatch::Equals(a),
            negated: false,
        });
    }
    if let Some(t) = issue_type {
        q.push(query::Term::Field {
            field: query::FieldName::Type,
            m: query::FieldMatch::Equals(t),
            negated: false,
        });
    }
    if let Some(p) = priority {
        q.push(query::Term::Field {
            field: query::FieldName::Priority,
            m: query::FieldMatch::Equals(p),
            negated: false,
        });
    }
    if let Some(s) = status {
        q.push(query::Term::Field {
            field: query::FieldName::Status,
            m: query::FieldMatch::Equals(s),
            negated: false,
        });
    }
    if let Some(e) = epic {
        q.push(query::Term::Field {
            field: query::FieldName::Epic,
            m: query::FieldMatch::Equals(e),
            negated: false,
        });
    }
    if let Some(l) = label {
        q.push(query::Term::Field {
            field: query::FieldName::Label,
            m: query::FieldMatch::Equals(l),
            negated: false,
        });
    }

    // Implicit folder default depends only on the CLI flags, not on
    // translated terms — otherwise `--status fixed` would silently
    // surface closed issues, breaking backwards compat with the
    // pre-query-engine `ls`. A *positional* query is the one
    // surface where the caller has explicitly opted into "scope it
    // yourself" mode.
    let folder_filter: Option<&'static str> = if all {
        None
    } else if closed {
        Some("closed")
    } else if query_str.is_some() {
        None
    } else {
        Some("open")
    };

    let issues = load();
    // `repo::load_issues` already returns issues sorted by slug, so
    // we don't re-sort here.
    let filtered: Vec<_> = issues
        .into_iter()
        .filter(|i| {
            folder_filter.map(|f| i.folder == f).unwrap_or(true) && query::matches(&q, i)
        })
        .collect();

    if json {
        let with_version: Vec<_> = filtered
            .iter()
            .map(|i| {
                let mut v = serde_json::to_value(i).expect("Issue serializes");
                if let serde_json::Value::Object(ref mut m) = v {
                    m.insert(
                        "version".into(),
                        serde_json::Value::String(canonical::canonical_hash(i)),
                    );
                }
                v
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&with_version)?);
    } else {
        print_issue_table(&filtered);
    }

    Ok(())
}

fn cmd_show(json: bool, slug: &str) -> Result<()> {
    let issues = load();
    let issue = issues.iter().find(|i| i.slug == slug);

    match issue {
        Some(i) => {
            if json {
                let mut v = serde_json::to_value(i).expect("Issue serializes");
                if let serde_json::Value::Object(ref mut m) = v {
                    m.insert(
                        "version".into(),
                        serde_json::Value::String(canonical::canonical_hash(i)),
                    );
                }
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print_issue_detail(i);
            }
            Ok(())
        }
        None => {
            eprintln!("Error: issue {slug} not found");
            std::process::exit(1);
        }
    }
}

fn cmd_search(json: bool, query_str: &str, all: bool) -> Result<()> {
    let q = query::parse(query_str).context("parsing search query")?;
    let issues = load();

    // `search` keeps the historical scope rule: open-only unless
    // `--all`. A positive `folder:`/`status:` term in the query
    // can still expand scope, but a negated one (e.g.
    // `-status:wontfix`) is exclusion, not scope expansion.
    let scope_expanded = all
        || q.has_positive_field(query::FieldName::Folder)
        || q.has_positive_field(query::FieldName::Status);

    let mut filtered: Vec<_> = issues
        .into_iter()
        .filter(|i| {
            if !scope_expanded && i.folder != "open" {
                return false;
            }
            query::matches(&q, i)
        })
        .collect();

    filtered.sort_by(|a, b| a.slug.cmp(&b.slug));

    if json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
    } else {
        print_issue_table(&filtered);
    }

    Ok(())
}

fn cmd_stats(json: bool) -> Result<()> {
    let issues = load();

    let open_count = issues.iter().filter(|i| i.folder == "open").count();
    let closed_count = issues.iter().filter(|i| i.folder == "closed").count();

    if json {
        let open_issues: Vec<_> = issues.iter().filter(|i| i.folder == "open").collect();
        let out = serde_json::json!({
            "total": issues.len(),
            "open": open_count,
            "closed": closed_count,
            "by_type": count_by_json(&open_issues, |i| &i.issue_type),
            "by_status": count_by_json(&open_issues, |i| &i.status),
            "by_priority": count_by_json(&open_issues, |i| &i.priority),
            "by_assignee": count_by_json(&open_issues, |i| {
                let a = i.effective_assignee();
                if a.is_empty() { "(none)" } else { a }
            }),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!(
            "Total: {}  (open: {}, closed: {})",
            issues.len(),
            open_count,
            closed_count
        );
        println!();

        let open_issues: Vec<_> = issues.iter().filter(|i| i.folder == "open").collect();
        print_counts("By type (open):", &open_issues, |i| &i.issue_type);
        print_counts("By status (open):", &open_issues, |i| &i.status);
        print_counts("By priority (open):", &open_issues, |i| &i.priority);
        print_counts("By assignee (open):", &open_issues, |i| {
            let a = i.effective_assignee();
            if a.is_empty() {
                "(none)"
            } else {
                a
            }
        });
    }

    Ok(())
}

pub(crate) struct NewArgs {
    pub issue_type: String,
    pub title: String,
    pub slug: Option<String>,
    pub reporter: Option<String>,
    pub assignee: Option<String>,
    pub owner: Option<String>,
    pub priority: String,
    pub epic: Option<String>,
    pub labels: Vec<String>,
    pub related: Vec<String>,
    pub source: Option<String>,
    pub description: Option<String>,
    pub custom_fields: Vec<(String, String)>,
}

pub(crate) struct NewOutcome {
    pub slug: String,
    pub title: String,
    pub item_path: PathBuf,
}

/// Typed error surfaced by `do_new_locked`. The mutate boundary maps
/// each variant to a `MutateError` (`Conflict` is renamed to
/// `ConflictingIntent`; the rest are 1:1) so the API picks the right
/// HTTP status without string-matching the formatted `anyhow::Error`.
#[derive(Debug)]
pub(crate) enum DoNewError {
    Validation(String),
    Conflict(String),
    SchemaViolation(String),
    SchemaConfig(String),
    Io(anyhow::Error),
}

impl From<DoNewError> for anyhow::Error {
    fn from(e: DoNewError) -> Self {
        match e {
            DoNewError::Io(e) => e,
            DoNewError::Validation(s)
            | DoNewError::Conflict(s)
            | DoNewError::SchemaConfig(s) => anyhow::Error::msg(s),
            DoNewError::SchemaViolation(s) => anyhow::Error::msg(format!("schema: {s}")),
        }
    }
}

fn cmd_new(json: bool, args: NewArgs) -> Result<()> {
    let root = find_root();
    let out = do_new(&root, args)?;
    if json {
        let report = serde_json::json!({
            "slug": out.slug,
            "title": out.title,
            "item_path": out.item_path.to_string_lossy(),
            "dir": out
                .item_path
                .parent()
                .map(|p| p.to_string_lossy().into_owned()),
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Created {}: {}", out.slug, out.title);
        println!("  {}", out.item_path.display());
    }
    Ok(())
}

pub(crate) fn do_new(root: &Path, args: NewArgs) -> Result<NewOutcome> {
    // M1 contract: every issuectl-mediated writer holds the repo
    // `flock`. Without this acquire, concurrent `issuectl new` from
    // the terminal would race against server-side mutations and
    // bypass the protocol's serialization guarantee.
    let lock = mutate::WriteLock::acquire(root)?;
    Ok(do_new_locked(&lock, root, args)?)
}

/// Body of `do_new` that assumes the caller holds the repo `WriteLock`.
/// Server-side `mutate::new_issue` uses this so it can hold the same
/// lock through the post-write parse + publish — without splitting the
/// sequence the synthetic `IssueUpserted` lands AFTER the lock is
/// released, inverting seq order against concurrent writers (C3).
pub(crate) fn do_new_locked(
    _lock: &mutate::WriteLock,
    root: &Path,
    args: NewArgs,
) -> std::result::Result<NewOutcome, DoNewError> {
    schema::ensure_default_written(root).map_err(DoNewError::Io)?;
    if args.issue_type == "epic" {
        if args.assignee.is_some() || args.reporter.is_some() {
            return Err(DoNewError::Validation(
                "epics use --owner, not --reporter/--assignee".into(),
            ));
        }
    } else if args.owner.is_some() {
        return Err(DoNewError::Validation(
            "--owner is only valid with --type epic".into(),
        ));
    }

    {
        // Reject `--field foo=a --field foo=b`. Silently letting the
        // last occurrence win is a reasonable default for many CLI
        // tools, but here it would mean the validated frontmatter and
        // the user's apparent intent diverge — better to fail loudly.
        let mut seen = std::collections::BTreeSet::new();
        for (k, _) in &args.custom_fields {
            if !seen.insert(k.as_str()) {
                return Err(DoNewError::Validation(format!(
                    "--field {k:?} given more than once"
                )));
            }
        }
    }

    let related = normalize_related_refs(&args.related)
        .map_err(|e| DoNewError::Validation(format!("{e:#}")))?;

    let new_args = write::NewIssueArgs {
        title: &args.title,
        issue_type: &args.issue_type,
        priority: &args.priority,
        reporter: args.reporter.as_deref(),
        assignee: args.assignee.as_deref(),
        owner: args.owner.as_deref(),
        epic: args.epic.as_deref(),
        labels: &args.labels,
        related: &related,
        source: args.source.as_deref(),
        description: args.description.as_deref(),
        custom_fields: &args.custom_fields,
    };
    // Build the frontmatter mapping and validate it BEFORE serializing.
    // Validating the in-memory Mapping avoids the round-trip through
    // string parsing that the previous version used (and that subtly
    // duplicated the fragile `find("\n---")` splitter logic).
    let frontmatter = write::build_new_frontmatter(&new_args);
    {
        let schema =
            schema::load(root).map_err(|e| DoNewError::SchemaConfig(format!("{e:#}")))?;
        let violations = schema::validate(&schema, &frontmatter);
        if !violations.is_empty() {
            let msg = violations
                .iter()
                .map(|v| v.message())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(DoNewError::SchemaViolation(msg));
        }
    }
    let render = write::render_new_item_from_fm(&new_args, &frontmatter);

    let issues_parent = root.join("issues");
    fs::create_dir_all(&issues_parent)
        .with_context(|| format!("cannot create {}", issues_parent.display()))
        .map_err(DoNewError::Io)?;

    // Pick a slug atomically: try `fs::create_dir` (which fails on
    // EEXIST) so two concurrent `issuectl new` invocations cannot race.
    // Post-flat-layout, the canonical home is `issues/<slug>/`.
    let (slug, dir) = match &args.slug {
        Some(s) => {
            let normalized = write::slugify(s, 10);
            if !slug::is_valid(&normalized) {
                return Err(DoNewError::Validation(format!(
                    "--slug {:?} normalized to {:?}, which is not a valid slug \
                     (need ≥2 lowercase ASCII kebab segments, optional digits)",
                    s, normalized
                )));
            }
            // Detect a pre-existing legacy copy of the slug so the
            // error message points at the migration command.
            let (_flat, legacy_open, legacy_closed) = repo::paths_for(root, &normalized);
            if legacy_open.exists() || legacy_closed.exists() {
                return Err(DoNewError::Conflict(format!(
                    "slug {normalized} already used at legacy path; run `issuectl doctor --fix` first"
                )));
            }
            let dir = issues_parent.join(&normalized);
            match fs::create_dir(&dir) {
                Ok(()) => (normalized, dir),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(DoNewError::Conflict(format!(
                        "target directory already exists: {}",
                        dir.display()
                    )));
                }
                Err(e) => {
                    return Err(DoNewError::Io(
                        anyhow::Error::from(e).context(format!("cannot create {}", dir.display())),
                    ));
                }
            }
        }
        None => claim_random_slug(root, &issues_parent).map_err(DoNewError::Io)?,
    };

    let item_path = dir.join("item.md");
    // `create_new(true)` is belt-and-braces here: the directory is
    // already exclusively ours, but if a caller somehow seeds an
    // `item.md` between the rename and write, we fail loudly.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&item_path)
            .with_context(|| format!("cannot create {}", item_path.display()))
            .map_err(DoNewError::Io)?;
        f.write_all(render.as_bytes())
            .with_context(|| format!("cannot write {}", item_path.display()))
            .map_err(DoNewError::Io)?;
    }

    Ok(NewOutcome {
        slug,
        title: args.title,
        item_path,
    })
}

/// Generate a random slug and atomically claim its flat directory.
/// Loops on `EEXIST` so that two concurrent processes that happen to
/// pick the same slug both retry rather than silently overwriting.
fn claim_random_slug(root: &Path, issues_parent: &Path) -> Result<(String, PathBuf)> {
    for _ in 0..16 {
        let candidate = slug::generate();
        // Cheap pre-check: skip slugs that already exist at any path
        // (flat or legacy) to avoid burning a random pick when the
        // answer is obvious.
        let (_flat, legacy_open, legacy_closed) = repo::paths_for(root, &candidate);
        if legacy_open.exists() || legacy_closed.exists() {
            continue;
        }
        let dir = issues_parent.join(&candidate);
        match fs::create_dir(&dir) {
            Ok(()) => return Ok((candidate, dir)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(
                    anyhow::Error::from(e).context(format!("cannot create {}", dir.display()))
                )
            }
        }
    }
    bail!("could not claim a unique slug after 16 attempts; wordlist exhausted?")
}

#[derive(Default)]
pub(crate) struct UpdateArgs {
    pub slug: String,
    pub status: Option<String>,
    pub assignee: Option<String>,
    pub owner: Option<String>,
    pub priority: Option<String>,
    pub epic: Option<String>,
    pub no_epic: bool,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
    pub add_related: Vec<String>,
    pub remove_related: Vec<String>,
    pub add_commits: Vec<String>,
    pub custom_fields: Vec<(String, String)>,
    pub clear_fields: Vec<String>,
    pub expected_version: Option<String>,
}

pub(crate) struct UpdateOutcome {
    pub final_dir: PathBuf,
    pub moved_to_closed: bool,
    pub moved_to_open: bool,
    pub version: String,
}

fn cmd_update(json: bool, args: UpdateArgs) -> Result<()> {
    if json && args.expected_version.is_none() {
        bail!("--expected-version is required with --json (per design D4=B); fetch with `issuectl show <slug> --json`");
    }
    let root = find_root();
    let slug = args.slug.clone();
    let out = do_update(&root, args)?;
    if json {
        let report = serde_json::json!({
            "slug": slug,
            "final_dir": out.final_dir.to_string_lossy(),
            "moved_to_closed": out.moved_to_closed,
            "moved_to_open": out.moved_to_open,
            "version": out.version,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if out.moved_to_closed {
        println!(
            "Updated {slug}: closing status set ({})",
            out.final_dir.display()
        );
    } else if out.moved_to_open {
        println!("Updated {slug}: re-opened ({})", out.final_dir.display());
    } else {
        println!("Updated {slug}");
    }
    Ok(())
}

pub(crate) fn do_update(root: &Path, args: UpdateArgs) -> Result<UpdateOutcome> {
    use mutate::Patch;
    let mut req = mutate::UpdateIssueRequest {
        expected_version: args.expected_version,
        ..Default::default()
    };
    if let Some(s) = args.status {
        req.status = Patch::Set(s);
    }
    if let Some(a) = args.assignee {
        req.assignee = Patch::Set(a);
    }
    if let Some(o) = args.owner {
        req.owner = Patch::Set(o);
    }
    if let Some(p) = args.priority {
        req.priority = Patch::Set(p);
    }
    if let Some(e) = args.epic {
        req.epic = Patch::Set(e);
    } else if args.no_epic {
        req.epic = Patch::Clear;
    }
    req.add_labels = args.add_labels;
    req.remove_labels = args.remove_labels;
    req.add_related = args.add_related;
    req.remove_related = args.remove_related;
    req.add_commits = args
        .add_commits
        .iter()
        .map(|spec| {
            let (hash, summary) = parse_commit_spec(spec)?;
            Ok::<_, anyhow::Error>(mutate::CommitSpec { hash, summary })
        })
        .collect::<Result<_, _>>()?;

    // `--field foo=a --field foo=b` collapses silently in a BTreeMap;
    // reject it explicitly so the user gets a precise error rather
    // than the last-wins surprise. `cmd_new` enforces the same
    // invariant — match its `BTreeSet<&str>` shape (no allocations,
    // deterministic overlap iteration).
    let mut seen_fields: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for (k, _) in &args.custom_fields {
        if !seen_fields.insert(k.as_str()) {
            bail!("--field {k:?} given more than once");
        }
    }
    let mut seen_clears: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for k in &args.clear_fields {
        if !seen_clears.insert(k.as_str()) {
            bail!("--clear-field {k:?} given more than once");
        }
    }
    if let Some(overlap) = seen_fields.intersection(&seen_clears).next() {
        bail!("field {overlap:?} appears in both --field and --clear-field");
    }
    for (k, v) in args.custom_fields {
        req.custom_fields.insert(k, mutate::Patch::Set(v));
    }
    for k in args.clear_fields {
        req.custom_fields.insert(k, mutate::Patch::Clear);
    }

    let outcome =
        mutate::update_issue(root, &args.slug, req, None).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(UpdateOutcome {
        final_dir: outcome.issue_dir,
        moved_to_closed: outcome.moved_to_closed,
        moved_to_open: outcome.moved_to_open,
        version: outcome.version,
    })
}

fn cmd_close(
    json: bool,
    slug: &str,
    status: Option<String>,
    commits: Vec<String>,
    expected_version: Option<String>,
) -> Result<()> {
    if json && expected_version.is_none() {
        bail!("--expected-version is required with --json (per design D4=B); fetch with `issuectl show <slug> --json`");
    }
    let root = find_root();
    let out = do_close(&root, slug, status, commits, expected_version)?;
    if json {
        let report = serde_json::json!({
            "slug": slug,
            "final_dir": out.final_dir.to_string_lossy(),
            "moved_to_closed": out.moved_to_closed,
            "version": out.version,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if out.moved_to_closed {
        println!("Closed {slug} ({})", out.final_dir.display());
    } else {
        println!("Updated {slug}");
    }
    Ok(())
}

pub(crate) fn do_close(
    root: &Path,
    slug: &str,
    status: Option<String>,
    commits: Vec<String>,
    expected_version: Option<String>,
) -> Result<UpdateOutcome> {
    // M4: read+decide+mutate now happen atomically inside
    // `mutate::close_issue` under one flock. The previous read-then-call
    // pattern was racy — a concurrent writer could flip the status or
    // type between the unlocked read here and the locked update later.
    let commit_specs = commits
        .iter()
        .map(|spec| {
            let (hash, summary) = parse_commit_spec(spec)?;
            Ok::<_, anyhow::Error>(mutate::CommitSpec { hash, summary })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let outcome = mutate::close_issue(root, slug, status, commit_specs, expected_version, None)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(UpdateOutcome {
        final_dir: outcome.issue_dir,
        moved_to_closed: outcome.moved_to_closed,
        moved_to_open: outcome.moved_to_open,
        version: outcome.version,
    })
}

/// Locate an issue by slug. Returns (folder, item.md path) where
/// `folder` is the kanban-bucket label derived from frontmatter status.
/// Delegates to `repo::locate_issue`, which handles flat layout plus
/// legacy compat reads.
pub fn locate_issue(root: &Path, slug: &str) -> Result<(String, PathBuf)> {
    repo::locate_issue(root, slug)
}

fn parse_commit_spec(spec: &str) -> Result<(String, String)> {
    let (hash, summary) = spec
        .split_once(':')
        .with_context(|| format!("commit spec must be HASH:summary, got {spec:?}"))?;
    let hash = hash.trim();
    let summary = summary.trim();
    if hash.is_empty() || summary.is_empty() {
        bail!("commit spec must be HASH:summary, got {spec:?}");
    }
    Ok((hash.to_string(), summary.to_string()))
}

/// Normalize a `--related`/`--add-related` reference. Accepts `@slug`, bare
/// `slug`, or legacy `#NN`. Output is canonical `@slug` form (or `#NN` if the
/// input was numeric — preserved verbatim so doctor can detect and migrate).
fn normalize_related_refs(refs: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        let trimmed = r.trim();
        if trimmed.is_empty() {
            bail!("related reference cannot be empty");
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
                bail!("related reference {:?} looks like #NN but isn't numeric", r);
            }
            out.push(format!("#{rest}"));
            continue;
        }
        let stripped = trimmed.strip_prefix('@').unwrap_or(trimmed);
        if !slug::is_valid(stripped) {
            bail!(
                "related reference must be @slug or a kebab-case slug, got {:?}",
                r
            );
        }
        out.push(format!("@{stripped}"));
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn cmd_note(
    json: bool,
    slug: &str,
    author: &str,
    message: &str,
    decision: bool,
    agent_run: bool,
    dry_run: bool,
    expected_version: Option<String>,
) -> Result<()> {
    if json && expected_version.is_none() {
        bail!(
            "--expected-version is required with --json (per design D4=B); fetch with `issuectl show <slug> --json`"
        );
    }
    let section = if decision {
        body_sections::DECISIONS
    } else if agent_run {
        body_sections::AGENT_RUNS
    } else {
        body_sections::COMMENTS
    };
    let root = find_root();
    let outcome = mutate::note_issue(
        &root,
        slug,
        author,
        message,
        section,
        expected_version,
        None,
        dry_run,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    finish_mutation(json, slug, &outcome, dry_run, "Appended note to")
}

fn cmd_set(
    json: bool,
    slug: &str,
    field: &str,
    value: Option<String>,
    clear: bool,
    dry_run: bool,
    expected_version: Option<String>,
) -> Result<()> {
    if json && expected_version.is_none() {
        bail!(
            "--expected-version is required with --json (per design D4=B); fetch with `issuectl show <slug> --json`"
        );
    }
    use mutate::Patch;
    let mut req = mutate::UpdateIssueRequest {
        expected_version,
        dry_run,
        ..Default::default()
    };
    let patch = if clear {
        Patch::Clear
    } else {
        Patch::Set(value.expect("required by clap unless --clear"))
    };
    // Status is the one field whose `Patch::Clear` is rejected by the
    // mutate layer; for everything else we let the shared
    // validate()/under-lock path produce the canonical error message
    // so CLI and API agree byte-for-byte. Custom fields land in the
    // schema-validated `custom_fields` slot — reserved keys like
    // `labels` / `related` produce a hint pointing at the right verb.
    match field {
        "status" => req.status = patch,
        "priority" => req.priority = patch,
        "assignee" => req.assignee = patch,
        "owner" => req.owner = patch,
        "epic" => req.epic = patch,
        other => {
            req.custom_fields.insert(other.to_string(), patch);
        }
    }
    let root = find_root();
    let outcome =
        mutate::update_issue(&root, slug, req, None).map_err(|e| anyhow::anyhow!("{e}"))?;
    finish_mutation(json, slug, &outcome, dry_run, "Updated")
}

fn cmd_check(
    json: bool,
    slug: &str,
    task: &str,
    dry_run: bool,
    expected_version: Option<String>,
) -> Result<()> {
    if json && expected_version.is_none() {
        bail!(
            "--expected-version is required with --json (per design D4=B); fetch with `issuectl show <slug> --json`"
        );
    }
    let root = find_root();
    let outcome = mutate::toggle_checkbox(&root, slug, task, expected_version, None, dry_run)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    finish_mutation(json, slug, &outcome, dry_run, "Toggled checkbox in")
}

fn cmd_label(
    json: bool,
    slug: &str,
    op: LabelOp,
    label: &str,
    dry_run: bool,
    expected_version: Option<String>,
) -> Result<()> {
    if json && expected_version.is_none() {
        bail!(
            "--expected-version is required with --json (per design D4=B); fetch with `issuectl show <slug> --json`"
        );
    }
    let mut req = mutate::UpdateIssueRequest {
        expected_version,
        dry_run,
        ..Default::default()
    };
    match op {
        LabelOp::Add => req.add_labels.push(label.to_string()),
        LabelOp::Remove => req.remove_labels.push(label.to_string()),
    }
    let root = find_root();
    let outcome =
        mutate::update_issue(&root, slug, req, None).map_err(|e| anyhow::anyhow!("{e}"))?;
    finish_mutation(json, slug, &outcome, dry_run, "Updated labels for")
}

fn cmd_apply(json: bool, patch_path: &Path, dry_run: bool) -> Result<()> {
    let yaml_text = fs::read_to_string(patch_path)
        .with_context(|| format!("cannot read patch file {}", patch_path.display()))?;
    let mut yaml: serde_yaml::Value = serde_yaml::from_str(&yaml_text)
        .with_context(|| format!("cannot parse {} as YAML", patch_path.display()))?;
    let map = yaml
        .as_mapping_mut()
        .ok_or_else(|| anyhow::anyhow!("patch file must be a YAML mapping at the top level"))?;
    let slug = map
        .remove(serde_yaml::Value::String("slug".into()))
        .ok_or_else(|| anyhow::anyhow!("patch file must declare `slug:`"))?
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("`slug:` must be a string"))?
        .to_string();
    if !slug::is_valid(&slug) {
        bail!("invalid slug shape: {slug:?}");
    }
    // `dry_run` is `#[serde(skip)]` on UpdateIssueRequest, so a
    // user-supplied `dry_run: true` would otherwise fail with a generic
    // "unknown field" error from `deny_unknown_fields`. Catch it here
    // with a precise message — dry-run is a CLI execution mode, not a
    // patch field.
    if map.contains_key(serde_yaml::Value::String("dry_run".into())) {
        bail!("`dry_run` is a CLI flag; use `issuectl apply --dry-run`, not a patch field");
    }
    let mut req: mutate::UpdateIssueRequest = serde_yaml::from_value(yaml)
        .with_context(|| format!("cannot parse patch fields in {}", patch_path.display()))?;
    // Validate `--json` D4=B contract AFTER deserialization so we
    // catch `expected_version: null` and empty/whitespace strings —
    // a presence-only `map.contains_key` check passed `null` through,
    // which then deserialized to `None` and bypassed concurrency.
    if json {
        match req.expected_version.as_deref() {
            Some(v) if !v.trim().is_empty() && v.trim() == v => {}
            _ => bail!(
                "patch must include a non-empty `expected_version:` when invoked with --json \
                 (per design D4=B); fetch with `issuectl show <slug> --json`"
            ),
        }
    }
    req.dry_run = dry_run;
    let root = find_root();
    let outcome =
        mutate::update_issue(&root, &slug, req, None).map_err(|e| anyhow::anyhow!("{e}"))?;
    finish_mutation(json, &slug, &outcome, dry_run, "Applied patch to")
}

/// Shared CLI epilogue for the new mutation verbs. On `--dry-run`
/// prints a unified diff between the bytes mutate.rs captured under
/// the flock; otherwise prints (or emits the JSON envelope for) the
/// standard mutation response. `--json` emits the same envelope as
/// `update` (slug, final_dir, moved_to_closed, moved_to_open,
/// version), with `dry_run: true` and `diff` added when planning.
fn finish_mutation(
    json: bool,
    slug: &str,
    outcome: &mutate::UpdateOutcome,
    dry_run: bool,
    human_verb: &str,
) -> Result<()> {
    if dry_run {
        // Both halves were captured under the held flock in mutate.rs.
        // No disk reads here — reading "before" outside the lock would
        // race a concurrent writer and produce a misleading diff.
        let before = outcome.before_serialized.as_deref().unwrap_or("");
        let after = outcome.pending_serialized.as_deref().unwrap_or(before);
        let diff = render_unified_diff(before, after, &outcome.issue_dir);
        if json {
            let report = serde_json::json!({
                "slug": slug,
                "final_dir": outcome.issue_dir.to_string_lossy(),
                "version": outcome.version,
                "moved_to_closed": outcome.moved_to_closed,
                "moved_to_open": outcome.moved_to_open,
                "dry_run": true,
                "diff": diff,
            });
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print!("{diff}");
        }
        return Ok(());
    }
    if json {
        let report = serde_json::json!({
            "slug": slug,
            "final_dir": outcome.issue_dir.to_string_lossy(),
            "version": outcome.version,
            "moved_to_closed": outcome.moved_to_closed,
            "moved_to_open": outcome.moved_to_open,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{human_verb} {slug}");
    }
    Ok(())
}

/// Render a git-style unified diff between `before` and `after`. The
/// header path is rendered as `issues/<slug>/item.md` rather than the
/// absolute path so the output looks like a normal `git diff` rather
/// than `--- a//abs/path/...` (the leading double-slash from joining
/// `a/` with an absolute path).
fn render_unified_diff(before: &str, after: &str, issue_dir: &Path) -> String {
    if before == after {
        return String::new();
    }
    let rel = issue_dir
        .file_name()
        .map(|n| format!("issues/{}/item.md", n.to_string_lossy()))
        .unwrap_or_else(|| issue_dir.join("item.md").display().to_string());
    let diff = similar::TextDiff::from_lines(before, after);
    let header_old = format!("--- a/{rel}\n");
    let header_new = format!("+++ b/{rel}\n");
    let body = diff
        .unified_diff()
        .context_radius(3)
        .to_string();
    format!("{header_old}{header_new}{body}")
}

fn cmd_body_set(
    json: bool,
    slug: &str,
    stdin: bool,
    from_file: Option<PathBuf>,
    expected_version: Option<String>,
) -> Result<()> {
    if json && expected_version.is_none() {
        bail!(
            "--expected-version is required with --json (per design D4=B); fetch with `issuectl show <slug> --json`"
        );
    }
    if !stdin && from_file.is_none() {
        bail!("specify exactly one of --stdin or --from-file");
    }
    let body = if let Some(path) = from_file {
        fs::read_to_string(&path)
            .with_context(|| format!("cannot read body from {}", path.display()))?
    } else {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("cannot read body from stdin")?;
        buf
    };
    let root = find_root();
    let outcome = mutate::update_body(&root, slug, expected_version, body, None, false)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if json {
        let report = serde_json::json!({
            "slug": slug,
            "version": outcome.version,
            "issue_dir": outcome.issue_dir.to_string_lossy(),
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Updated body of {slug}");
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct MigrateMove {
    pub slug: String,
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug)]
pub(crate) struct MigrateConflict {
    pub slug: String,
    pub detail: String,
}

/// One-shot move of every legacy `issues/{open,closed}/<slug>/` to
/// `issues/<slug>/`. Held under the repo write lock so a concurrent
/// CLI/server mutation cannot race the migration.
///
/// Two-pass plan-then-execute: discover everything → classify into
/// `moves` and `conflicts` → if any conflict exists, return without
/// touching disk. Only when the plan is clean does the rename pass
/// run. This honours the docstring's all-or-nothing intent and matches
/// what reviewers and the JSON exit-code contract expect (C6, M7).
///
/// Skips legacy directory entries whose names don't pass `slug::is_valid`
/// — `issues/open/scratchwork` (or any non-kebab name) is reported as a
/// `MigrateConflict` rather than silently migrated to `issues/scratchwork`
/// (M6).
#[derive(Debug, Default)]
pub(crate) struct MigrateLayoutPlan {
    pub moves: Vec<(String, PathBuf, PathBuf)>,
    pub conflicts: Vec<MigrateConflict>,
}

/// Read-only plan: discover what would move and what conflicts. No renames.
/// Safe to call without the write lock — callers wanting consistency
/// should re-run after acquiring the lock.
pub(crate) fn plan_migrate_layout(root: &Path) -> Result<MigrateLayoutPlan> {
    let issues = root.join("issues");

    use std::collections::BTreeMap;
    let mut by_slug: BTreeMap<String, Vec<(PathBuf, &'static str)>> = BTreeMap::new();
    let mut conflicts = Vec::new();
    for legacy in ["open", "closed"] {
        let legacy_dir = issues.join(legacy);
        let Ok(rd) = fs::read_dir(&legacy_dir) else {
            continue;
        };
        for entry in rd.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !slug::is_valid(&name) {
                // M6: don't silently migrate non-slug-shaped names.
                conflicts.push(MigrateConflict {
                    slug: name.clone(),
                    detail: format!(
                        "{} is not a valid slug shape — rename or move out of issues/{} before migrating",
                        entry.path().display(),
                        legacy
                    ),
                });
                continue;
            }
            by_slug
                .entry(name)
                .or_default()
                .push((entry.path(), legacy));
        }
    }

    // Pass 2: classify. flat-exists OR multiple-legacy → conflict.
    let mut moves: Vec<(String, PathBuf, PathBuf)> = Vec::new();
    for (slug, locations) in by_slug {
        let dest = issues.join(&slug);
        if dest.exists() {
            conflicts.push(MigrateConflict {
                slug: slug.clone(),
                detail: format!(
                    "both flat ({}) and legacy ({}) exist",
                    dest.display(),
                    locations
                        .iter()
                        .map(|(p, _)| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
            continue;
        }
        if locations.len() > 1 {
            conflicts.push(MigrateConflict {
                slug: slug.clone(),
                detail: format!(
                    "slug exists in both legacy folders ({})",
                    locations
                        .iter()
                        .map(|(p, _)| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" and ")
                ),
            });
            continue;
        }
        let (src, _) = locations.into_iter().next().unwrap();
        moves.push((slug, src, dest));
    }

    // C6: all-or-nothing. If any conflict exists, the plan carries no
    // moves so callers don't accidentally execute a partial migration.
    if !conflicts.is_empty() {
        return Ok(MigrateLayoutPlan {
            moves: Vec::new(),
            conflicts,
        });
    }

    Ok(MigrateLayoutPlan { moves, conflicts })
}

/// Execute a previously-planned migration. Caller must hold the repo
/// write lock.
pub(crate) fn execute_migrate_layout_plan(
    root: &Path,
    moves: Vec<(String, PathBuf, PathBuf)>,
) -> Result<Vec<MigrateMove>> {
    let issues = root.join("issues");
    let mut migrated = Vec::new();
    for (slug, src, dest) in moves {
        fs::rename(&src, &dest)
            .with_context(|| format!("cannot rename {} → {}", src.display(), dest.display()))?;
        migrated.push(MigrateMove {
            slug,
            from: src,
            to: dest,
        });
    }

    // Best-effort: prune now-empty legacy parent dirs so the repo
    // doesn't keep ghost directories.
    for legacy in ["open", "closed"] {
        let p = issues.join(legacy);
        if p.is_dir() {
            let _ = fs::remove_dir(&p);
        }
    }

    Ok(migrated)
}

fn cmd_skill_install(agent: &str, force: bool) -> Result<()> {
    let agents = match agent {
        "claude" => vec![skill::Agent::Claude],
        "codex" => vec![skill::Agent::Codex],
        "all" => vec![skill::Agent::Claude, skill::Agent::Codex],
        other => bail!("unknown agent {other:?}; expected claude, codex, or all"),
    };
    let root = find_root();
    skill::install_skill(&root, &agents, force)
}

fn cmd_skill_print(agent: &str) -> Result<()> {
    let resolved = skill::Agent::from_str(agent)?;
    skill::print_skill(resolved)
}

// ── Display helpers ─────────────────────────────────────────────────────────

const TABLE_HEADERS: &[&str] = &["Slug", "Title", "Type", "Status", "Pri", "Assignee"];

fn print_issue_table(issues: &[models::Issue]) {
    if issues.is_empty() {
        println!("No issues found.");
        return;
    }

    let rows: Vec<Vec<String>> = issues
        .iter()
        .map(|i| {
            vec![
                i.slug.clone(),
                truncate(&i.title, 50),
                i.issue_type.clone(),
                i.status.clone(),
                i.priority.clone(),
                i.effective_assignee().to_string(),
            ]
        })
        .collect();

    let mut widths: Vec<usize> = TABLE_HEADERS.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (j, cell) in row.iter().enumerate() {
            widths[j] = widths[j].max(cell.len());
        }
    }

    let header: String = TABLE_HEADERS
        .iter()
        .enumerate()
        .map(|(j, h)| format!("{:width$}", h, width = widths[j] + 1))
        .collect::<Vec<_>>()
        .join("");
    println!("{}", header.trim_end());

    let sep: String = widths
        .iter()
        .map(|w| "─".repeat(*w + 1))
        .collect::<Vec<_>>()
        .join("");
    println!("{}", sep.trim_end());

    for row in &rows {
        let line: String = row
            .iter()
            .enumerate()
            .map(|(j, cell)| format!("{:width$}", cell, width = widths[j] + 1))
            .collect::<Vec<_>>()
            .join("");
        println!("{}", line.trim_end());
    }

    println!("\n{} issue(s)", rows.len());
}

fn print_issue_detail(issue: &models::Issue) {
    println!("{}  {}", issue.slug, issue.title);
    println!("{}", "─".repeat(60));
    println!("Status:   {}  ({})", issue.status, issue.folder);
    println!("Type:     {}", issue.issue_type);
    println!("Priority: {}", issue.priority);
    if let Some(ref a) = issue.assignee {
        println!("Assignee: {}", a);
    }
    if let Some(ref o) = issue.owner {
        println!("Owner:    {}", o);
    }
    if let Some(ref r) = issue.reporter {
        println!("Reporter: {}", r);
    }
    if let Some(ref c) = issue.created {
        println!("Created:  {}", c);
    }
    if let Some(ref u) = issue.updated {
        println!("Updated:  {}", u);
    }
    if let Some(ref e) = issue.epic {
        println!("Epic:     @{}", e);
    }
    if let Some(ref lbs) = issue.labels {
        if !lbs.is_empty() {
            println!("Labels:   {}", lbs.join(", "));
        }
    }
    if let Some(ref rel) = issue.related {
        if !rel.is_empty() {
            println!("Related:  {}", rel.join(", "));
        }
    }
    if let Some(ref cl) = issue.closed {
        println!("Closed:   {}", cl);
    }
    if let Some(ref commits) = issue.commits {
        if !commits.is_empty() {
            println!("Commits:");
            for c in commits {
                println!("  {}  {}", c.hash, c.summary);
            }
        }
    }
    println!();
    println!("{}", issue.body);
}

fn truncate(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        format!("{}…", &text[..max_len.saturating_sub(1)])
    }
}

fn count_by_json<'a, F>(issues: &[&'a models::Issue], key_fn: F) -> serde_json::Value
where
    F: Fn(&'a models::Issue) -> &str,
{
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for issue in issues {
        let key = key_fn(issue).to_string();
        *counts.entry(key).or_insert(0) += 1;
    }
    serde_json::to_value(counts).unwrap_or_default()
}

fn print_counts<F>(header: &str, issues: &[&models::Issue], key_fn: F)
where
    F: Fn(&models::Issue) -> &str,
{
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for issue in issues {
        let key = key_fn(issue).to_string();
        *counts.entry(key).or_insert(0) += 1;
    }

    println!("{}", header);
    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
    for (key, count) in sorted {
        println!("  {:20} {}", key, count);
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fresh_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        tmp
    }

    fn new_args(t: &str, title: &str) -> NewArgs {
        NewArgs {
            issue_type: t.to_string(),
            title: title.to_string(),
            slug: None,
            reporter: None,
            assignee: None,
            owner: None,
            priority: "normal".to_string(),
            epic: None,
            labels: vec![],
            related: vec![],
            source: None,
            description: None,
            custom_fields: vec![],
        }
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn new_creates_random_slug_directory() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "First bug");
        args.reporter = Some("alice".into());
        args.assignee = Some("bob".into());
        let out = do_new(tmp.path(), args).unwrap();
        assert!(
            slug::is_valid(&out.slug),
            "{} should be valid slug",
            out.slug
        );
        assert!(out.item_path.exists());
        let content = read(&out.item_path);
        assert!(content.contains("type: bug"));
        assert!(content.contains("reporter: alice"));
        assert!(content.contains("assignee: bob"));
        assert!(content.contains("# First bug"));
    }

    #[test]
    fn new_honors_explicit_slug_override() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "Some Long Title");
        args.slug = Some("custom-thing".into());
        let out = do_new(tmp.path(), args).unwrap();
        assert_eq!(out.slug, "custom-thing");
        assert!(out.item_path.to_string_lossy().contains("/custom-thing/"));
    }

    #[test]
    fn new_rejects_unsluggable_explicit_slug() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "Title");
        args.slug = Some("!!!".into());
        assert!(do_new(tmp.path(), args).is_err());
    }

    #[test]
    fn new_rejects_existing_slug() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/taken")).unwrap();
        fs::write(
            tmp.path().join("issues/taken/item.md"),
            "---\nstatus: open\n---\n",
        )
        .unwrap();
        let mut args = new_args("bug", "Title");
        args.slug = Some("taken".into());
        assert!(do_new(tmp.path(), args).is_err());
    }

    #[test]
    fn new_rejects_epic_with_reporter() {
        let tmp = fresh_repo();
        let mut args = new_args("epic", "API v2");
        args.reporter = Some("alice".into());
        assert!(do_new(tmp.path(), args).is_err());
    }

    #[test]
    fn new_rejects_owner_for_non_epic() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "B");
        args.owner = Some("alice".into());
        assert!(do_new(tmp.path(), args).is_err());
    }

    #[test]
    fn new_creates_epic_with_owner() {
        let tmp = fresh_repo();
        let mut args = new_args("epic", "API v2 migration");
        args.owner = Some("cara".into());
        let out = do_new(tmp.path(), args).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("type: epic"));
        assert!(content.contains("owner: cara"));
    }

    #[test]
    fn new_normalizes_related_to_at_form() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "B");
        args.related = vec!["@extremely-quiet-otter".into(), "amber-loud-fox".into()];
        let out = do_new(tmp.path(), args).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("@extremely-quiet-otter"));
        assert!(content.contains("@amber-loud-fox"));
    }

    #[test]
    fn new_preserves_legacy_numeric_related() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "B");
        args.related = vec!["#7".into()];
        let out = do_new(tmp.path(), args).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("'#7'") || content.contains("\"#7\""));
    }

    #[test]
    fn update_sets_status_and_bumps_updated() {
        let tmp = fresh_repo();
        let mut a = new_args("bug", "Bug");
        a.slug = Some("my-test-slug".into());
        a.reporter = Some("rep".into());
        a.assignee = Some("ass".into());
        let n = do_new(tmp.path(), a).unwrap();
        do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                status: Some("in-progress".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(content.contains("status: in-progress"));
    }

    #[test]
    fn update_with_closing_status_does_not_move_directory() {
        let tmp = fresh_repo();
        let mut a = new_args("bug", "Bug");
        a.slug = Some("close-me".into());
        a.reporter = Some("r".into());
        a.assignee = Some("a".into());
        let n = do_new(tmp.path(), a).unwrap();
        let outcome = do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                status: Some("fixed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(outcome.moved_to_closed);
        // Flat layout: the directory does not change on close.
        assert_eq!(outcome.final_dir, n.item_path.parent().unwrap());
        let content = read(&n.item_path);
        assert!(content.contains("status: fixed"));
        assert!(content.contains("closed:"));
    }

    #[test]
    fn update_set_epic_replaces_value() {
        let tmp = fresh_repo();
        let mut a = new_args("task", "T");
        a.slug = Some("task-x".into());
        a.epic = Some("api-v2".into());
        let n = do_new(tmp.path(), a).unwrap();
        do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                epic: Some("api-v3".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(content.contains("epic: api-v3"));
    }

    #[test]
    fn update_no_epic_clears_field() {
        let tmp = fresh_repo();
        let mut a = new_args("task", "T");
        a.slug = Some("task-y".into());
        a.epic = Some("api-v2".into());
        let n = do_new(tmp.path(), a).unwrap();
        do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                no_epic: true,
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(!content.contains("epic:"));
    }

    #[test]
    fn close_defaults_to_fixed_for_bug() {
        let tmp = fresh_repo();
        let mut a = new_args("bug", "Bug");
        a.slug = Some("bug-slug".into());
        a.reporter = Some("r".into());
        a.assignee = Some("a".into());
        let n = do_new(tmp.path(), a).unwrap();
        let outcome = do_close(tmp.path(), &n.slug, None, vec![], None).unwrap();
        assert!(outcome.moved_to_closed);
        let content = read(&outcome.final_dir.join("item.md"));
        assert!(content.contains("status: fixed"));
    }

    #[test]
    fn close_defaults_to_done_for_task() {
        let tmp = fresh_repo();
        let mut a = new_args("task", "Task");
        a.slug = Some("task-slug".into());
        a.reporter = Some("r".into());
        a.assignee = Some("a".into());
        let n = do_new(tmp.path(), a).unwrap();
        let outcome = do_close(tmp.path(), &n.slug, None, vec![], None).unwrap();
        let content = read(&outcome.final_dir.join("item.md"));
        assert!(content.contains("status: done"));
    }

    #[test]
    fn close_rejects_already_closed() {
        let tmp = fresh_repo();
        let mut a = new_args("bug", "Bug");
        a.slug = Some("once-only".into());
        a.reporter = Some("r".into());
        a.assignee = Some("a".into());
        let n = do_new(tmp.path(), a).unwrap();
        do_close(tmp.path(), &n.slug, None, vec![], None).unwrap();
        assert!(do_close(tmp.path(), &n.slug, None, vec![], None).is_err());
    }

    #[test]
    fn locate_issue_finds_flat() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/foo-bar")).unwrap();
        fs::write(
            tmp.path().join("issues/foo-bar/item.md"),
            "---\nstatus: open\n---\n",
        )
        .unwrap();
        let (folder, _) = locate_issue(tmp.path(), "foo-bar").unwrap();
        assert_eq!(folder, "open");
        assert!(locate_issue(tmp.path(), "missing").is_err());
    }

    #[test]
    fn locate_issue_finds_legacy_path() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/closed/old-fox-here")).unwrap();
        fs::write(
            tmp.path().join("issues/closed/old-fox-here/item.md"),
            "---\nstatus: fixed\n---\n",
        )
        .unwrap();
        let (folder, item) = locate_issue(tmp.path(), "old-fox-here").unwrap();
        assert_eq!(folder, "closed");
        assert!(item
            .to_string_lossy()
            .contains("issues/closed/old-fox-here"));
    }

    #[test]
    fn parse_commit_spec_basic() {
        assert_eq!(
            parse_commit_spec("abc123:fix login").unwrap(),
            ("abc123".to_string(), "fix login".to_string())
        );
    }

    #[test]
    fn parse_commit_spec_rejects_no_colon() {
        assert!(parse_commit_spec("abc123 fix").is_err());
    }

    #[test]
    fn normalize_related_accepts_at_and_bare_slug() {
        assert_eq!(
            normalize_related_refs(&["@extremely-quiet-otter".to_string()]).unwrap(),
            vec!["@extremely-quiet-otter".to_string()]
        );
        assert_eq!(
            normalize_related_refs(&["amber-loud-fox".to_string()]).unwrap(),
            vec!["@amber-loud-fox".to_string()]
        );
    }

    #[test]
    fn normalize_related_preserves_legacy_numeric() {
        assert_eq!(
            normalize_related_refs(&["#7".to_string()]).unwrap(),
            vec!["#7".to_string()]
        );
    }

    #[test]
    fn normalize_related_rejects_garbage() {
        assert!(normalize_related_refs(&["not a slug".to_string()]).is_err());
        assert!(normalize_related_refs(&["@".to_string()]).is_err());
        assert!(normalize_related_refs(&["#abc".to_string()]).is_err());
        assert!(normalize_related_refs(&["foo".to_string()]).is_err()); // no hyphen
    }

    #[test]
    fn parse_non_empty_rejects_empty_and_padded() {
        assert!(parse_non_empty("").is_err());
        assert!(parse_non_empty("  ").is_err());
        assert!(parse_non_empty(" a").is_err());
    }

    #[test]
    fn new_writes_default_schema_on_first_use() {
        let tmp = fresh_repo();
        assert!(!tmp.path().join("issues/.schema.yaml").exists());
        let args = new_args("bug", "First bug");
        do_new(tmp.path(), args).unwrap();
        assert!(
            tmp.path().join("issues/.schema.yaml").is_file(),
            "schema file should be auto-written on first new"
        );
    }

    #[test]
    fn new_rejects_when_custom_required_field_missing() {
        let tmp = fresh_repo();
        // Pre-write a schema demanding a `team` field. Without `--field`
        // creation must fail loudly rather than silently producing an
        // invalid issue.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let res = do_new(tmp.path(), new_args("bug", "Will fail"));
        let err = res.err().expect("schema-required field missing should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("schema") && msg.contains("team"),
            "expected schema/team in error, got {msg:?}"
        );
    }

    #[test]
    fn new_with_field_satisfies_custom_required_field() {
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n    enum: [payments, infra]\n",
        )
        .unwrap();
        let mut args = new_args("bug", "With team");
        args.custom_fields = vec![("team".into(), "payments".into())];
        let out = do_new(tmp.path(), args).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("team: payments"));
    }

    #[test]
    fn new_rejects_field_outside_schema_enum() {
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n    enum: [payments, infra]\n",
        )
        .unwrap();
        let mut args = new_args("bug", "Bad team");
        args.custom_fields = vec![("team".into(), "marketing".into())];
        let err = do_new(tmp.path(), args).err().unwrap();
        let msg = err.to_string();
        assert!(
            msg.contains("schema") && msg.contains("team") && msg.contains("marketing"),
            "expected schema/team/marketing in error, got {msg:?}"
        );
    }

    #[test]
    fn parse_custom_field_rejects_built_in_keys() {
        // Built-in keys must use their dedicated flags so we don't
        // shadow validation done by clap (e.g. `--field type=garbage`).
        for k in ["type", "title", "slug", "status", "priority"] {
            let s = format!("{k}=foo");
            assert!(parse_custom_field(&s).is_err(), "{k} must be rejected");
        }
    }

    #[test]
    fn new_rejects_duplicate_field_keys() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "Dup");
        args.custom_fields = vec![
            ("team".into(), "a".into()),
            ("team".into(), "b".into()),
        ];
        let err = do_new(tmp.path(), args).err().unwrap();
        let msg = err.to_string();
        assert!(
            msg.contains("team") && msg.contains("more than once"),
            "expected duplicate-rejection, got {msg:?}"
        );
    }

    #[test]
    fn parse_custom_field_message_points_at_real_flag() {
        // Round-2 review: previous message hardcoded `--<key>` for keys
        // (`commits`, `closed`) that have no matching flag. The hint
        // table now points at the real flag or behavior.
        let err = parse_custom_field("commits=foo").unwrap_err();
        assert!(
            err.contains("--add-commit"),
            "expected --add-commit hint, got {err:?}"
        );
        let err = parse_custom_field("closed=foo").unwrap_err();
        assert!(
            err.contains("status") || err.contains("closing"),
            "expected status/closing hint, got {err:?}"
        );
    }

    #[test]
    fn parse_custom_field_accepts_kebab_and_underscore() {
        assert!(parse_custom_field("team=payments").is_ok());
        assert!(parse_custom_field("team-name=payments").is_ok());
        assert!(parse_custom_field("severity_level=p1").is_ok());
        assert!(parse_custom_field("=payments").is_err());
        assert!(parse_custom_field("team=").is_err());
        assert!(parse_custom_field("team:payments").is_err());
    }

    #[test]
    fn parse_custom_field_rejects_padded_input() {
        // Aligns with `parse_non_empty`'s reject-padding policy so
        // `--field` and `--clear-field` don't silently strip whitespace
        // the user did not intend.
        assert!(parse_custom_field(" team=payments").is_err());
        assert!(parse_custom_field("team =payments").is_err());
        assert!(parse_custom_field("team= payments").is_err());
        assert!(parse_custom_field("team=payments ").is_err());
    }

    #[test]
    fn parse_custom_field_key_accepts_valid_keys_and_rejects_built_ins() {
        assert!(parse_custom_field_key("team").is_ok());
        assert!(parse_custom_field_key("team-name").is_ok());
        assert!(parse_custom_field_key("severity_level").is_ok());

        for (k, _) in mutate::RESERVED_CUSTOM_FIELD_KEYS {
            assert!(
                parse_custom_field_key(k).is_err(),
                "{k} must be rejected as built-in"
            );
        }

        assert!(parse_custom_field_key("").is_err());
        assert!(parse_custom_field_key(" team").is_err(), "padded key");
        assert!(parse_custom_field_key("team ").is_err(), "padded key");
        assert!(parse_custom_field_key("bad key").is_err());
        assert!(parse_custom_field_key("team:name").is_err());
    }

    #[test]
    fn is_closing_status_classifies_correctly() {
        for s in CLOSING_STATUSES {
            assert!(is_closing_status(s));
        }
        for s in ACTIVE_STATUSES {
            assert!(!is_closing_status(s));
        }
    }

    fn write_raw_issue(root: &Path, slug: &str, fm: &str, body: &str) {
        let dir = root.join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            format!("---\n{fm}---\n{body}"),
        )
        .unwrap();
    }

    #[test]
    fn ls_query_filters_by_status_and_label() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: in-progress\npriority: high\nassignee: alice\nlabels: [frontend]\n",
            "# Login is broken\n",
        );
        write_raw_issue(
            tmp.path(),
            "calm-bright-newt",
            "type: feature\nstatus: open\npriority: normal\nassignee: bob\nlabels: [wontfix]\n",
            "# Add export\n",
        );

        let issues = repo::load_issues(tmp.path());
        let q = query::parse("status:in-progress assignee:alice").unwrap();
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| query::matches(&q, i))
            .map(|i| i.slug.clone())
            .collect();
        assert_eq!(hits, vec!["amber-loud-fox".to_string()]);

        let q = query::parse("-label:wontfix").unwrap();
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| query::matches(&q, i))
            .map(|i| i.slug.clone())
            .collect();
        assert_eq!(hits, vec!["amber-loud-fox".to_string()]);
    }

    /// Regression for the `--status` flag scope: pre-query-engine
    /// `ls -s fixed` returned nothing without `--all`/`--closed`
    /// because the implicit "open only" filter ran first. The query
    /// engine must preserve that — flag-translation must not flip
    /// the default.
    #[test]
    fn ls_flag_status_still_scoped_to_open() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\n",
            "# Open\n",
        );
        write_raw_issue(
            tmp.path(),
            "calm-bright-newt",
            "type: bug\nstatus: fixed\nclosed: 2026-05-01\n",
            "# Closed\n",
        );

        let issues = repo::load_issues(tmp.path());

        // Reproduce the cmd_list folder-default rule for the
        // flag-only path: open-only unless --all or --closed.
        let mut q = query::Query::default();
        q.push(query::Term::Field {
            field: query::FieldName::Status,
            m: query::FieldMatch::Equals("fixed".to_string()),
            negated: false,
        });
        let folder_filter = Some("open"); // mirrors flag-only branch

        let hits: Vec<_> = issues
            .iter()
            .filter(|i| {
                folder_filter.map(|f| i.folder == f).unwrap_or(true) && query::matches(&q, i)
            })
            .map(|i| i.slug.clone())
            .collect();
        assert!(
            hits.is_empty(),
            "ls -s fixed must not surface closed issues without --all/--closed; got {hits:?}"
        );
    }

    /// `search -status:wontfix` should remain scoped to open. A
    /// negated status term is exclusion, not scope expansion.
    #[test]
    fn search_negated_status_does_not_expand_scope() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\n",
            "# Open\n",
        );
        write_raw_issue(
            tmp.path(),
            "calm-bright-newt",
            "type: bug\nstatus: done\nclosed: 2026-05-01\n",
            "# Done\n",
        );

        let issues = repo::load_issues(tmp.path());
        let q = query::parse("-status:wontfix").unwrap();

        let scope_expanded = q.has_positive_field(query::FieldName::Folder)
            || q.has_positive_field(query::FieldName::Status);
        assert!(!scope_expanded, "negation must not expand scope");

        let hits: Vec<_> = issues
            .iter()
            .filter(|i| {
                if !scope_expanded && i.folder != "open" {
                    return false;
                }
                query::matches(&q, i)
            })
            .map(|i| i.slug.clone())
            .collect();
        assert_eq!(hits, vec!["amber-loud-fox".to_string()]);
    }

    #[test]
    fn search_query_combines_text_and_field() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\n",
            "# Login deadlock\n\nUser hits flock contention.",
        );
        write_raw_issue(
            tmp.path(),
            "calm-bright-newt",
            "type: feature\nstatus: open\n",
            "# Just a deadlock-themed feature note.",
        );

        let issues = repo::load_issues(tmp.path());
        let q = query::parse("deadlock text:flock").unwrap();
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| query::matches(&q, i))
            .map(|i| i.slug.clone())
            .collect();
        assert_eq!(hits, vec!["amber-loud-fox".to_string()]);
    }
}
