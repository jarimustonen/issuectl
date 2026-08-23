use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::builder::PossibleValuesParser;
use clap::{ArgGroup, Command as ClapCommand, CommandFactory, Parser, Subcommand, ValueEnum};

use issuectl_core::issue_fields::{ISSUE_TYPES, PRIORITIES};
use issuectl_core::{
    agents, body_sections, canonical, config, context, cycle as cycle_mod, dag, doctor, duplicates,
    envelope, epic_tree, estimate as estimate_mod, fmt, git_trailers, help, hooks,
    init as init_cmd, merge_driver, models, mutate, patch_input, query, recurrence, repo,
    report as report_mod, schema, skill, slug, sync_commits,
};
use mutate::new_issue::{do_new, NewArgs};

static JSON_OUTPUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PENDING_WARNINGS: std::sync::OnceLock<std::sync::Mutex<Vec<serde_json::Value>>> =
    std::sync::OnceLock::new();

const DEPRECATED_SURFACE_REMOVAL_VERSION: &str = "0.18.0";

fn deprecation_warnings_suppressed() -> bool {
    std::env::var("ISSUECTL_NO_DEPRECATION_WARNINGS").as_deref() == Ok("1")
}

fn emit_deprecation_warning(
    json: bool,
    id: &str,
    deprecated: &str,
    replacement_argv: &[&str],
    additional_guidance: Option<&str>,
) {
    if deprecation_warnings_suppressed() {
        return;
    }
    let mut message = format!(
        "{deprecated} is deprecated and will be removed in issuectl {DEPRECATED_SURFACE_REMOVAL_VERSION}; use `{}` instead",
        replacement_argv.join(" ")
    );
    if let Some(guidance) = additional_guidance {
        message.push_str("; ");
        message.push_str(guidance);
    }
    if json {
        PENDING_WARNINGS
            .get_or_init(Default::default)
            .lock()
            .expect("warning mutex poisoned")
            .push(serde_json::json!({
                "id": id,
                "message": message,
                "replacement_argv": replacement_argv,
                "removal_version": DEPRECATED_SURFACE_REMOVAL_VERSION,
            }));
    } else {
        eprintln!("warning: {message}");
    }
}

/// Emit one canonical JSON success envelope when `--json` is active. Command
/// handlers still render their domain result normally, keeping dispatch thin;
/// this final output seam makes the envelope impossible to omit accidentally.
fn emit_stdout(value: String, newline: bool) {
    use std::io::Write;
    if JSON_OUTPUT.load(std::sync::atomic::Ordering::Relaxed) {
        // Text-mode renderers sometimes add a trailing `println!()` after a
        // JSON `print!()`. It is formatting only, never a second result.
        if value.is_empty() {
            return;
        }
        let data = serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value));
        let mut enveloped = envelope::success(&data).expect("JSON envelope must serialize");
        if let Some(pending) = PENDING_WARNINGS.get() {
            let mut pending = pending.lock().expect("warning mutex poisoned");
            if let Some(warnings) = enveloped["warnings"].as_array_mut() {
                warnings.append(&mut pending);
            }
        }
        let rendered =
            serde_json::to_string_pretty(&enveloped).expect("JSON envelope must serialize");
        let mut stdout = std::io::stdout().lock();
        let _ = newline;
        writeln!(stdout, "{rendered}").expect("stdout must be writable");
        stdout.flush().expect("stdout must be writable");
    } else {
        let mut stdout = std::io::stdout().lock();
        if newline {
            writeln!(stdout, "{value}").expect("stdout must be writable");
        } else {
            write!(stdout, "{value}").expect("stdout must be writable");
        }
        stdout.flush().expect("stdout must be writable");
    }
}

macro_rules! println {
    () => { emit_stdout(String::new(), true) };
    ($($arg:tt)*) => { emit_stdout(format!($($arg)*), true) };
}
macro_rules! print {
    ($($arg:tt)*) => { emit_stdout(format!($($arg)*), false) };
}

const TOP_LEVEL_HELP: &str = "\
Examples:
  issuectl ls                              List open issues
  issuectl ls -t bug -p high               Filter by type and priority
  issuectl ls --closed --json              Closed issues as JSON
  issuectl show extremely-quiet-otter          Full details by slug
  issuectl open extremely-quiet-otter          Edit item.md in $EDITOR
  issuectl search redirect                 Keyword search
  issuectl duplicates                      Flag likely-duplicate issue pairs
  issuectl create --type bug --title \"...\" --slug login-loop    Create a new issue
  issuectl update <slug> --status testing  Change status
  issuectl close <slug> --status fixed     Set a closing status (fixed/done/...)
  issuectl attach <slug> shot.png log.txt  Copy files into the issue's attachments/
  issuectl bulk \"label:stale\" --set status=wontfix  Mutate every matched issue
  issuectl cycle current                   Print current ISO-week cycle label
  issuectl cycle status                    Open/closed rollup for current cycle
  issuectl export json > issues.json       Export issues (json/markdown/csv)
  issuectl import json issues.json          Import issues from a JSON file
  issuectl import github --repo o/r         Import open GitHub issues via gh
  issuectl init                            Bootstrap a new repo (schema, agents, skill)
  issuectl config show                     Inspect effective schema configuration
  issuectl doctor                          Health-check the repo
  issuectl doctor --fix                    Migrate legacy numbered issues
  issuectl skill list                      List bundled companion skills
  issuectl skill install                   Install /issue (+ /issue-new, /issue-intake) skills
";

#[derive(Parser)]
#[command(
    name = "issuectl",
    version,
    about = "Manage markdown-based issues with frontmatter",
    after_help = TOP_LEVEL_HELP
)]
struct Cli {
    // Keep the very wide derived subcommand enum off the parser's stack. Clap
    // constructs the complete command tree before selecting a verb, and an
    // inline `Command` pushed Linux's normal 2 MiB test-thread stack over its
    // limit as the CLI grew.
    #[command(subcommand)]
    command: Box<Command>,

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

/// Parse a day count for `--older-than`: a bare integer (`90`) or a
/// `<N>d` suffix form (`90d`). Must be non-negative.
fn parse_days(s: &str) -> std::result::Result<i64, String> {
    let digits = s.strip_suffix('d').unwrap_or(s);
    let n: i64 = digits
        .parse()
        .map_err(|_| format!("expected a day count like `90` or `90d`, got {s:?}"))?;
    if n < 0 {
        return Err(format!("day count cannot be negative: {s:?}"));
    }
    Ok(n)
}

/// Resolve the "current user" for `:me`-style query terms. Falls
/// back through `$ISSUECTL_USER`, `$GIT_AUTHOR_NAME`,
/// `$GIT_COMMITTER_NAME`, and finally `git config user.name`. None
/// when nothing resolves — callers either bail (the query mentions
/// `:me`) or proceed with `:me` left as a literal that matches
/// nothing in practice. Whitespace is trimmed; empty results count
/// as "unresolved".
fn whoami() -> Option<String> {
    for var in ["ISSUECTL_USER", "GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    let out = std::process::Command::new("git")
        .args(["config", "--get", "user.name"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

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

fn parse_threshold(s: &str) -> std::result::Result<f64, String> {
    let v: f64 = s
        .parse()
        .map_err(|_| format!("threshold must be a number, got {s:?}"))?;
    if !(0.0..=1.0).contains(&v) {
        return Err(format!("threshold must be between 0.0 and 1.0, got {v}"));
    }
    Ok(v)
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
    mutate::validate_custom_field_key(key)?;
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
    mutate::validate_custom_field_key(s)?;
    Ok(s.to_string())
}

/// Clap value parser for `bulk --set <key=value>`. Unlike
/// `parse_custom_field`, this accepts built-in keys (`status`, `type`,
/// ...) so a bulk caller can set them through one uniform flag —
/// `cmd_bulk` routes built-ins to their typed slots and everything else
/// to the schema-validated custom-field slot. Shape and whitespace rules
/// match `parse_custom_field`; reserved-vs-custom semantics are enforced
/// downstream by the routing + `UpdateIssueRequest::validate`.
fn parse_bulk_set(s: &str) -> std::result::Result<(String, String), String> {
    let (key, value) = s
        .split_once('=')
        .ok_or_else(|| format!("--set expects key=value, got {s:?}"))?;
    if key.is_empty() {
        return Err(format!("--set key cannot be empty: {s:?}"));
    }
    if value.is_empty() {
        return Err(format!(
            "--set {key:?}: value cannot be empty (use --clear to remove the field)"
        ));
    }
    if key.trim() != key || value.trim() != value {
        return Err(format!(
            "--set {s:?} has leading or trailing whitespace; remove it"
        ));
    }
    if !mutate::is_valid_custom_field_key(key) {
        return Err(format!(
            "--set key {key:?} must be alphanumeric / underscore / hyphen"
        ));
    }
    reject_unroutable_reserved_key("--set", key)?;
    Ok((key.to_string(), value.to_string()))
}

/// Built-in single-value fields `bulk` routes through their typed slots
/// (so `--set status=done` gets closed-date handling, etc.). Any other
/// key is treated as a custom field.
fn is_bulk_routable_builtin(key: &str) -> bool {
    matches!(
        key,
        "status" | "type" | "priority" | "assignee" | "owner" | "epic"
    )
}

/// Reject reserved keys that `bulk --set`/`--clear` can't route, with a
/// hint pointing at the right flag. Routable built-ins (status, type,
/// priority, assignee, owner, epic) pass through; list-shaped built-ins
/// (`labels`/`related`) and auto-managed keys (`title`, `commits`,
/// dates, ...) are rejected here rather than silently landing in the
/// custom-field slot and erroring late with a vaguer message.
fn reject_unroutable_reserved_key(flag: &str, key: &str) -> std::result::Result<String, String> {
    if is_bulk_routable_builtin(key) {
        return Ok(key.to_string());
    }
    match key {
        "labels" => Err(format!(
            "{flag} {key:?} is built-in: use bulk --add-label / --remove-label"
        )),
        "related" => Err(format!(
            "{flag} {key:?} is built-in: use bulk --add-related / --remove-related"
        )),
        other => match mutate::reserved_custom_field_hint(other) {
            Some(hint) => Err(format!("{flag} {key:?} is built-in: {hint}")),
            None => Ok(key.to_string()),
        },
    }
}

/// Clap value parser for `bulk --clear <key>`. Bare-key counterpart of
/// [`parse_bulk_set`]; accepts built-in keys (routing rejects the ones
/// that can't be cleared, e.g. `status`/`type`).
fn parse_bulk_clear_key(s: &str) -> std::result::Result<String, String> {
    if s.is_empty() {
        return Err("--clear key cannot be empty".to_string());
    }
    if s.trim() != s {
        return Err(format!(
            "--clear key {s:?} has leading or trailing whitespace; remove it"
        ));
    }
    if !mutate::is_valid_custom_field_key(s) {
        return Err(format!(
            "--clear key {s:?} must be alphanumeric / underscore / hyphen"
        ));
    }
    reject_unroutable_reserved_key("--clear", s)?;
    Ok(s.to_string())
}

/// Clap value parser for any slug-shaped CLI argument. Rejects anything
/// that wouldn't pass [`slug::is_valid`], which closes the path-traversal
/// door for `Show/Update/Close <slug>` and keeps `--epic` / `--related`
/// in line with the canonical slug shape.
fn parse_slug_arg(s: &str) -> std::result::Result<String, String> {
    let s = parse_non_empty(s)?;
    // Accept either a canonical slug or a slug *prefix* (single segment
    // allowed). The central resolver (`repo::resolve_slug_input`,
    // called from `locate_issue_full`) expands a unique prefix to the
    // matching canonical slug, so `issuectl show extremely` works the
    // same as the full slug. Creation paths (`create --slug`, `rename
    // new_slug`) still apply the stricter `slug::is_valid` check
    // server-side so a single-segment input cannot land on disk.
    if !slug::is_valid_prefix(&s) {
        return Err(format!(
            "{s:?} is not a valid slug or slug prefix (lowercase ASCII letters/digits, kebab-case, no path separators)"
        ));
    }
    Ok(s)
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Frequently used issue-management commands.
    #[command(flatten)]
    Primary(Box<PrimaryCommand>),

    /// Repository tooling, reporting, and intake commands.
    #[command(flatten)]
    Extended(Box<ExtendedCommand>),
}

impl Command {
    #[cfg(test)]
    fn into_primary(self) -> Box<PrimaryCommand> {
        match self {
            Self::Primary(command) => command,
            Self::Extended(_) => panic!("expected a primary command"),
        }
    }
}

#[derive(Subcommand)]
pub(crate) enum PrimaryCommand {
    /// Print the running CLI and JSON-contract versions for drift audits.
    Version,

    /// Inspect the effective schema configuration and its sources
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

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

        /// Filter by status. Schema-aware — accepts any status declared
        /// in `issues/.schema.yaml`'s `status` enum, including project-
        /// added values like `archived`. Unknown statuses simply match
        /// nothing rather than erroring (filter semantics).
        #[arg(short = 's', long, value_parser = parse_non_empty)]
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

    /// Open an issue's `item.md` in your editor (or its directory with
    /// `--dir`). The editor is `--editor`, else `$VISUAL`, else `$EDITOR`.
    Open {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Open the issue directory instead of `item.md`
        #[arg(long)]
        dir: bool,

        /// Editor command to launch (e.g. `code`, `zed`, `vim`).
        /// Overrides `$VISUAL` / `$EDITOR`. May include arguments,
        /// e.g. `--editor "code -w"`.
        #[arg(long, value_parser = parse_non_empty)]
        editor: Option<String>,
    },

    /// Copy one or more files into an issue's `attachments/` directory,
    /// creating the directory on demand. Each FILE is copied under its
    /// basename; collisions are auto-renamed with a numeric suffix
    /// (`shot.png` → `shot-1.png`) so a batch attach never bails halfway.
    /// `--json` reports the per-file outcomes (including `renamed`).
    Attach {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// One or more source files to copy in
        #[arg(required = true, num_args = 1..)]
        files: Vec<PathBuf>,
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

    /// Report Definition-of-Done completion for an issue. Parses the
    /// canonical `## Acceptance Criteria`, `## Tests Run`, and
    /// `## Implementation Notes` sections, counts checked / unchecked
    /// task-list items, and exits 0 when the AC section is fully
    /// checked, 1 otherwise. Use `--json` for machine-readable output.
    Ready {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
    },

    /// Flag likely-duplicate issues using local heuristics (title,
    /// label, and body-token overlap — no remote AI). With a SLUG,
    /// reports issues similar to that one; without, scans all pairs.
    #[command(alias = "dups")]
    Duplicates {
        /// Score candidates against this issue only. Omit to scan every
        /// pair of issues.
        #[arg(value_parser = parse_slug_arg)]
        slug: Option<String>,

        /// Minimum similarity score to report, 0.0–1.0 (default 0.30).
        #[arg(long, value_parser = parse_threshold)]
        threshold: Option<f64>,

        /// Include closed issues in the candidate pool (default: open only).
        #[arg(long)]
        all: bool,
    },

    /// Create a new issue or epic.
    ///
    /// When neither `--slug` nor `--slug-random` is supplied, derive a descriptive 2-3 word kebab slug from the title; collisions get a numeric suffix. Use `--slug-random` to opt into a random `intensifier-adjective-noun` slug. Titles with no sensible slug fall back to random.
    #[command(visible_alias = "new")]
    #[command(group(clap::ArgGroup::new("title_input").required(true).multiple(false).args(["title_pos", "title_flag"])))]
    Create {
        /// Item type
        #[arg(short = 't', long = "type", value_parser = PossibleValuesParser::new(ISSUE_TYPES))]
        issue_type: String,

        /// Item title (markdown heading), as a positional argument —
        /// e.g. `create "Login loops" --type bug`. Mutually exclusive with
        /// `--title`; exactly one of the two is required.
        #[arg(value_name = "TITLE", value_parser = parse_non_empty)]
        title_pos: Option<String>,

        /// Item title (markdown heading), canonical flag form. Mutually
        /// exclusive with the positional `TITLE`; exactly one is required.
        #[arg(long = "title", value_name = "TITLE", value_parser = parse_non_empty)]
        title_flag: Option<String>,

        /// Explicit descriptive 2-3 word kebab-case slug (e.g. `login-redirect-loops`), authoritative when passed. Omit to auto-derive a kebab slug from the title (collisions get a numeric suffix); a title with no sensible slug falls back to a random one. Use `--slug-random` to force the random form
        #[arg(long, value_parser = parse_non_empty)]
        slug: Option<String>,

        /// Force a random `intensifier-adjective-noun` slug instead of the
        /// title-derived default. Use when the title would leak sensitive
        /// data (customer names, emails, secrets) into the directory /
        /// branch name, or when the derived slug just isn't wanted. Ignored
        /// if `--slug` is given (an explicit slug always wins).
        #[arg(long = "slug-random", conflicts_with = "slug")]
        slug_random: bool,

        /// Reporter username (issues only)
        #[arg(long, value_parser = parse_non_empty)]
        reporter: Option<String>,

        /// Assignee username (issues only)
        #[arg(short = 'a', long, value_parser = parse_non_empty)]
        assignee: Option<String>,

        /// Owner username (epics only)
        #[arg(long, value_parser = parse_non_empty)]
        owner: Option<String>,

        /// Priority: `low` (can wait), `normal` (the default), or `high`
        /// (jumps the queue). The schema ships these three and NOT
        /// `medium`/`critical` — keep triage cheap; `critical` incidents
        /// are handled out-of-band (paging, hotfix). Order is presentation
        /// only — no ranking/sorting is implied. Repos that need finer
        /// gradations can widen the enum in `issues/.schema.yaml`; see
        /// `docs/design/frontmatter-schema.md`.
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

        /// Set the scheduling lane at creation (see `issuectl dag`), so a
        /// new issue is born into the DAG in one call instead of a
        /// follow-up `update --lane`. Mirrors `update --lane`.
        #[arg(long, value_parser = parse_non_empty)]
        lane: Option<String>,

        /// Set the coarse intra-lane precedence key at creation (see
        /// `issuectl dag`); consulted after `blocked_by` and priority,
        /// before the slug tie-break. Mirrors `update --lane-seq`.
        #[arg(long = "lane-seq", allow_hyphen_values = true)]
        lane_seq: Option<i64>,

        /// Add a collision hot-file token at creation (repeatable).
        /// Mirrors `update --add-collision`.
        #[arg(long = "add-collision", value_parser = parse_non_empty)]
        add_collision: Vec<String>,

        /// Source line for the body (e.g. "frontend/login")
        #[arg(long, value_parser = parse_non_empty)]
        source: Option<String>,

        /// Description body (free text). `--body` is accepted as an alias.
        #[arg(long, visible_alias = "body", value_parser = parse_non_empty)]
        description: Option<String>,

        /// Read the initial body from a file, written below the
        /// `# <title>` heading. Pass `-` to read stdin (use `./-` for a
        /// file literally named `-`). Mutually exclusive with
        /// `--description`/`--body`. A body using a reserved legacy
        /// section heading (`## Notes` — use `## Comments`) is accepted
        /// but warns; `issuectl doctor --fix` migrates it later.
        #[arg(long = "body-file", conflicts_with = "description")]
        body_file: Option<PathBuf>,

        /// Set a custom frontmatter field (repeatable). Format `key=value`.
        /// Use this for fields the schema declares but no built-in flag
        /// covers (e.g. `--field team=payments`). Built-in fields use
        /// their dedicated flags (`--type`, `--priority`, ...).
        #[arg(long = "field", value_parser = parse_custom_field)]
        custom_fields: Vec<(String, String)>,

        /// Before creating, scan existing issues for a strong duplicate
        /// (same heuristics as `duplicates`). If one is found, abort
        /// without creating and print the matches; re-run without this
        /// flag to create anyway.
        #[arg(long = "check-duplicates")]
        check_duplicates: bool,

        /// Deprecated: create an inbox draft. Use `intake file`; existing
        /// inbox drafts are migrated by `doctor --fix`.
        #[arg(long, hide = true)]
        inbox: bool,
    },

    /// Selectively update one issue, a query result, or a YAML patch
    #[command(group(ArgGroup::new("update_target")
        .required(true)
        .multiple(false)
        .args(["slug", "patch_file", "query"])))]
    #[command(group(ArgGroup::new("update_fields")
        .required(false)
        .multiple(true)
        .args([
            "title", "status", "issue_type", "assignee", "no_reporter",
            "no_assignee", "owner", "no_owner", "priority", "epic", "no_epic",
            "lane", "no_lane", "lane_seq", "no_lane_seq", "add_collision",
            "remove_collision", "add_labels", "remove_labels", "add_related",
            "remove_related", "add_blocked_by", "remove_blocked_by", "add_commits",
            "custom_fields", "clear_fields", "description", "body_file",
            "expected_version"
        ])))]
    Update {
        /// Issue slug. Mutually exclusive with `--query` and `--patch-file`.
        #[arg(value_parser = parse_slug_arg)]
        slug: Option<String>,

        /// Apply an `apply`-compatible YAML/JSON patch in one transaction.
        /// Pass `-` to read stdin (use `./-` for a file literally named `-`).
        /// The parser, expected-version contract, body_ops, output, and
        /// errors are identical to `apply <path|->`.
        #[arg(
            long = "patch-file",
            value_name = "PATH|-",
            conflicts_with = "update_fields"
        )]
        patch_file: Option<PathBuf>,

        /// Select every issue matching this `bulk`-compatible query and
        /// apply the named update flags as one locked batch.
        #[arg(long, value_parser = parse_non_empty, allow_hyphen_values = true)]
        query: Option<String>,

        /// Plan a `--patch-file` or `--query` update without writing.
        #[arg(long, conflicts_with = "slug")]
        dry_run: bool,

        /// Rewrite the issue title stored in the markdown body's H1
        #[arg(long, value_parser = parse_non_empty)]
        title: Option<String>,

        /// New status (active or closing — frontmatter only, no directory move).
        /// Schema-aware: any status in `issues/.schema.yaml`'s `status` enum
        /// is accepted; final validation happens under-lock against the
        /// schema, where invalid statuses are rejected with the project's
        /// allowed-set spelled out.
        #[arg(short = 's', long, value_parser = parse_non_empty)]
        status: Option<String>,

        /// Change the issue type. A lone reporter automatically becomes the
        /// owner when changing to `epic`, with a warning; an assignee or
        /// conflicting owner is rejected with a runnable remediation command.
        /// Rejected with `MutateError::SchemaViolation` when the new type's
        /// schema-required body sections aren't already present (the user must
        /// add them first), and rejected when combined with a close→open reopen
        /// on the same call. Allowed values follow
        /// `issues/.schema.yaml` (`fields.type.enum`); CLI accepts any
        /// non-empty string and lets schema validation do the rejecting so
        /// repos that extend the type enum work end-to-end.
        #[arg(short = 't', long = "type", value_parser = parse_non_empty)]
        issue_type: Option<String>,

        /// New assignee (issues)
        #[arg(short = 'a', long, value_parser = parse_non_empty)]
        assignee: Option<String>,

        /// Remove the reporter
        #[arg(long)]
        no_reporter: bool,

        /// Remove the assignee
        #[arg(long, conflicts_with = "assignee")]
        no_assignee: bool,

        /// New owner (epics)
        #[arg(long, value_parser = parse_non_empty)]
        owner: Option<String>,

        /// Remove the owner
        #[arg(long, conflicts_with = "owner")]
        no_owner: bool,

        /// New priority
        #[arg(short = 'p', long, value_parser = PossibleValuesParser::new(PRIORITIES))]
        priority: Option<String>,

        /// Set parent epic slug
        #[arg(short = 'e', long, value_parser = parse_slug_arg)]
        epic: Option<String>,

        /// Remove the parent epic reference
        #[arg(long, conflicts_with = "epic")]
        no_epic: bool,

        /// Set the scheduling lane (see `issuectl dag`)
        #[arg(long, value_parser = parse_non_empty)]
        lane: Option<String>,

        /// Remove the scheduling lane
        #[arg(long, conflicts_with = "lane")]
        no_lane: bool,

        /// Set the coarse intra-lane precedence key (see `issuectl dag`);
        /// consulted after `blocked_by` and priority, before the slug tie-break
        #[arg(long = "lane-seq", allow_hyphen_values = true)]
        lane_seq: Option<i64>,

        /// Remove the lane_seq precedence key
        #[arg(long = "no-lane-seq", conflicts_with = "lane_seq")]
        no_lane_seq: bool,

        /// Add a collision hot-file token (repeatable)
        #[arg(long = "add-collision", value_parser = parse_non_empty)]
        add_collision: Vec<String>,

        /// Remove a collision hot-file token (repeatable)
        #[arg(long = "remove-collision", value_parser = parse_non_empty)]
        remove_collision: Vec<String>,

        /// Add a label (repeatable)
        #[arg(long = "add-label", value_parser = parse_non_empty)]
        add_labels: Vec<String>,

        /// Remove a label (repeatable)
        #[arg(long = "remove-label", value_parser = parse_non_empty)]
        remove_labels: Vec<String>,

        /// Add a related reference like `@<slug>` or a bare slug (repeatable)
        #[arg(long = "add-related", value_parser = parse_non_empty)]
        add_related: Vec<String>,

        /// Remove a related reference (repeatable)
        #[arg(long = "remove-related", value_parser = parse_non_empty)]
        remove_related: Vec<String>,

        /// Add a `blocked_by:` dependency edge like `@<slug>` or a bare slug
        /// (repeatable). Mirrors `depend add`; sets the DAG blocker edge.
        #[arg(long = "add-blocked-by", value_parser = parse_non_empty)]
        add_blocked_by: Vec<String>,

        /// Remove a `blocked_by:` dependency edge (repeatable)
        #[arg(long = "remove-blocked-by", value_parser = parse_non_empty)]
        remove_blocked_by: Vec<String>,

        /// Record a commit, format HASH:summary (repeatable)
        #[arg(long = "add-commit", value_parser = parse_non_empty)]
        add_commits: Vec<String>,

        /// Set a custom frontmatter field (repeatable). Format `key=value`.
        /// Mirrors `create --field`. Built-in fields use their dedicated
        /// flags (`--status`, `--priority`, ...).
        #[arg(long = "field", value_parser = parse_custom_field)]
        custom_fields: Vec<(String, String)>,

        /// Remove a custom frontmatter field (repeatable). Built-in fields
        /// have dedicated removal mechanics (e.g. `--no-epic`); use this
        /// only for keys the schema or client added beyond the built-in
        /// set.
        #[arg(long = "clear-field", value_parser = parse_custom_field_key)]
        clear_fields: Vec<String>,

        /// Replace the issue body with this free text. `--body` is accepted
        /// as an alias. Mirrors `create`'s `--description`/`--body`; the whole
        /// existing body is replaced (frontmatter is untouched). Mutually
        /// exclusive with `--body-file`.
        #[arg(long, visible_alias = "body", value_parser = parse_non_empty)]
        description: Option<String>,

        /// Replace the issue body with the contents of a file. Pass `-` to
        /// read stdin (use `./-` for a file literally named `-`). Mirrors
        /// `new`'s `--body-file`; mutually exclusive with
        /// `--description`/`--body`. A body using a reserved legacy section
        /// heading (`## Notes` — use `## Comments`) is accepted but warns;
        /// `issuectl doctor --fix` migrates it later.
        #[arg(long = "body-file", conflicts_with = "description")]
        body_file: Option<PathBuf>,

        /// Optimistic-concurrency token from a prior `show`/`list --json`.
        /// Optional in both modes (opt-in compare-and-swap): pass it only
        /// when you want the write to fail on a version mismatch; it is
        /// enforced when passed. `flock` prevents corruption regardless.
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },

    /// Set a closing status (frontmatter only; flat layout has no directory move)
    Close {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Closing status (default: `fixed` for bugs, `done` otherwise).
        /// Schema-aware: a project that declares a custom closing status
        /// in `issues/.schema.yaml` (`status_classes: { archived: closing }`)
        /// can pass it here. Schema validation under-lock rejects values
        /// outside the project's `status` enum.
        #[arg(short = 's', long, value_parser = parse_non_empty)]
        status: Option<String>,

        /// Optional closer attribution, recorded as the `closed_by:`
        /// frontmatter field (e.g. `alice` or `agent-name`). Same author
        /// grammar as `note --as`, but optional here — omit it to close
        /// without recording who made the call.
        #[arg(long = "as", value_parser = parse_non_empty)]
        author: Option<String>,

        /// Closing rationale, appended as a timestamped block under a
        /// `## Resolution` section in one step (no separate `note` +
        /// commit). `--note` (compatibility) and `--message` are aliases.
        /// `--comment` / `--message` are the shared note body vocabulary.
        /// Attributed to `--as` when given, else recorded under `@issuectl`.
        #[arg(
            long = "comment",
            visible_aliases = ["note", "message"],
            value_parser = parse_non_empty
        )]
        comment: Option<String>,

        /// Record a commit, format HASH:summary (repeatable)
        #[arg(long = "commit", value_parser = parse_non_empty)]
        commits: Vec<String>,

        /// After closing, rewrite the current HEAD commit's message to
        /// append a `Fixes-Issue: @<slug>` trailer, so the trailer-driven
        /// `issuectl changelog` picks the landing commit up with zero
        /// manual trailer discipline. Run this AFTER committing the fix
        /// (it stamps whatever HEAD is) and BEFORE pushing/merging
        /// (rewriting changes HEAD's sha). Message-only: the tree,
        /// author, and dates are preserved and the index is left alone.
        /// Skipped — never fails the close — when HEAD is detached, a
        /// merge commit, signed, mid rebase/cherry-pick/merge/revert, or
        /// there is no commit to stamp. Cannot combine with a `--commit`
        /// that points at HEAD (the rewrite would orphan that reference).
        #[arg(long = "stamp")]
        stamp: bool,

        /// Optimistic-concurrency token; same semantics as `update`.
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },

    /// Rename an issue's slug, rewriting every reference across the repo:
    /// the on-disk directory plus `epic:` / `related:` / `blocked_by:`
    /// frontmatter refs and `@slug` body mentions in all other issues.
    /// After a manual `mv`, `issuectl doctor` flags the now-dangling refs.
    Rename {
        /// Current slug
        #[arg(value_parser = parse_slug_arg)]
        old_slug: String,

        /// New slug (must be free and a valid kebab-case slug)
        #[arg(value_parser = parse_slug_arg)]
        new_slug: String,

        /// Report what would change without writing anything
        #[arg(long)]
        dry_run: bool,
    },

    /// List active issues that have gone stale — no frontmatter `updated`
    /// bump and no commit touching their `item.md` within the window.
    /// Long-running `in-progress` issues are flagged. Read-only.
    Stale {
        /// Staleness threshold in days (default 30). Accepts a bare
        /// number or a `<N>d` suffix.
        #[arg(long, value_parser = parse_days, default_value = "30d")]
        days: i64,
    },

    /// Move old closed issues into cold storage at
    /// `issues/archive/YYYY/MM/<slug>/`, keeping the active tree small.
    /// Archived issues remain readable by `show`/`list`/queries.
    Archive {
        /// Only archive issues closed at least this many days ago
        /// (default 90). Accepts a bare number or a `<N>d` suffix.
        #[arg(long = "older-than", value_parser = parse_days, default_value = "90d")]
        older_than: i64,

        /// Report what would move without touching disk.
        #[arg(long)]
        dry_run: bool,
    },

    /// Append a timestamped block to an issue's `## Comments` section
    /// (or `## Decisions` / `## Agent Runs` with `--decision` / `--agent-run`).
    /// Invokable as `comment` too.
    ///
    /// The note text comes from exactly one source — the positional
    /// argument, `--message`/`--body`/`--comment`, `--body-file PATH`
    /// (`-` = stdin), `--stdin`, or `--from-file PATH`. The `note_body` arg group makes
    /// clap enforce the at-most-one rule (passing two is a usage error);
    /// passing none is left to the handler (existing behavior/error).
    #[command(visible_alias = "comment")]
    #[command(group(clap::ArgGroup::new("note_body")
        .required(false)
        .multiple(false)
        .args(["message", "message_flag", "stdin", "from_file"])))]
    Note {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Author of the note (e.g. `alice` or `agent-name`)
        #[arg(long = "as", value_parser = parse_non_empty)]
        author: String,

        /// Note text (one positional argument; quote multi-word input).
        /// Back-compat form, equivalent to `--message`/`--body`. Mutually
        /// exclusive with the other body sources (`note_body` group).
        #[arg(value_parser = parse_non_empty)]
        message: Option<String>,

        /// Note text as a flag; `--body` and `--comment` are aliases.
        /// Mirrors `close --comment`/`--message` and `create --body`, so the
        /// whole family shares one vocabulary. Mutually exclusive with the
        /// positional body and the other body-source flags.
        #[arg(
            long = "message",
            visible_aliases = ["body", "comment"],
            value_parser = parse_non_empty
        )]
        message_flag: Option<String>,

        /// Read the note text from stdin instead of a positional argument
        #[arg(long)]
        stdin: bool,

        /// Read the note text from a file. `--body-file` is a visible
        /// alias (matching `create --body-file`), so both spellings name this
        /// one source. Pass `-` to read stdin (use `./-` for a file
        /// literally named `-`). Mutually exclusive with the other body
        /// sources; capped at 10 MiB.
        #[arg(long = "from-file", visible_alias = "body-file", value_name = "PATH")]
        from_file: Option<PathBuf>,

        /// Append to the `## Decisions` section instead of `## Comments`.
        #[arg(long, conflicts_with = "agent_run")]
        decision: bool,

        /// Append to the `## Agent Runs` section instead of `## Comments`.
        #[arg(long = "agent-run", conflicts_with = "decision")]
        agent_run: bool,

        /// Plan only: print a unified diff and exit 0 without writing.
        #[arg(long)]
        dry_run: bool,

        /// Optimistic-concurrency token; optional (opt-in compare-and-swap)
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

        /// Optimistic-concurrency token; optional (opt-in compare-and-swap)
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },

    /// Assign an issue to a user. Convenience wrapper around
    /// `set <slug> assignee <user>` — routes through the identical typed
    /// update path, with the same validation and idempotency. Use
    /// `--clear` (instead of a user) to unassign.
    Assign {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Username to assign. Required unless `--clear` is given.
        #[arg(value_parser = parse_non_empty, required_unless_present = "clear")]
        user: Option<String>,

        /// Unassign the issue instead of setting an assignee.
        #[arg(long, conflicts_with = "user")]
        clear: bool,

        /// Plan only: print a unified diff and exit 0 without writing.
        #[arg(long)]
        dry_run: bool,

        /// Optimistic-concurrency token; optional (opt-in compare-and-swap)
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

        /// Optimistic-concurrency token; optional (opt-in compare-and-swap)
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },

    /// Add or remove a label. Two equivalent forms: the positional
    /// `label <slug> add|remove <label>` and the flag form
    /// `label <slug> --add|--remove <label>`; supply exactly one. Re-running
    /// the same call is safe: a duplicate add is a no-op on labels (the list
    /// is deduped) and removing an absent label is a no-op too. Note that the
    /// `updated:` frontmatter date is still bumped on every call —
    /// idempotency here means "won't error / won't double the
    /// label," not "byte-identical file."
    ///
    /// The `label_target` group makes clap enforce "exactly one form" itself,
    /// so an incomplete invocation (bare `label <slug>`, or `op` with no
    /// `<label>`) is a proper clap usage error — exit 2 in human mode, the
    /// `usage-error` envelope under `--json` — never a late runtime failure.
    #[command(group(clap::ArgGroup::new("label_target")
        .required(true)
        .multiple(false)
        .args(["op", "add", "remove"])))]
    Label {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Operation (positional form): `add` or `remove`. Optional at the
        /// clap layer only so the `--add`/`--remove` flag form can stand in;
        /// when given it `requires` the positional `<label>`.
        #[arg(value_enum, requires = "label")]
        op: Option<LabelOp>,

        /// Label (positional form). Supplied with a positional `op`; the
        /// `op` → `label` `requires` edge makes clap demand it.
        #[arg(value_parser = parse_non_empty)]
        label: Option<String>,

        /// Flag form: add this label (alias for the positional `add <label>`).
        /// In the `label_target` group, so it is mutually exclusive with the
        /// positional `op` and with `--remove`; also conflicts with a
        /// positional `<label>`.
        #[arg(
            long,
            value_name = "LABEL",
            value_parser = parse_non_empty,
            conflicts_with = "label",
        )]
        add: Option<String>,

        /// Flag form: remove this label (alias for the positional
        /// `remove <label>`).
        #[arg(
            long,
            value_name = "LABEL",
            value_parser = parse_non_empty,
            conflicts_with = "label",
        )]
        remove: Option<String>,

        /// Plan only: print a unified diff and exit 0 without writing.
        #[arg(long)]
        dry_run: bool,

        /// Optimistic-concurrency token; optional (opt-in compare-and-swap)
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },

    /// Apply a multi-field YAML/JSON patch in a single transaction.
    /// Pass a file path, or `-` to read stdin; use `./-` for a file
    /// literally named `-`. Inline JSON argv is intentionally not accepted.
    /// The patch declares `slug:` plus any combination of built-in
    /// fields, `custom_fields:`, label/related list ops, commits,
    /// and `body_ops:` (`set_checkbox` / `append_note`), all applied
    /// under one flock with one schema-validation pass.
    ///
    /// Minimal `body_ops` patch covering every operation:
    ///
    /// ```yaml
    /// slug: my-issue
    /// body_ops:
    ///   - set_checkbox:
    ///       match: "tests passing"
    ///       checked: true
    ///   - append_note:
    ///       author: ci-bot
    ///       message: "all checks green"
    ///       section: agent_runs # optional; defaults to comments
    /// ```
    ///
    /// UNLIKE the single-field verbs (`update`/`set`/`note`/…), where
    /// `--expected-version` is optional under `--json`, this
    /// transactional patch still requires a non-empty `expected_version:`
    /// in the patch file when invoked with `--json`: a multi-field
    /// transaction assembled from an earlier `show` is exactly the
    /// read-modify-write shape a stale token protects.
    #[command(verbatim_doc_comment)]
    Apply {
        /// YAML/JSON patch file path, or `-` for stdin (`./-` names a literal file)
        #[arg(value_name = "PATH|-")]
        patch: PathBuf,

        /// Plan only: print a unified diff and exit 0 without writing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Apply one mutation to every issue matching a query, in a single
    /// batch. The query uses the same syntax as `ls`/`search`/`?q=`
    /// (e.g. `status:open label:wontfix`). Every matched issue is
    /// rewritten through the same validated path as `update`, so the
    /// result is one set of file changes the user commits together.
    /// `--dry-run` prints the affected slugs plus a per-issue diff and
    /// writes nothing.
    Bulk {
        /// Query selecting the issues to mutate. Same syntax as `ls`.
        /// No implicit open-only default — the query is authoritative,
        /// so an unqualified query can match closed issues too. Pass
        /// leading-hyphen negations as a single quoted argument:
        /// `bulk "-label:wontfix" --add-label triaged`.
        #[arg(value_parser = parse_non_empty, allow_hyphen_values = true)]
        query: String,

        /// Set a field to a value (repeatable). Format `key=value`.
        /// Built-in fields (`status`, `type`, `priority`, `assignee`,
        /// `owner`, `epic`) route through their typed slots; any other
        /// key is a schema-validated custom field. Use `--clear` to
        /// remove a field instead.
        #[arg(long = "set", value_parser = parse_bulk_set)]
        set: Vec<(String, String)>,

        /// Remove a field (repeatable). Built-in fields route through
        /// their typed slots (e.g. `--clear epic`); `status`/`type`
        /// cannot be cleared. Any other key clears a custom field.
        #[arg(long = "clear", value_parser = parse_bulk_clear_key)]
        clear: Vec<String>,

        /// Add a label to every matched issue (repeatable)
        #[arg(long = "add-label", value_parser = parse_non_empty)]
        add_labels: Vec<String>,

        /// Remove a label from every matched issue (repeatable)
        #[arg(long = "remove-label", value_parser = parse_non_empty)]
        remove_labels: Vec<String>,

        /// Add a related reference to every matched issue (repeatable)
        #[arg(long = "add-related", value_parser = parse_non_empty)]
        add_related: Vec<String>,

        /// Remove a related reference from every matched issue (repeatable)
        #[arg(long = "remove-related", value_parser = parse_non_empty)]
        remove_related: Vec<String>,

        /// Plan only: print affected slugs and a per-issue unified diff,
        /// writing nothing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Edit issue body markdown
    Body {
        #[command(subcommand)]
        action: BodyAction,
    },

    /// Bootstrap a new repo: schema scaffold, `.issuectl/AGENTS.md`,
    /// `/issue` skill (Claude + Codex by default), and optionally the
    /// pre-commit hook and YAML merge driver. Idempotent — safe to
    /// re-run on an already-initialized repo.
    Init {
        /// Which agent's skill format(s) to install
        #[arg(short = 'a', long, value_enum, default_value_t = AgentArg::All)]
        agent: AgentArg,

        /// Also install the opt-in pre-commit hook that runs
        /// `issuectl doctor` on staged issue files.
        #[arg(long)]
        with_hooks: bool,

        /// Also configure the `issuectl-yaml` git merge driver. You
        /// still need to add `issues/**/item.md merge=issuectl-yaml`
        /// to `.gitattributes` and commit it.
        #[arg(long)]
        with_merge_driver: bool,

        /// Overwrite existing per-step artifacts. For
        /// `.issuectl/AGENTS.md` only the schema-derived managed block
        /// is regenerated; user prose above the sentinels is preserved
        /// (use `issuectl agents init --force` to fully overwrite that
        /// file). For the merge driver, also bypasses the refusal to
        /// clobber an existing differing `merge.issuectl-yaml.driver`
        /// git config value.
        #[arg(long)]
        force: bool,
    },

    /// Health-check the repo and (with --fix) migrate legacy layouts and
    /// numbered issues to the canonical flat slug layout
    Doctor {
        /// Apply migrations and fixes (otherwise read-only report)
        #[arg(long)]
        fix: bool,
        /// Print every entry in long warning lists. By default,
        /// lists with more than a handful of entries collapse to a
        /// one-line count so "fix-something-rerun-doctor" loops
        /// don't refill the screen with the same warnings each
        /// iteration. (issue: @ridiculously-outrageous-fold)
        #[arg(long)]
        verbose: bool,
    },

    /// Install / uninstall the opt-in pre-commit hook that runs
    /// `issuectl doctor` on staged issue files
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },

    /// Walk git history and append commits to issue `commits[]`
    /// arrays based on `Refs-Issue:` / `Fixes-Issue:` trailers
    /// (with branch-name fallback). Idempotent.
    SyncCommits {
        /// Range expression passed to `git log` (e.g. `main..HEAD`,
        /// `<sha>..`). Defaults to `<merge-base of HEAD and
        /// main/master>..HEAD`; falls back to `HEAD` when no
        /// merge-base is found. NOTE: on `main` the merge-base is
        /// often `HEAD`, so the default collapses to an empty
        /// `HEAD..HEAD` and scans nothing — to record the last commit
        /// on main, pass `--range HEAD~1..HEAD` (or
        /// `--range origin/main..HEAD` before pushing). An empty
        /// default range is surfaced as a warning.
        #[arg(long)]
        range: Option<String>,

        /// Disable branch-name fallback. By default, commits with
        /// no trailer on a branch named after a known slug (or
        /// `<prefix>/<slug>` / `<prefix>-<slug>`) are attributed
        /// to that slug.
        #[arg(long = "no-branch-fallback")]
        no_branch_fallback: bool,

        /// Print the planned mutations without writing.
        #[arg(long = "dry-run")]
        dry_run: bool,
    },

    /// Manage `.issuectl/AGENTS.md` — durable, repo-local policy file
    /// that AI agents read by convention. Distinct from
    /// `issuectl prompt` (per-issue prompt rendering): this is policy,
    /// not ephemeral prompt content.
    Agents {
        #[command(subcommand)]
        action: AgentsAction,
    },
}

#[derive(Subcommand)]
pub(crate) enum ExtendedCommand {
    /// Install or preview the /issue skill template (Claude Code or Codex)
    Skill {
        #[command(subcommand)]
        action: SkillAction,
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

    /// Export issues to stdout in a portable format (json, markdown, csv).
    /// JSON serializes the full issue and is what `import json` reads back
    /// (import re-creates issues, so slug/status/dates are not preserved —
    /// see `import`). CSV and Markdown are lossy, human-oriented views.
    Export {
        /// Output format
        #[arg(value_enum)]
        format: ExportFmt,

        /// Optional query to scope the export (same syntax as `list`).
        /// When supplied, the implicit "open only" default is disabled —
        /// combine with `--all` / `--closed` or an explicit folder term.
        #[arg(value_parser = parse_non_empty, allow_hyphen_values = true)]
        query: Option<String>,

        /// Include closed issues
        #[arg(long)]
        all: bool,

        /// Export only closed issues
        #[arg(long)]
        closed: bool,
    },

    /// Deprecated inbox-draft compatibility command
    #[command(hide = true)]
    Triage {
        /// Inbox slug to promote. Omit to list inbox drafts.
        #[arg(value_parser = parse_slug_arg)]
        slug: Option<String>,
    },

    /// Fuzzy-pick one issue and print its slug to stdout. Designed for
    /// piping into other commands: `issuectl pick | xargs issuectl show`.
    /// Without QUERY, all open issues are listed for interactive
    /// selection. With QUERY, a substring match across slug, title, and
    /// labels narrows the list; a unique match is printed immediately
    /// (non-interactive). The interactive prompt is sent to stderr so
    /// stdout stays clean for piping.
    Pick {
        /// Optional substring to filter on (matched against slug, title, labels).
        query: Option<String>,

        /// Include closed issues in the candidate pool.
        #[arg(long)]
        all: bool,

        /// Skip the interactive prompt — when multiple candidates match,
        /// pick the first (sorted alphabetically by slug) and exit.
        /// Combine with QUERY for scripted slug resolution.
        #[arg(long)]
        first: bool,
    },

    /// Generate shell completion scripts. Pipe to your shell's
    /// completion directory, for example with `issuectl completions zsh`
    /// redirected to `~/.zsh/completions/_issuectl`. The generated script statically
    /// covers subcommand and option names; dynamic value completions for
    /// slugs / statuses / labels / users are exposed via the hidden
    /// helper `issuectl _complete <kind>` which prints one value on each
    /// line; the generated script (with manual wiring) or a shell completion
    /// hook can consume that helper.
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: ShellArg,
    },

    /// Walk a hidden helper used by shell completion scripts. Prints one
    /// candidate value per line on stdout — the kind controls what is
    /// listed:
    /// - `slugs`: every active (non-archived, non-inbox) issue slug
    /// - `slugs-all`: every slug, including inbox + closed + archived
    /// - `statuses`: schema-declared status enum
    /// - `labels`: every label currently in use
    /// - `users`: every reporter/assignee/owner currently in use
    #[command(hide = true, name = "_complete")]
    CompleteValues {
        /// Kind of value to list.
        #[arg(value_enum)]
        kind: CompleteKind,
    },

    /// Walk repository source files and report `TODO(issue: <slug>)`
    /// markers. Categorises each hit as `tracked` (slug → open issue),
    /// `stale` (slug → closed issue), `unknown` (slug not found), or
    /// `untracked` (marker without a slug).
    ScanTodos {
        /// File every untracked TODO through the standard intake flow
        /// with provenance `scan-todos` and a stable source reference.
        #[arg(long = "file-intake")]
        file_intake: bool,

        /// Deprecated alias for `--file-intake`.
        #[arg(long = "create-inbox", hide = true)]
        create_inbox: bool,
    },

    /// Import issues from an external source. Each issue is created fresh
    /// through the same validation path as `issuectl create`: it gets a new
    /// slug and `open` status. Source status (so closed issues arrive
    /// open), dates, commits, and custom fields are not carried over.
    Import {
        #[command(subcommand)]
        source: ImportSource,
    },

    /// Show recent commits that touched issue files. Reads `git log`
    /// scoped to `issues/` and groups affected `item.md` paths back to
    /// slugs. History rewrites (rebase/squash) reshape what appears
    /// here; frontmatter `updated:` is authoritative when it disagrees.
    Activity {
        /// Time window (e.g. `7d`, `30`, or bare integer days).
        #[arg(long, value_parser = parse_non_empty)]
        since: Option<String>,

        /// Cap the number of entries returned.
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Reconstruct the status-transition history for one issue from
    /// `git log --follow -p` on its `item.md`. Only commits that change
    /// `status:` are listed (plus the creation commit). Frontmatter
    /// `created:` / `closed:` are authoritative when history has been
    /// rewritten.
    Timeline {
        /// Issue slug.
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
    },

    /// Generate markdown release notes for a git range. Walks
    /// `git log <range>` for `Refs-Issue:` / `Fixes-Issue:` trailers
    /// and groups the referenced issues by type.
    Changelog {
        /// Git revision range (`<ref>..<ref>` or a single SHA).
        #[arg(value_parser = parse_non_empty)]
        range: String,
    },

    /// Lightweight metrics derived from issue frontmatter. Cycle time
    /// uses `closed - created`; throughput counts closed issues; the
    /// workload rollup counts open issues by effective assignee.
    Metrics {
        /// Time window for throughput / cycle-time (e.g. `30d`).
        #[arg(long, value_parser = parse_non_empty)]
        since: Option<String>,
    },

    /// Edit an issue's blocker relationships. `blocked_by:` is the
    /// canonical frontmatter list; the reverse `blocks` view is
    /// derived at read time and never stored.
    Depend {
        #[command(subcommand)]
        action: DependAction,
    },

    /// Render the scheduling DAG: lanes, per-lane order, `blocked_by`
    /// mirror, and computed heads-of-line, all derived on read from the
    /// `lane` / `collision` fields joined with live status. The output
    /// reports each lane's serial depth and the current count of spawnable
    /// heads: the practical answer to "how parallel is my plan right now?"
    ///
    /// Design lanes as serial queues, not theme labels: only each lane's
    /// head-of-line can spawn, so the number of lanes is the parallelism
    /// budget and a nine-issue theme lane is nine serial slices. Put lane
    /// boundaries at independently mergeable conflict boundaries. A shared
    /// cross-lane file belongs in `collision:`, not by merging whole lanes;
    /// a hot file attracting many issues is a scheduling problem.
    ///
    /// Within each lane, ordering is `blocked_by` topology first, then
    /// priority (high, normal, low), `lane_seq` (ascending, with set values
    /// before unset), creation time, and finally slug. Priority deliberately
    /// outranks `lane_seq`.
    ///
    /// `lane: unlaned` means confirmed parallel-safe work: every member is
    /// independently headed and spawnable. It differs from an absent lane,
    /// which means unclassified work. See `docs/design/lane-design.md` for
    /// the full lane-design guidance. Deterministic and AI-first (`--json`
    /// carries `schema_version`). Optionally pass `--reservations` so
    /// spawnability accounts for the lane/collision tokens an orchestrator's
    /// in-flight runs already hold; issuectl never reads that from
    /// orchestratectl itself.
    Dag {
        /// Caller-supplied live run reservations, as JSON. Accepts a file
        /// path, `-` for stdin, or an inline JSON string. Shapes:
        /// `{"lanes":[..],"collision":[..]}` or an array of holds
        /// `[{"lane":..,"collision":[..]}]`. Without it, spawnability
        /// ignores reservations (head-of-line reported spawnable).
        #[arg(long, value_parser = parse_non_empty)]
        reservations: Option<String>,
    },

    /// Epic navigation views (read-only).
    ///
    /// Epics gather child issues via each child's `epic:` back-reference.
    /// `epic tree <slug>` renders that hierarchy as an indented tree.
    Epic {
        #[command(subcommand)]
        action: EpicAction,
    },

    /// Linear-style lightweight cycles (iterations).
    ///
    /// Issues opt in via an optional `cycle:` frontmatter label
    /// (e.g. `cycle: 2026-W22`). Set it with `issuectl set <slug>
    /// cycle 2026-W22`. There is no cycle catalog — the label is
    /// whatever string the team chose.
    Cycle {
        #[command(subcommand)]
        action: CycleAction,
    },

    /// Recurring / scheduled issues.
    ///
    /// Definitions live in `.issuectl/recurrences/<name>.yaml`
    /// (title, schedule cron, type, priority, labels, assignee,
    /// reporter, description). `issuectl schedule run` materializes a
    /// new issue file per due cron fire, with `recurrence_of:` and
    /// `occurrence:` frontmatter. The manifest at
    /// `.issuectl/recurrences/.manifest.yaml` dedupes occurrences —
    /// closing an instance has no effect on the next one.
    Schedule {
        #[command(subcommand)]
        action: ScheduleAction,
    },

    /// Aggregate open + in-progress workload across assignee, priority,
    /// cycle, and epic. Sums point-equivalents from `size:` (S=1, M=3,
    /// L=5, XL=8) and `estimate:` (free-form numeric) frontmatter; an
    /// issue without either contributes to the `unestimated` counter.
    Workload,

    /// ASCII burndown chart for a cycle. `--cycle <name>` selects the
    /// cycle label (use `current` for today's ISO week). When the
    /// label is an ISO week tag (`YYYY-Ww`) the chart spans Mon→Sun;
    /// otherwise it falls back to earliest-`created` → today. Closed
    /// issues subtract their points on their `closed:` date.
    Burndown {
        /// Cycle label (e.g. `2026-W22`). `current` expands to
        /// today's ISO week.
        #[arg(long, value_parser = parse_non_empty)]
        cycle: String,
    },

    /// Standard intake flow: file, triage, and dispose of bug reports and
    /// feature requests. Filing (`file`) is for the reporting agent;
    /// everything else is the developer / product-manager surface. See
    /// `docs/design/intake-flow.md`.
    Intake {
        #[command(subcommand)]
        action: IntakeAction,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigAction {
    /// Print the effective schema configuration file path
    Path,
    /// Show each effective schema value and whether it came from the repo file or a built-in default
    Show,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ExportFmt {
    Json,
    Markdown,
    Csv,
}

impl From<ExportFmt> for issuectl_core::transfer::ExportFormat {
    fn from(f: ExportFmt) -> Self {
        match f {
            ExportFmt::Json => Self::Json,
            ExportFmt::Markdown => Self::Markdown,
            ExportFmt::Csv => Self::Csv,
        }
    }
}

#[derive(Subcommand)]
pub(crate) enum EpicAction {
    /// Print an epic and its child issues as an indented tree. Children
    /// are the issues whose `epic:` back-reference points at the epic;
    /// a child that is itself an epic is expanded in turn. Read-only.
    ///
    /// With a `<slug>`, roots the tree at that issue. Without one, prints
    /// a forest of every top-level epic. `--json` emits the tree
    /// structurally (a nested `children` array per node).
    Tree {
        /// Epic to render. Accepts a bare slug or `@<slug>`, and a unique
        /// prefix. Omit to render all top-level epics.
        #[arg(value_parser = parse_slug_arg)]
        slug: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum DependAction {
    /// Add `<other>` (repeatable) to `<slug>`'s `blocked_by:` list.
    /// Each `--blocked-by` accepts a bare slug or `@<slug>`. Idempotent
    /// per-blocker: a value already in the list is a no-op for that
    /// entry. Self-references are rejected — an issue cannot block
    /// itself.
    Add {
        /// Issue whose `blocked_by:` list is being edited
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Slug of the blocker (repeatable). `--blocked-by foo --blocked-by bar`
        /// adds both in one call.
        #[arg(long = "blocked-by", value_parser = parse_non_empty, required = true)]
        blocked_by: Vec<String>,

        /// Optimistic-concurrency token; optional (opt-in compare-and-swap).
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },

    /// Remove `<other>` (repeatable) from `<slug>`'s `blocked_by:`
    /// list. Removing a value that isn't present is a no-op for that
    /// entry.
    Remove {
        /// Issue whose `blocked_by:` list is being edited
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Slug of the blocker to remove (repeatable).
        #[arg(long = "blocked-by", value_parser = parse_non_empty, required = true)]
        blocked_by: Vec<String>,

        /// Optimistic-concurrency token; optional (opt-in compare-and-swap).
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum CycleAction {
    /// Print the current cycle label (today's ISO week tag, e.g.
    /// `2026-W22`). With `--json`, emits `{"cycle":"..."}`.
    Current,

    /// List issues planned for `<name>`. Without `--all` / `--closed`,
    /// shows open issues only (mirrors `ls`). Output respects `--json`.
    Plan {
        /// Cycle label (e.g. `2026-W22`). Pass `current` to use the
        /// label `cycle current` would print — handy for scripts.
        #[arg(value_parser = parse_non_empty)]
        name: String,

        /// Include closed issues
        #[arg(long)]
        all: bool,

        /// Show only closed issues
        #[arg(long)]
        closed: bool,
    },

    /// Show open/closed rollup counts for a cycle. With no `<name>`,
    /// uses the current cycle. With `--all`, lists every distinct
    /// cycle found in the repo and its counts (ignores `<name>`).
    Status {
        /// Cycle label (optional). Pass `current` to use the
        /// current-cycle label.
        #[arg(value_parser = parse_non_empty)]
        name: Option<String>,

        /// Roll up every distinct cycle in the repo instead of one.
        #[arg(long, conflicts_with = "name")]
        all: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ScheduleAction {
    /// List loaded recurrence definitions.
    List,

    /// Materialize a new issue per due cron occurrence. First sight
    /// of a definition only "subscribes" — no retro-materialization.
    Run {
        /// Show what would be materialized without writing issues or
        /// updating the manifest.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ImportSource {
    /// Import from a JSON file: a top-level array of issue objects (or a
    /// single object). Reads issuectl's own `export json` output as well
    /// as hand-authored arrays. Each object needs at least a `title`.
    Json {
        /// Path to the JSON file
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// Issue type to assign when a record omits `type`
        #[arg(long, default_value = "task", value_parser = PossibleValuesParser::new(ISSUE_TYPES))]
        default_type: String,
    },

    /// Import open/closed issues from a GitHub repository via the `gh`
    /// CLI (`gh issue list --json …`). Requires `gh` on PATH and an
    /// authenticated session.
    Github {
        /// Repository in `owner/name` form
        #[arg(long, value_parser = parse_non_empty)]
        repo: String,

        /// Issue state to fetch: open, closed, or all. Note: imported
        /// issues are always created `open` regardless of this filter.
        #[arg(long, default_value = "open", value_parser = PossibleValuesParser::new(["open", "closed", "all"]))]
        state: String,

        /// Maximum number of issues to fetch
        #[arg(long, default_value_t = 100)]
        limit: u32,

        /// Issue type to assign to every imported issue
        #[arg(long, default_value = "task", value_parser = PossibleValuesParser::new(ISSUE_TYPES))]
        default_type: String,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum LabelOp {
    Add,
    Remove,
}

/// Clap-side mirror of `init_cmd::AgentSelection`. Kept here so the
/// core crate doesn't have to depend on clap.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ShellArg {
    Bash,
    Zsh,
    Fish,
    Powershell,
    Elvish,
}

impl From<ShellArg> for clap_complete::Shell {
    fn from(s: ShellArg) -> Self {
        match s {
            ShellArg::Bash => clap_complete::Shell::Bash,
            ShellArg::Zsh => clap_complete::Shell::Zsh,
            ShellArg::Fish => clap_complete::Shell::Fish,
            ShellArg::Powershell => clap_complete::Shell::PowerShell,
            ShellArg::Elvish => clap_complete::Shell::Elvish,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum CompleteKind {
    Slugs,
    SlugsAll,
    Statuses,
    Labels,
    Users,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum AgentArg {
    Claude,
    Codex,
    All,
}

impl From<AgentArg> for init_cmd::AgentSelection {
    fn from(a: AgentArg) -> Self {
        match a {
            AgentArg::Claude => Self::Claude,
            AgentArg::Codex => Self::Codex,
            AgentArg::All => Self::All,
        }
    }
}

#[derive(Subcommand)]
pub(crate) enum BodyAction {
    /// Replace the markdown body of an issue. Read content from stdin
    /// (`--stdin`) or a file (`--from-file PATH`). `--expected-version`
    /// is optional in both modes: pass it only when you want a
    /// compare-and-swap (it is enforced when passed). `flock` prevents
    /// corruption regardless; without a token, blind clobber is allowed.
    /// If the replacement has no H1, the existing title H1 is preserved
    /// and a warning is emitted. A different H1 is accepted but warns;
    /// prefer `update <slug> --title ...` for explicit retitling. A body
    /// using a reserved legacy section heading (`## Notes` — use
    /// `## Comments`) is accepted but warns; `issuectl doctor --fix`
    /// migrates it later.
    Set {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Read body from stdin
        #[arg(long, conflicts_with = "from_file")]
        stdin: bool,

        /// Read body from this file. Pass `-` to read stdin (use `./-`
        /// for a file literally named `-`). Input is capped at 10 MiB.
        #[arg(long = "from-file", value_name = "PATH", conflicts_with = "stdin")]
        from_file: Option<PathBuf>,

        /// Optimistic-concurrency token; optional (opt-in compare-and-swap)
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum AgentsAction {
    /// Write `.issuectl/AGENTS.md` with sensible defaults plus a
    /// schema-derived policy block (fenced with HTML-comment
    /// sentinels). Without `--force`, refuses to overwrite an
    /// existing file. `issuectl doctor --fix` regenerates the
    /// schema-derived block in place; the prose around it is
    /// preserved.
    Init {
        /// Overwrite an existing `.issuectl/AGENTS.md`.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum HooksAction {
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
pub(crate) enum SkillAction {
    /// List the bundled companion skills and their install targets. Read-only;
    /// unlike `pi-status`, this describes what this binary can install.
    List,
    /// Install the /issue skill template into the current repo. By default
    /// installs the Claude Code skill; use --agent codex for Codex CLI, or
    /// --agent all for both. The Claude install also ships the standalone
    /// intake skills /issue-new and /issue-intake (Claude-only).
    Install {
        /// Which agent's skill format to install
        #[arg(short = 'a', long, default_value = "claude", value_parser = PossibleValuesParser::new(["claude", "codex", "all"]))]
        agent: String,

        /// Overwrite existing skill bodies. A diverged issues/AGENTS.md is preserved.
        #[arg(long)]
        force: bool,

        /// Regenerate issues/AGENTS.md even when it contains repo-authored content.
        #[arg(long = "force-scaffold")]
        force_scaffold: bool,
    },
    /// Print the skill template to stdout (preview before installing)
    Print {
        /// Which agent's skill format to print
        #[arg(short = 'a', long, default_value = "claude", value_parser = PossibleValuesParser::new(["claude", "codex"]))]
        agent: String,
    },
    /// Report the state of the global pi.dev skill corpus
    /// (~/.pi/agent/skills): which issuectl-owned entries are up to date,
    /// stale (a different binary wrote them), hand-modified, missing,
    /// orphaned (a skill issuectl no longer ships, e.g. /triage-bugs), or
    /// unmanaged (not written by issuectl). Read-only.
    PiStatus,
    /// Prune the pi.dev skill corpus: remove orphaned issuectl-owned entries
    /// and clear manifest rows whose copy is gone. Dry-run by default — pass
    /// --force to apply. Never touches unmanaged (hand-authored) entries.
    PiPrune {
        /// Apply the removals (default is a dry run that only reports them)
        #[arg(long)]
        force: bool,
    },
}

/// The `--kind` axis of `intake reject`, mapped onto the
/// `disposition_reason` enum in the mutation layer.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum RejectKindArg {
    ByDesign,
    Wontfix,
    OutOfScope,
}

impl From<RejectKindArg> for mutate::intake::RejectKind {
    fn from(k: RejectKindArg) -> Self {
        match k {
            RejectKindArg::ByDesign => Self::ByDesign,
            RejectKindArg::Wontfix => Self::Wontfix,
            RejectKindArg::OutOfScope => Self::OutOfScope,
        }
    }
}

// Clap command enum: variants map 1:1 to subcommands and are parsed once, so
// the size imbalance between the wide `File` variant and the small ones is
// irrelevant — boxing here would only obscure the flat clap definition.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub(crate) enum IntakeAction {
    /// File a new intake item (reporting agent). Creates it directly in
    /// the `untriaged` reception state; idempotent on
    /// `(--provenance, --source-ref)`.
    File {
        /// Item type (never `epic`)
        #[arg(short = 't', long = "type", value_parser = PossibleValuesParser::new(ISSUE_TYPES))]
        issue_type: String,
        /// One-line title
        #[arg(long, value_parser = parse_non_empty)]
        title: String,
        /// Report body (free text). `--body-file` reads it from a file.
        #[arg(long, visible_alias = "description", value_parser = parse_non_empty)]
        body: Option<String>,
        /// Read the body from a file; `-` reads stdin.
        #[arg(long = "body-file", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Who reported it
        #[arg(long, value_parser = parse_non_empty)]
        reporter: Option<String>,
        /// Where the report came from (e.g. chat, email). Must be an
        /// accepted value when the repo constrains `provenance`.
        #[arg(long, value_parser = parse_non_empty)]
        provenance: String,
        /// Free-text provenance detail (for the open-ended `other` case)
        #[arg(long = "provenance-detail", value_parser = parse_non_empty)]
        provenance_detail: Option<String>,
        /// External identity of the source report (idempotency key),
        /// e.g. `chat:123/message:456`
        #[arg(long = "source-ref", value_parser = parse_non_empty)]
        source_ref: Option<String>,
        /// Filing-time severity hint
        #[arg(short = 'p', long, value_parser = PossibleValuesParser::new(PRIORITIES))]
        priority: Option<String>,
        /// Descriptive kebab-case slug
        #[arg(long, value_parser = parse_non_empty)]
        slug: Option<String>,
        /// Add a label (repeatable)
        #[arg(short = 'l', long = "label", value_parser = parse_non_empty)]
        labels: Vec<String>,
        /// Set a non-protected custom field (repeatable), `key=value`.
        /// Lifecycle keys (`status`, `type`, `reporter`, `provenance`, …)
        /// are rejected — use their dedicated flags.
        #[arg(long = "field", value_parser = parse_custom_field)]
        fields: Vec<(String, String)>,
    },
    /// Inspect the actionable intake queue (default: `untriaged`, oldest
    /// first).
    Queue {
        /// Filter by type
        #[arg(long = "type", value_parser = parse_non_empty)]
        issue_type: Option<String>,
        /// Filter by provenance
        #[arg(long, value_parser = parse_non_empty)]
        provenance: Option<String>,
        /// Only items lacking a `## Triage analysis` section
        #[arg(long = "needs-analysis")]
        needs_analysis: bool,
        /// View a non-default intake state instead of `untriaged`
        #[arg(long, value_parser = PossibleValuesParser::new(["deferred", "needs-info"]))]
        state: Option<String>,
    },
    /// Show one intake item with its attachments and analysis section
    Show {
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
    },
    /// Accept into the backlog (`→ open`)
    Accept {
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
        #[arg(short = 'a', long, value_parser = parse_non_empty)]
        assignee: Option<String>,
        #[arg(short = 'p', long, value_parser = PossibleValuesParser::new(PRIORITIES))]
        priority: Option<String>,
    },
    /// Park it (`→ deferred`)
    Defer {
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
        #[arg(long, value_parser = parse_non_empty)]
        reason: String,
        /// Wake-up date (`deferred_until`)
        #[arg(long, value_parser = parse_non_empty)]
        until: Option<String>,
    },
    /// Ask the reporter for more (`→ needs-info`)
    NeedInfo {
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
        #[arg(long, value_parser = parse_non_empty)]
        reason: String,
    },
    /// Reject / not-a-bug (`→ wontfix` + disposition reason)
    Reject {
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
        #[arg(long, value_parser = parse_non_empty)]
        reason: String,
        #[arg(long, value_enum, default_value_t = RejectKindArg::Wontfix)]
        kind: RejectKindArg,
    },
    /// Cannot reproduce (`→ cannot-reproduce`; bug-only)
    CannotReproduce {
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
        #[arg(long, value_parser = parse_non_empty)]
        reason: String,
    },
    /// Mark as a duplicate (`→ duplicate` + directed `duplicate_of`)
    Duplicate {
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
        /// Canonical item this duplicates
        #[arg(long, value_parser = parse_slug_arg)]
        of: String,
    },
    /// Obsolete / superseded (`→ obsolete`)
    Obsolete {
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
        #[arg(long, value_parser = parse_non_empty)]
        reason: String,
        #[arg(long = "superseded-by", value_parser = parse_slug_arg)]
        superseded_by: Option<String>,
    },
    /// Reclassify the type (valid only in an intake state)
    Retype {
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
        #[arg(long = "to", value_parser = PossibleValuesParser::new(ISSUE_TYPES))]
        to: String,
    },
    /// Reopen a closed item (`→ untriaged` by default, or `open`)
    Reopen {
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
        #[arg(long = "to", value_parser = PossibleValuesParser::new(["untriaged", "open"]))]
        to: Option<String>,
        #[arg(long, value_parser = parse_non_empty)]
        reason: String,
    },
    /// Reporter retracts their own untriaged report (`→ wontfix`)
    Withdraw {
        #[arg(value_parser = parse_slug_arg)]
        slug: String,
        #[arg(long, value_parser = parse_non_empty)]
        reason: String,
    },
    /// Migrate legacy label-encoded intake state to the new
    /// statuses/fields. **Dry-run by default** — reports what it would do;
    /// re-run with `--apply` to write. Idempotent and per-issue atomic;
    /// refuses ambiguous items rather than guessing.
    Migrate {
        /// Perform the migration (default is a dry-run report only)
        #[arg(long)]
        apply: bool,
    },
}

#[cfg(test)]
mod cli_tests;
mod cmd_skill;
mod intake;
mod read;
mod repo_admin;
/// Build the shared `--json` error object: `{"error":{"code","message"[,…]}}`.
/// `extra` (when an object) is merged into the inner `error` object so a
/// command can attach structured context (e.g. `matches` for a duplicate
/// precheck) without inventing a new top-level shape.
#[allow(dead_code)] // kept for focused legacy error-shape unit tests
mod runtime;
mod views;
mod views_extra;
mod views_tail;
mod write;

pub(crate) use cmd_skill::*;
pub(crate) use intake::*;
pub(crate) use read::*;
pub(crate) use repo_admin::*;
pub(crate) use runtime::*;
pub(crate) use views::*;
pub(crate) use views_extra::*;
pub(crate) use views_tail::*;
pub(crate) use write::*;
