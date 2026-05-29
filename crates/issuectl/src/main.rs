use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::builder::PossibleValuesParser;
use clap::{Parser, Subcommand, ValueEnum};

use issuectl_core::issue_fields::{ISSUE_TYPES, PRIORITIES};
use issuectl_core::repo_config::UncachedConfig;
use issuectl_core::{
    agents, body_sections, canonical, context, docs, doctor, duplicates, fmt, hooks,
    init as init_cmd, merge_driver, models, mutate, query, repo, server, skill, slug, sync_commits,
};

const TOP_LEVEL_HELP: &str = "\
Examples:
  issuectl ls                              List open issues
  issuectl ls -t bug -p high               Filter by type and priority
  issuectl ls --closed --json              Closed issues as JSON
  issuectl show extremely-quiet-otter          Full details by slug
  issuectl open extremely-quiet-otter          Edit item.md in $EDITOR
  issuectl search redirect                 Keyword search
  issuectl duplicates                      Flag likely-duplicate issue pairs
  issuectl new --type bug --title \"...\"    Create a new issue (random slug)
  issuectl update <slug> --status testing  Change status
  issuectl close <slug> --status fixed     Set a closing status (fixed/done/...)
  issuectl bulk \"label:stale\" --set status=wontfix  Mutate every matched issue
  issuectl export json > issues.json       Export issues (json/markdown/csv)
  issuectl import json issues.json          Import issues from a JSON file
  issuectl import github --repo o/r         Import open GitHub issues via gh
  issuectl init                            Bootstrap a new repo (schema, agents, skill)
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

        /// Before creating, scan existing issues for a strong duplicate
        /// (same heuristics as `duplicates`). If one is found, abort
        /// without creating and print the matches; re-run without this
        /// flag to create anyway.
        #[arg(long = "check-duplicates")]
        check_duplicates: bool,
    },

    /// Update fields of an existing issue or epic
    Update {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// New status (active or closing — frontmatter only, no directory move).
        /// Schema-aware: any status in `issues/.schema.yaml`'s `status` enum
        /// is accepted; final validation happens under-lock against the
        /// schema, where invalid statuses are rejected with the project's
        /// allowed-set spelled out.
        #[arg(short = 's', long, value_parser = parse_non_empty)]
        status: Option<String>,

        /// Change the issue type. Rejected with `MutateError::SchemaViolation`
        /// when the new type's schema-required body sections aren't already
        /// present (the user must add them first), and rejected when combined
        /// with a close→open reopen on the same call. Allowed values follow
        /// `issues/.schema.yaml` (`fields.type.enum`); CLI accepts any
        /// non-empty string and lets schema validation do the rejecting so
        /// repos that extend the type enum work end-to-end.
        #[arg(short = 't', long = "type", value_parser = parse_non_empty)]
        issue_type: Option<String>,

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

        /// Closing status (default: `fixed` for bugs, `done` otherwise).
        /// Schema-aware: a project that declares a custom closing status
        /// in `issues/.schema.yaml` (`status_classes: { archived: closing }`)
        /// can pass it here. Schema validation under-lock rejects values
        /// outside the project's `status` enum.
        #[arg(short = 's', long, value_parser = parse_non_empty)]
        status: Option<String>,

        /// Record a commit, format HASH:summary (repeatable)
        #[arg(long = "commit", value_parser = parse_non_empty)]
        commits: Vec<String>,

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
    /// (or `## Decisions` / `## Agent Runs` with `--decision` / `--agent-run`)
    Note {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// Author of the note (e.g. `alice` or `agent-name`)
        #[arg(long = "as", value_parser = parse_non_empty)]
        author: String,

        /// Note text (one positional argument; quote multi-word input).
        /// Mutually exclusive with `--stdin` / `--from-file`.
        #[arg(
            value_parser = parse_non_empty,
            conflicts_with_all = ["stdin", "from_file"]
        )]
        message: Option<String>,

        /// Read the note text from stdin instead of a positional argument
        #[arg(long, conflicts_with = "from_file")]
        stdin: bool,

        /// Read the note text from this file. Pass `-` to read stdin
        /// (use `./-` for a file literally named `-`). Input is capped
        /// at 10 MiB.
        #[arg(long = "from-file", value_name = "PATH", conflicts_with = "stdin")]
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
    /// fields, `custom_fields:`, label/related list ops, commits,
    /// and `body_ops:` (toggle_checkbox / append_note) — all
    /// applied under one flock with one schema-validation pass.
    Apply {
        /// Path to the YAML patch file
        #[arg(value_name = "PATCH")]
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
        /// merge-base is found.
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

    /// Import issues from an external source. Each issue is created fresh
    /// through the same validation path as `issuectl new`: it gets a new
    /// slug and `open` status. Source status (so closed issues arrive
    /// open), dates, commits, and custom fields are not carried over.
    Import {
        #[command(subcommand)]
        source: ImportSource,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum ExportFmt {
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
enum ImportSource {
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
enum LabelOp {
    Add,
    Remove,
}

/// Clap-side mirror of `init_cmd::AgentSelection`. Kept here so the
/// core crate doesn't have to depend on clap.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum AgentArg {
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

        /// Read body from this file. Pass `-` to read stdin (use `./-`
        /// for a file literally named `-`). Input is capped at 10 MiB.
        #[arg(long = "from-file", value_name = "PATH", conflicts_with = "stdin")]
        from_file: Option<PathBuf>,

        /// Optimistic-concurrency token; required with --json
        #[arg(long = "expected-version", value_parser = parse_non_empty)]
        expected_version: Option<String>,
    },
}

#[derive(Subcommand)]
enum AgentsAction {
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

/// Build the shared `--json` error object: `{"error":{"code","message"[,…]}}`.
/// `extra` (when an object) is merged into the inner `error` object so a
/// command can attach structured context (e.g. `matches` for a duplicate
/// precheck) without inventing a new top-level shape.
fn json_error_value(code: &str, message: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut err = serde_json::Map::new();
    err.insert("code".into(), serde_json::Value::String(code.to_string()));
    err.insert("message".into(), serde_json::Value::String(message.to_string()));
    if let serde_json::Value::Object(map) = extra {
        for (k, v) in map {
            err.insert(k, v);
        }
    }
    serde_json::json!({ "error": serde_json::Value::Object(err) })
}

/// Print the shared `--json` error object to stderr.
fn emit_json_error(code: &str, message: &str, extra: serde_json::Value) {
    eprintln!(
        "{}",
        serde_json::to_string_pretty(&json_error_value(code, message, extra)).unwrap_or_default()
    );
}

/// Fail a command under the unified output contract. With `--json` it
/// emits the shared `{"error":{…}}` object to stderr; otherwise it prints
/// the historical `Error: <message>` line. Exits with `code` (1 = generic
/// failure / not-found, 2 = refused-but-actionable). Used by the explicit
/// `process::exit` sites so they honour `--json` like the bubble-up path
/// in `main`.
fn fail(json: bool, code: i32, err_code: &str, message: &str, extra: serde_json::Value) -> ! {
    if json {
        emit_json_error(err_code, message, extra);
    } else {
        eprintln!("Error: {message}");
    }
    std::process::exit(code);
}

fn main() -> Result<()> {
    // Parse manually instead of `Cli::parse()` so clap's own usage
    // errors honour the `--json` contract too. By default clap prints
    // free-form text to stderr and exits 2 — an agent that always passes
    // `--json` would get un-parseable output and an exit code that
    // collides with our "refused-but-actionable" 2. We prescan argv for
    // `--json` (the flag itself can't have been parsed yet) and, when
    // set, render usage errors as the shared error envelope with exit 1,
    // reserving 2 for genuine refused-but-actionable outcomes. Help and
    // version requests are not errors — let clap print them normally.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind;
            let wants_json = std::env::args().skip(1).any(|a| a == "--json");
            if wants_json
                && !matches!(
                    e.kind(),
                    ErrorKind::DisplayHelp
                        | ErrorKind::DisplayVersion
                        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                )
            {
                emit_json_error("usage-error", e.to_string().trim_end(), serde_json::Value::Null);
                std::process::exit(1);
            }
            e.exit();
        }
    };
    let json_output = cli.json;
    ROOT_OVERRIDE.set(cli.root).ok();

    // Unified `--json` error contract: any error that bubbles up to
    // `main` is rendered as the shared `{"error":{code,message}}` object
    // on stderr (exit 1) so agents parse one shape regardless of which
    // command failed. Without `--json` we return the error unchanged so
    // anyhow's default `Error: …` rendering (and existing tests) are
    // preserved byte-for-byte.
    let result = dispatch(cli.command, json_output);
    if json_output {
        if let Err(e) = result {
            emit_json_error("command-failed", &format!("{e:#}"), serde_json::Value::Null);
            std::process::exit(1);
        }
        return Ok(());
    }
    result
}

fn dispatch(command: Command, json_output: bool) -> Result<()> {
    match command {
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
        Command::Open {
            slug,
            dir,
            editor,
        } => cmd_open(json_output, &slug, dir, editor),
        Command::Search { query, all } => cmd_search(json_output, &query, all),
        Command::Stats => cmd_stats(json_output),
        Command::Duplicates {
            slug,
            threshold,
            all,
        } => cmd_duplicates(json_output, slug.as_deref(), threshold, all),
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
            check_duplicates,
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
            check_duplicates,
        ),
        Command::Update {
            slug,
            status,
            issue_type,
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
                issue_type,
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
        Command::Rename {
            old_slug,
            new_slug,
            dry_run,
        } => cmd_rename(json_output, &old_slug, &new_slug, dry_run),
        Command::Stale { days } => cmd_stale(json_output, days),
        Command::Archive {
            older_than,
            dry_run,
        } => cmd_archive(json_output, older_than, dry_run),
        Command::Note {
            slug,
            author,
            message,
            stdin,
            from_file,
            decision,
            agent_run,
            dry_run,
            expected_version,
        } => cmd_note(
            json_output,
            &slug,
            &author,
            message,
            stdin,
            from_file,
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
        } => cmd_set(
            json_output,
            &slug,
            &field,
            value,
            clear,
            dry_run,
            expected_version,
        ),
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
        Command::Bulk {
            query,
            set,
            clear,
            add_labels,
            remove_labels,
            add_related,
            remove_related,
            dry_run,
        } => cmd_bulk(
            json_output,
            &query,
            BulkSpec {
                set,
                clear,
                add_labels,
                remove_labels,
                add_related,
                remove_related,
            },
            dry_run,
        ),
        Command::Body { action } => match action {
            BodyAction::Set {
                slug,
                stdin,
                from_file,
                expected_version,
            } => cmd_body_set(json_output, &slug, stdin, from_file, expected_version),
        },
        Command::Init {
            agent,
            with_hooks,
            with_merge_driver,
            force,
        } => {
            let root = find_root();
            let opts = init_cmd::InitOptions {
                agent: agent.into(),
                with_hooks,
                with_merge_driver,
                force,
            };
            init_cmd::run(&root, opts, json_output)
        }
        Command::Doctor { fix, verbose } => doctor::run(&find_root(), fix, json_output, verbose),
        Command::Hooks { action } => match action {
            HooksAction::Install { uninstall, force } => hooks::run(&find_root(), uninstall, force),
        },
        Command::SyncCommits {
            range,
            no_branch_fallback,
            dry_run,
        } => cmd_sync_commits(json_output, range, no_branch_fallback, dry_run),
        Command::Agents { action } => match action {
            AgentsAction::Init { force } => agents::run_init(&find_root(), force, json_output),
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
        Command::Export {
            format,
            query,
            all,
            closed,
        } => cmd_export(json_output, format, query, all, closed),
        Command::Import { source } => match source {
            ImportSource::Json { file, default_type } => {
                cmd_import_json(json_output, &file, &default_type)
            }
            ImportSource::Github {
                repo,
                state,
                limit,
                default_type,
            } => cmd_import_github(json_output, &repo, &state, limit, &default_type),
        },
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

/// The implicit folder scope shared by `list` and `export`: open issues
/// only by default, unless `--all` (no filter), `--closed`, or a
/// positional query (caller opts into scoping it themselves) is given.
fn folder_default_filter(all: bool, closed: bool, has_query: bool) -> Option<&'static str> {
    if all {
        None
    } else if closed {
        Some("closed")
    } else if has_query {
        None
    } else {
        Some("open")
    }
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
    let folder_filter = folder_default_filter(all, closed, query_str.is_some());

    let issues = load();
    // `repo::load_issues` already returns issues sorted by slug, so
    // we don't re-sort here.
    let filtered: Vec<_> = issues
        .into_iter()
        .filter(|i| folder_filter.map(|f| i.folder == f).unwrap_or(true) && query::matches(&q, i))
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
        None => fail(
            json,
            1,
            "not-found",
            &format!("issue {slug} not found"),
            serde_json::Value::Null,
        ),
    }
}

/// Open an issue's `item.md` (or its directory with `--dir`) in an
/// editor. The issue is a real file on disk, so we just resolve the
/// path and hand it to the editor. With `--json` we print the resolved
/// path instead of launching anything — agents and scripts cannot drive
/// an interactive editor, so spawning one would only hang them.
fn cmd_open(json: bool, slug: &str, dir: bool, editor: Option<String>) -> Result<()> {
    let root = find_root();
    // `locate_issue` returns (folder, item.md path) where `folder` is a
    // bare name like "open"/"closed", not a path — so the issue
    // directory is the parent of item.md.
    let (_, item_md) = locate_issue(&root, slug)?;
    let target = if dir {
        item_md
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow::anyhow!("cannot determine issue directory for {slug}"))?
    } else {
        item_md
    };

    if json {
        let report = serde_json::json!({
            "slug": slug,
            "path": target.to_string_lossy(),
            // `is_dir` (not `dir`) so the key never collides with the
            // issue-directory `dir` string field used by the action
            // commands — here it is the boolean "was --dir requested".
            "is_dir": dir,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let editor = editor
        .or_else(|| std::env::var("VISUAL").ok().filter(|s| !s.trim().is_empty()))
        .or_else(|| std::env::var("EDITOR").ok().filter(|s| !s.trim().is_empty()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no editor configured; pass --editor <cmd> or set $VISUAL / $EDITOR"
            )
        })?;

    // Hand the editor string to `sh -c` so shell quoting works the way
    // it does for git's `GIT_EDITOR` — `--editor "code -w"` or an editor
    // path containing spaces (quoted by the user) both behave correctly,
    // rather than the naive whitespace split that mangles them. The
    // target path is passed as a positional arg, not interpolated, so a
    // path with spaces or shell metacharacters is never re-parsed.
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$@\""))
        .arg("sh")
        .arg(&target)
        .status()
        .with_context(|| format!("failed to launch editor {editor:?}"))?;
    if !status.success() {
        // Propagate the editor's own exit code so callers can tell, e.g.,
        // a vim `:cq` abort from a crash.
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
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
        // Mirror `list`/`show`: attach the optimistic-concurrency
        // `version` token to each issue so a `search` hit can be fed
        // straight into a mutation without a second `show` round-trip.
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

fn cmd_duplicates(
    json: bool,
    slug: Option<&str>,
    threshold: Option<f64>,
    all: bool,
) -> Result<()> {
    let threshold = threshold.unwrap_or(duplicates::DEFAULT_THRESHOLD);
    let issues = load();

    match slug {
        Some(slug) => {
            let target = match issues.iter().find(|i| i.slug == slug) {
                Some(t) => t,
                None => fail(
                    json,
                    1,
                    "not-found",
                    &format!("issue {slug} not found"),
                    serde_json::Value::Null,
                ),
            };
            // The target is always a valid candidate scope; `--all`
            // only controls whether *closed* issues are compared
            // against it.
            let pool = issues
                .iter()
                .filter(|c| all || c.folder == "open" || c.slug == slug);
            let matches = duplicates::find_duplicates(target, pool, threshold);

            if json {
                let out: Vec<_> = matches
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "slug": m.slug,
                            "title": m.title,
                            "score": m.score,
                            "title_overlap": m.title_overlap,
                            "body_overlap": m.body_overlap,
                            "label_overlap": m.label_overlap,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if matches.is_empty() {
                println!("No likely duplicates of {slug} (threshold {threshold:.2}).");
            } else {
                println!("Likely duplicates of {slug} (threshold {threshold:.2}):");
                for m in &matches {
                    println!("  {:.2}  {}  {}", m.score, m.slug, m.title);
                }
            }
        }
        None => {
            let pool: Vec<_> = if all {
                issues
            } else {
                issues.into_iter().filter(|i| i.folder == "open").collect()
            };
            let pairs = duplicates::find_all_pairs(&pool, threshold);

            if json {
                let out: Vec<_> = pairs
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "a_slug": p.a_slug,
                            "a_title": p.a_title,
                            "b_slug": p.b_slug,
                            "b_title": p.b_title,
                            "score": p.score,
                            "title_overlap": p.title_overlap,
                            "body_overlap": p.body_overlap,
                            "label_overlap": p.label_overlap,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if pairs.is_empty() {
                println!("No likely duplicate pairs (threshold {threshold:.2}).");
            } else {
                println!("Likely duplicate pairs (threshold {threshold:.2}):");
                for p in &pairs {
                    println!(
                        "  {:.2}  {} <-> {}\n        {}\n        {}",
                        p.score, p.a_slug, p.b_slug, p.a_title, p.b_title
                    );
                }
            }
        }
    }

    Ok(())
}

use mutate::new_issue::{do_new, NewArgs};

/// Pre-creation duplicate check for `new --check-duplicates`. Builds a
/// synthetic issue from the prospective fields, scores it against the
/// existing open issues, and prints any strong matches. Returns `true`
/// when at least one strong match was found (the caller then refuses to
/// create).
fn duplicate_precheck(json: bool, args: &NewArgs) -> bool {
    let candidate = models::Issue {
        slug: String::new(),
        folder: "open".to_string(),
        created: None,
        status: "open".to_string(),
        updated: None,
        priority: args.priority.clone(),
        issue_type: args.issue_type.clone(),
        reporter: None,
        assignee: None,
        owner: None,
        epic: None,
        related: None,
        labels: if args.labels.is_empty() {
            None
        } else {
            Some(args.labels.clone())
        },
        closed: None,
        commits: None,
        title: args.title.clone(),
        body: args.description.clone().unwrap_or_default(),
        extra: BTreeMap::new(),
    };

    let existing = load();
    let open = existing.iter().filter(|i| i.folder == "open");
    let matches = duplicates::find_duplicates(&candidate, open, duplicates::STRONG_THRESHOLD);

    if matches.is_empty() {
        return false;
    }

    if json {
        let out: Vec<_> = matches
            .iter()
            .map(|m| {
                serde_json::json!({
                    "slug": m.slug,
                    "title": m.title,
                    "score": m.score,
                    "title_overlap": m.title_overlap,
                    "body_overlap": m.body_overlap,
                    "label_overlap": m.label_overlap,
                })
            })
            .collect();
        // Unified error contract: `{"error":{code,message,matches}}` on
        // stderr. The caller exits 2 (refused-but-actionable).
        emit_json_error(
            "duplicate-precheck",
            "strong duplicate(s) found; not created (re-run without --check-duplicates to create anyway)",
            serde_json::json!({ "matches": out }),
        );
    } else {
        eprintln!("Refusing to create: strong duplicate(s) found:");
        for m in &matches {
            eprintln!("  {:.2}  {}  {}", m.score, m.slug, m.title);
        }
        eprintln!("Re-run without --check-duplicates to create anyway.");
    }
    true
}

fn cmd_new(json: bool, args: NewArgs, check_duplicates: bool) -> Result<()> {
    let root = find_root();
    if check_duplicates && duplicate_precheck(json, &args) {
        // A strong match was found and printed; refuse to create.
        std::process::exit(2);
    }
    let out = do_new(&root, args, &UncachedConfig)?;
    if json {
        let report = serde_json::json!({
            "slug": out.slug,
            "title": out.title,
            // `path` = the item.md file; `dir` = the issue directory.
            // Shared vocabulary across all commands (see AGENTS.md).
            "path": out.item_path.to_string_lossy(),
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

fn cmd_export(
    json: bool,
    format: ExportFmt,
    query_str: Option<String>,
    all: bool,
    closed: bool,
) -> Result<()> {
    let q = match query_str.as_deref() {
        Some(s) => query::parse(s).context("parsing export query")?,
        None => query::Query::default(),
    };
    let folder_filter = folder_default_filter(all, closed, query_str.is_some());

    let issues = load();
    let filtered: Vec<_> = issues
        .into_iter()
        .filter(|i| folder_filter.map(|f| i.folder == f).unwrap_or(true) && query::matches(&q, i))
        .collect();

    let rendered = issuectl_core::transfer::export(&filtered, format.into())?;
    print!("{rendered}");
    if !rendered.ends_with('\n') {
        println!();
    }
    let _ = json; // export format is selected by `format`, not `--json`
    Ok(())
}

struct ImportOutcome {
    created: Vec<(String, String)>,
    failed: Vec<(String, String)>,
}

/// Create every parsed import record through `do_new`, accumulating
/// successes and per-record failures. Failures never abort the run — a
/// single malformed record should not lose the rest of an import.
fn run_import(
    records: Vec<issuectl_core::transfer::ImportRecord>,
    default_type: &str,
) -> ImportOutcome {
    let root = find_root();
    let mut outcome = ImportOutcome {
        created: Vec::new(),
        failed: Vec::new(),
    };
    for rec in records {
        let title = rec.title.clone();
        let args = rec.into_new_args(default_type);
        match do_new(&root, args, &UncachedConfig) {
            Ok(out) => outcome.created.push((out.slug, out.title)),
            Err(e) => outcome.failed.push((title, format!("{e:#}"))),
        }
    }
    outcome
}

fn report_import(json: bool, outcome: ImportOutcome) -> Result<()> {
    let ImportOutcome { created, failed } = outcome;
    if json {
        let report = serde_json::json!({
            "created": created
                .iter()
                .map(|(slug, title)| serde_json::json!({"slug": slug, "title": title}))
                .collect::<Vec<_>>(),
            "failed": failed
                .iter()
                .map(|(title, error)| serde_json::json!({"title": title, "error": error}))
                .collect::<Vec<_>>(),
            "created_count": created.len(),
            "failed_count": failed.len(),
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for (slug, title) in &created {
            println!("Created {slug}: {title}");
        }
        for (title, error) in &failed {
            eprintln!("Failed to import {title:?}: {error}");
        }
        println!(
            "Imported {} issue(s); {} failed.",
            created.len(),
            failed.len()
        );
    }
    // Distinct exit codes so scripts can tell total from partial
    // failure: 1 = nothing imported, 2 = some imported but some failed.
    // Exit 0 only when every record landed.
    if !failed.is_empty() {
        std::process::exit(if created.is_empty() { 1 } else { 2 });
    }
    Ok(())
}

fn cmd_import_json(json: bool, file: &Path, default_type: &str) -> Result<()> {
    let raw = fs::read_to_string(file)
        .with_context(|| format!("cannot read import file {}", file.display()))?;
    let records = issuectl_core::transfer::parse_json(&raw)?;
    let outcome = run_import(records, default_type);
    report_import(json, outcome)
}

fn cmd_import_github(
    json: bool,
    repo: &str,
    state: &str,
    limit: u32,
    default_type: &str,
) -> Result<()> {
    let output = std::process::Command::new("gh")
        .args([
            "issue",
            "list",
            "--repo",
            repo,
            "--state",
            state,
            "--limit",
            &limit.to_string(),
            "--json",
            "number,title,body,labels,state,assignees,url",
        ])
        .output()
        .context(
            "failed to run `gh` — install the GitHub CLI and authenticate with `gh auth login`",
        )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`gh issue list` failed: {}", stderr.trim());
    }
    let stdout = String::from_utf8(output.stdout).context("`gh` produced non-UTF8 output")?;
    let records = issuectl_core::transfer::parse_github(&stdout)?;
    let outcome = run_import(records, default_type);
    report_import(json, outcome)
}

#[derive(Default)]
pub(crate) struct UpdateArgs {
    pub slug: String,
    pub status: Option<String>,
    pub issue_type: Option<String>,
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
            "dir": out.final_dir.to_string_lossy(),
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
    if let Some(t) = args.issue_type {
        req.issue_type = Patch::Set(t);
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

    let outcome = mutate::update_issue(root, &args.slug, req, None, &UncachedConfig)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
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
            "dir": out.final_dir.to_string_lossy(),
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
    let outcome = mutate::close_issue(
        root,
        slug,
        status,
        commit_specs,
        expected_version,
        None,
        &UncachedConfig,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(UpdateOutcome {
        final_dir: outcome.issue_dir,
        moved_to_closed: outcome.moved_to_closed,
        moved_to_open: outcome.moved_to_open,
        version: outcome.version,
    })
}

fn cmd_rename(json: bool, old: &str, new: &str, dry_run: bool) -> Result<()> {
    let root = find_root();
    let outcome = repo::rename_issue(&root, old, new, dry_run)?;
    for s in &outcome.skipped {
        eprintln!(
            "Warning: skipped {} ({}); any references to {old} there were left untouched — run `issuectl doctor`",
            s.slug, s.reason
        );
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
        return Ok(());
    }
    let total: usize = outcome.changes.iter().map(|c| c.occurrences).sum();
    let files = outcome
        .changes
        .iter()
        .map(|c| c.slug.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    if dry_run {
        println!(
            "Would rename {old} → {new} and rewrite {total} reference(s) across {files} file(s)"
        );
        for c in &outcome.changes {
            println!("  {} {} ({})", c.slug, c.field, c.occurrences);
        }
    } else {
        println!(
            "Renamed {old} → {new} ({}); rewrote {total} reference(s) across {files} file(s)",
            outcome.new_dir.display()
        );
    }
    Ok(())
}

fn cmd_stale(json: bool, days: i64) -> Result<()> {
    let root = find_root();
    let report = issuectl_core::stale::find_stale(&root, days);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if report.stale.is_empty() {
        println!("No issues stale for {days}+ days.");
        return Ok(());
    }
    println!(
        "{} issue(s) with no activity in {days}+ days:",
        report.stale.len()
    );
    for s in &report.stale {
        let wip = if s.in_progress { " [in-progress]" } else { "" };
        let who = s
            .assignee
            .as_deref()
            .map(|a| format!(" — {a}"))
            .unwrap_or_default();
        println!(
            "  {} ({}){wip}  {} days, last {} via {}{who}",
            s.slug, s.status, s.days_inactive, s.last_activity, s.source
        );
    }
    Ok(())
}

fn cmd_archive(json: bool, older_than: i64, dry_run: bool) -> Result<()> {
    let root = find_root();
    let report = mutate::archive::archive_closed(&root, older_than, dry_run, &UncachedConfig)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let verb = if dry_run { "Would archive" } else { "Archived" };
    if report.archived.is_empty() {
        println!("Nothing to archive (closed {older_than}+ days ago).");
    } else {
        println!("{verb} {} issue(s):", report.archived.len());
        for mv in &report.archived {
            println!("  {} → {}", mv.slug, mv.to.display());
        }
    }
    for sk in &report.skipped {
        eprintln!("Warning: skipped {} ({})", sk.slug, sk.reason);
    }
    Ok(())
}

/// Locate an issue by slug. Returns (folder, item.md path) where
/// `folder` is the kanban-bucket label derived from frontmatter status.
/// Delegates to `repo::locate_issue`, which handles flat layout plus
/// legacy compat reads.
pub fn locate_issue(root: &Path, slug: &str) -> Result<(String, PathBuf)> {
    repo::locate_issue(root, slug)
}

fn cmd_sync_commits(
    json: bool,
    range: Option<String>,
    no_branch_fallback: bool,
    dry_run: bool,
) -> Result<()> {
    let root = find_root();
    let report = sync_commits::run(
        &root,
        sync_commits::SyncOptions {
            range,
            no_branch_fallback,
            dry_run,
        },
    )?;

    if json {
        let planned: Vec<_> = report
            .planned
            .iter()
            .map(|p| {
                serde_json::json!({
                    "slug": p.slug,
                    "hash": p.hash,
                    "summary": p.summary,
                    "kind": match p.kind {
                        sync_commits::AttributionKind::Refs => "refs",
                        sync_commits::AttributionKind::Fixes => "fixes",
                        sync_commits::AttributionKind::Branch => "branch",
                    },
                    "already_present": p.already_present,
                })
            })
            .collect();
        let applied: serde_json::Map<_, _> = report
            .applied
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::from(*v)))
            .collect();
        let envelope = serde_json::json!({
            "range": report.range,
            "branch_slug": report.branch,
            "planned": planned,
            "applied": serde_json::Value::Object(applied),
            "fixes_hints": report.fixes_hints.iter().collect::<Vec<_>>(),
            "unknown_slugs": report.unknown_slugs.iter().collect::<Vec<_>>(),
            "load_warnings": report.load_warnings,
            "dry_run": report.dry_run,
        });
        println!("{}", serde_json::to_string_pretty(&envelope)?);
    } else {
        println!("Range: {}", report.range);
        if let Some(b) = &report.branch {
            println!("Branch fallback slug: @{b}");
        }
        if report.planned.is_empty() {
            println!("No commits with Refs-Issue / Fixes-Issue trailers in range.");
        } else {
            for p in &report.planned {
                let tag = match p.kind {
                    sync_commits::AttributionKind::Refs => "refs",
                    sync_commits::AttributionKind::Fixes => "fixes",
                    sync_commits::AttributionKind::Branch => "branch",
                };
                let suffix = if p.already_present {
                    " (already present)"
                } else if dry_run {
                    " (would add)"
                } else {
                    ""
                };
                println!(
                    "  @{slug:<32} {hash} {summary} [{tag}]{suffix}",
                    slug = p.slug,
                    hash = p.hash,
                    summary = p.summary,
                );
            }
        }
        if !report.applied.is_empty() {
            let total: usize = report.applied.values().sum();
            println!(
                "Added {total} commit(s) across {n} issue(s).",
                n = report.applied.len()
            );
        } else if !dry_run && !report.planned.is_empty() {
            println!("No new commits to add (all already present).");
        }
        for slug in &report.fixes_hints {
            eprintln!("Hint: @{slug} has Fixes-Issue trailer — consider `issuectl close {slug}`",);
        }
        for slug in &report.unknown_slugs {
            eprintln!("Warning: trailer references unknown slug @{slug} (no issue with that slug)",);
        }
        for w in &report.load_warnings {
            eprintln!("Warning: {w}");
        }
    }
    Ok(())
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

/// Resolve the note text from exactly one of: a positional `message`,
/// `--stdin`, or `--from-file PATH`. clap's `conflicts_with` guards
/// against more than one being set; this enforces that at least one is
/// present and that the resulting text is non-empty (a blank note is a
/// no-op that would only clutter the issue body). Returns the text with
/// surrounding whitespace trimmed so a stray trailing newline (e.g. from
/// `echo … | issuectl note --stdin`) doesn't bloat the issue body.
fn read_message_arg(
    message: Option<String>,
    stdin: bool,
    from_file: Option<PathBuf>,
) -> Result<String> {
    let text = if let Some(m) = message {
        m
    } else if let Some(path) = from_file {
        read_capped_file(&path, "note")?
    } else if stdin {
        read_capped_stdin("note")?
    } else {
        bail!("provide the note text as an argument, or use --stdin / --from-file PATH");
    };
    let text = text.trim();
    if text.is_empty() {
        bail!("note text is empty");
    }
    Ok(text.to_string())
}

/// Upper bound on text read from stdin or a `--from-file` path. Bounds
/// memory so an unbounded source (`/dev/zero`, a runaway producer) fails
/// with a clear error instead of OOM-ing the process. 10 MiB is far above
/// any realistic note or issue body yet small enough to never threaten the
/// process.
const MAX_INPUT_BYTES: u64 = 10 * 1024 * 1024;

/// Read up to [`MAX_INPUT_BYTES`] from `reader`, erroring if the source
/// exceeds the cap or isn't valid UTF-8. `what`/`source` name the input in
/// error messages (e.g. "note" from "stdin"). Reads one byte past `limit`
/// so an exactly-at-limit input passes but anything larger is rejected
/// — and so an unbounded source (`/dev/zero`) is short-circuited after
/// `limit + 1` bytes rather than buffered to exhaustion. `limit` is a
/// parameter (not the constant directly) so tests can exercise the cap
/// with a tiny bound instead of allocating megabytes.
fn read_capped<R: std::io::Read>(
    reader: R,
    limit: u64,
    what: &str,
    source: impl std::fmt::Display,
) -> Result<String> {
    use std::io::Read;
    let mut buf = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut buf)
        .with_context(|| format!("cannot read {what} from {source}"))?;
    if buf.len() as u64 > limit {
        bail!("{what} from {source} exceeds the {limit}-byte limit");
    }
    String::from_utf8(buf).map_err(|e| {
        // Surface the byte offset so an agent (or human) can locate the
        // bad byte instead of guessing across a multi-megabyte input.
        let offset = e.utf8_error().valid_up_to();
        anyhow::anyhow!("{what} from {source} is not valid UTF-8 (first invalid byte at offset {offset})")
    })
}

/// Read `what` text from stdin, capped at [`MAX_INPUT_BYTES`]. Refuses to
/// block on an interactive terminal — an agent or script that forgot to
/// pipe input gets a clear error rather than a hung process.
fn read_capped_stdin(what: &str) -> Result<String> {
    use std::io::IsTerminal;
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        bail!("stdin is a terminal; pipe the {what} text in, or pass a real file path to --from-file");
    }
    read_capped(stdin.lock(), MAX_INPUT_BYTES, what, "stdin")
}

/// Read `what` text from a `--from-file` path, capped at [`MAX_INPUT_BYTES`].
/// A path of `-` means stdin (the standard Unix convention), routed through
/// [`read_capped_stdin`] so it gets the same TTY guard and cap. To read a
/// file literally named `-`, pass `./-`.
fn read_capped_file(path: &Path, what: &str) -> Result<String> {
    if path.as_os_str() == "-" {
        return read_capped_stdin(what);
    }
    let file = fs::File::open(path)
        .with_context(|| format!("cannot read {what} from {}", path.display()))?;
    read_capped(file, MAX_INPUT_BYTES, what, path.display())
}

#[allow(clippy::too_many_arguments)]
fn cmd_note(
    json: bool,
    slug: &str,
    author: &str,
    message: Option<String>,
    stdin: bool,
    from_file: Option<PathBuf>,
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
    let message = read_message_arg(message, stdin, from_file)?;
    let message = message.as_str();
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
        &UncachedConfig,
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
    let outcome = mutate::update_issue(&root, slug, req, None, &UncachedConfig)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
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
    let outcome = mutate::toggle_checkbox(
        &root,
        slug,
        task,
        expected_version,
        None,
        dry_run,
        &UncachedConfig,
    )
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
    let outcome = mutate::update_issue(&root, slug, req, None, &UncachedConfig)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    finish_mutation(json, slug, &outcome, dry_run, "Updated labels for")
}

fn cmd_apply(json: bool, patch_path: &Path, dry_run: bool) -> Result<()> {
    let yaml_text = fs::read_to_string(patch_path)
        .with_context(|| format!("cannot read patch file {}", patch_path.display()))?;
    let (slug, mut req) = parse_apply_patch(&yaml_text, json)
        .with_context(|| format!("cannot parse patch fields in {}", patch_path.display()))?;
    req.dry_run = dry_run;
    let root = find_root();
    let outcome = mutate::update_issue(&root, &slug, req, None, &UncachedConfig)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    finish_mutation(json, &slug, &outcome, dry_run, "Applied patch to")
}

/// Mutation to apply to every issue a `bulk` query matches. Mirrors the
/// subset of `UpdateArgs` that makes sense across many issues at once:
/// per-field set/clear and the label/related list ops. Per-issue
/// concerns (`expected_version`, commits) are intentionally absent —
/// bulk can't carry a distinct version per target.
#[derive(Default)]
pub(crate) struct BulkSpec {
    pub set: Vec<(String, String)>,
    pub clear: Vec<String>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
    pub add_related: Vec<String>,
    pub remove_related: Vec<String>,
}

impl BulkSpec {
    fn is_empty(&self) -> bool {
        self.set.is_empty()
            && self.clear.is_empty()
            && self.add_labels.is_empty()
            && self.remove_labels.is_empty()
            && self.add_related.is_empty()
            && self.remove_related.is_empty()
    }
}

/// One issue's outcome from a `bulk` run. `diff` is `Some` only in
/// dry-run mode (the bytes the write would have produced).
#[derive(Debug)]
pub(crate) struct BulkResult {
    pub slug: String,
    pub version: String,
    pub final_dir: PathBuf,
    pub diff: Option<String>,
}

/// Route a single field patch onto an `UpdateIssueRequest`. Built-in
/// single-value fields go to their typed slots (so e.g. a `status` set
/// gets the closed-date handling and a `status`/`type` clear hits the
/// canonical "cannot be cleared" error); everything else lands in the
/// schema-validated custom-field slot. Mirrors `cmd_set`'s routing,
/// extended with `type`.
fn route_bulk_field(req: &mut mutate::UpdateIssueRequest, key: &str, patch: mutate::Patch<String>) {
    match key {
        "status" => req.status = patch,
        "type" => req.issue_type = patch,
        "priority" => req.priority = patch,
        "assignee" => req.assignee = patch,
        "owner" => req.owner = patch,
        "epic" => req.epic = patch,
        other => {
            req.custom_fields.insert(other.to_string(), patch);
        }
    }
}

/// Build a fresh request from the spec. `mutate::bulk_update` calls this
/// factory once per target per phase (validate, then write) because
/// `UpdateIssueRequest` is not `Clone` and each write consumes its own
/// request; the mutation content is identical every time.
fn build_bulk_request(spec: &BulkSpec, dry_run: bool) -> mutate::UpdateIssueRequest {
    use mutate::Patch;
    let mut req = mutate::UpdateIssueRequest {
        dry_run,
        ..Default::default()
    };
    for (k, v) in &spec.set {
        route_bulk_field(&mut req, k, Patch::Set(v.clone()));
    }
    for k in &spec.clear {
        route_bulk_field(&mut req, k, Patch::Clear);
    }
    req.add_labels = spec.add_labels.clone();
    req.remove_labels = spec.remove_labels.clone();
    req.add_related = spec.add_related.clone();
    req.remove_related = spec.remove_related.clone();
    req
}

/// CLI-side spec checks that don't need disk access: at least one
/// mutation, and no key named twice or in both `--set` and `--clear`
/// (a `BTreeMap` would silently keep the last write otherwise). Mirrors
/// the `--field`/`--clear-field` dedup rules in `do_update`.
fn validate_bulk_spec(spec: &BulkSpec) -> Result<()> {
    if spec.is_empty() {
        bail!("bulk requires at least one mutation (--set/--clear/--add-label/--remove-label/--add-related/--remove-related)");
    }
    let mut seen_set: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for (k, _) in &spec.set {
        if !seen_set.insert(k.as_str()) {
            bail!("--set {k:?} given more than once");
        }
    }
    let mut seen_clear: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for k in &spec.clear {
        if !seen_clear.insert(k.as_str()) {
            bail!("--clear {k:?} given more than once");
        }
    }
    if let Some(overlap) = seen_set.intersection(&seen_clear).next() {
        bail!("field {overlap:?} appears in both --set and --clear");
    }
    Ok(())
}

/// Apply `spec` to every issue matching `q`, as one batch under a single
/// repo-wide flock (see [`mutate::bulk_update`]). Every target is
/// validated before any write lands, so a bad value writes nothing, and
/// there is no concurrent-writer race between validation and write.
pub(crate) fn bulk_apply(
    root: &Path,
    q: &query::Query,
    spec: &BulkSpec,
    dry_run: bool,
) -> Result<Vec<BulkResult>> {
    let slugs: Vec<String> = repo::load_issues(root)
        .into_iter()
        .filter(|i| query::matches(q, i))
        .map(|i| i.slug)
        .collect();
    if slugs.is_empty() {
        return Ok(Vec::new());
    }

    let outcomes = mutate::bulk_update(
        root,
        &slugs,
        |dr| build_bulk_request(spec, dr),
        dry_run,
        None,
        &UncachedConfig,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    let results = slugs
        .into_iter()
        .zip(outcomes)
        .map(|(slug, outcome)| {
            let diff = dry_run.then(|| {
                let before = outcome.before_serialized.as_deref().unwrap_or("");
                let after = outcome.pending_serialized.as_deref().unwrap_or(before);
                render_unified_diff(before, after, &outcome.issue_dir)
            });
            BulkResult {
                slug,
                version: outcome.version,
                final_dir: outcome.issue_dir,
                diff,
            }
        })
        .collect();
    Ok(results)
}

fn cmd_bulk(json: bool, query_str: &str, spec: BulkSpec, dry_run: bool) -> Result<()> {
    validate_bulk_spec(&spec)?;
    let q = query::parse(query_str).context("parsing bulk query")?;
    let root = find_root();
    let results = bulk_apply(&root, &q, &spec, dry_run)?;

    if json {
        let arr: Vec<_> = results
            .iter()
            .map(|r| {
                let mut o = serde_json::json!({
                    "slug": r.slug,
                    "version": r.version,
                    "dir": r.final_dir.to_string_lossy(),
                });
                if let Some(d) = &r.diff {
                    o["diff"] = serde_json::Value::String(d.clone());
                }
                o
            })
            .collect();
        let report = serde_json::json!({
            "dry_run": dry_run,
            "count": results.len(),
            "results": arr,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    if results.is_empty() {
        println!("No issues match the query.");
        return Ok(());
    }
    if dry_run {
        println!("{} issue(s) would be updated:", results.len());
        for r in &results {
            println!("  {}", r.slug);
        }
        println!();
        for r in &results {
            if let Some(d) = &r.diff {
                if !d.is_empty() {
                    print!("{d}");
                }
            }
        }
    } else {
        println!("Updated {} issue(s):", results.len());
        for r in &results {
            println!("  {}", r.slug);
        }
    }
    Ok(())
}

/// Parse the YAML patch text into `(slug, UpdateIssueRequest)`,
/// applying every CLI-side rule that doesn't require disk access.
/// Extracted so tests can pin the `--json` `expected_version`
/// rejection rules (round-2 #4) without spinning up `find_root` or
/// the global `ROOT_OVERRIDE`.
pub(crate) fn parse_apply_patch(
    yaml_text: &str,
    json: bool,
) -> Result<(String, mutate::UpdateIssueRequest)> {
    let mut yaml: serde_yaml::Value =
        serde_yaml::from_str(yaml_text).context("cannot parse as YAML")?;
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
    let req: mutate::UpdateIssueRequest =
        serde_yaml::from_value(yaml).context("cannot parse patch fields")?;
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
    Ok((slug, req))
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
                "dir": outcome.issue_dir.to_string_lossy(),
                "version": outcome.version,
                "moved_to_closed": outcome.moved_to_closed,
                "moved_to_open": outcome.moved_to_open,
                "dry_run": true,
                "diff": diff,
                "warnings": outcome.warnings,
            });
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print!("{diff}");
            emit_warnings_to_stderr(&outcome.warnings);
        }
        return Ok(());
    }
    if json {
        let report = serde_json::json!({
            "slug": slug,
            "dir": outcome.issue_dir.to_string_lossy(),
            "version": outcome.version,
            "moved_to_closed": outcome.moved_to_closed,
            "moved_to_open": outcome.moved_to_open,
            "warnings": outcome.warnings,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{human_verb} {slug}");
        emit_warnings_to_stderr(&outcome.warnings);
    }
    Ok(())
}

/// Print non-fatal advisories from `UpdateOutcome::warnings` to stderr
/// so the human-readable CLI surface mirrors the JSON `warnings` key.
/// Body-only verbs (`note`, `check`) emit transition-rule mismatches
/// here without blocking the write — see `mutate::transition_warnings`.
fn emit_warnings_to_stderr(warnings: &[String]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
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
    let body = diff.unified_diff().context_radius(3).to_string();
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
    let body = if let Some(path) = from_file {
        read_capped_file(&path, "body")?
    } else if stdin {
        read_capped_stdin("body")?
    } else {
        bail!("specify exactly one of --stdin or --from-file");
    };
    // Strip only *trailing* whitespace, not leading: a stray final newline
    // from `echo … |` or an editor's end-of-file newline shouldn't bloat the
    // stored body, but a body legitimately starts with whitespace (a leading
    // 4-space indented code block, intentional spacing) that a full `trim()`
    // would silently corrupt. This is the deliberate divergence from `note`,
    // whose text is short prose that is fully trimmed and rejected when empty;
    // a body is a whole document and its leading content is the user's intent.
    // `update_body` re-adds the canonical leading newline.
    let body = body.trim_end().to_string();
    let root = find_root();
    let outcome = mutate::update_body(
        &root,
        slug,
        expected_version,
        body,
        None,
        false,
        &UncachedConfig,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    if json {
        let report = serde_json::json!({
            "slug": slug,
            "version": outcome.version,
            "dir": outcome.issue_dir.to_string_lossy(),
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Updated body of {slug}");
    }
    Ok(())
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

/// Truncate `text` to roughly `max_len` Unicode scalar values, ending
/// with `…` when truncated. Note: this counts `chars()` (scalar
/// values), not grapheme clusters or terminal-display columns — CJK
/// wide characters and emoji ZWJ sequences may still misalign the
/// table. Switching to `unicode-width` is tracked as a follow-up;
/// this guard exists only to avoid panicking on UTF-8 byte boundaries.
fn truncate(text: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }
    let char_count = text.chars().count();
    if char_count <= max_len {
        text.to_string()
    } else {
        let take = max_len.saturating_sub(1);
        let mut out: String = text.chars().take(take).collect();
        out.push('…');
        out
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
    fn truncate_handles_non_ascii_at_boundary() {
        // Regression: byte-index slicing panicked at non-char boundary
        // for Finnish titles like "Käyttäjän kirjautuminen rikki".
        let title = "Käyttäjän kirjautuminen rikki sisäänkirjautumisessa";
        // Should not panic for any max_len <= char count.
        for n in 1..=title.chars().count() {
            let _ = truncate(title, n);
        }
        // Truncated output should be a valid string ending in ellipsis
        // when truncation actually happens.
        let out = truncate(title, 10);
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().count(), 10);
    }

    #[test]
    fn truncate_keeps_short_text_unchanged() {
        assert_eq!(truncate("ä", 5), "ä");
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_max_len_zero_returns_empty() {
        // Without the guard, `max_len = 0` would push `…` and return
        // a single-character string, violating the contract.
        assert_eq!(truncate("anything", 0), "");
        assert_eq!(truncate("", 0), "");
    }

    #[test]
    fn update_sets_status_and_bumps_updated() {
        let tmp = fresh_repo();
        let mut a = new_args("bug", "Bug");
        a.slug = Some("my-test-slug".into());
        a.reporter = Some("rep".into());
        a.assignee = Some("ass".into());
        let n = do_new(tmp.path(), a, &UncachedConfig).unwrap();
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
        let n = do_new(tmp.path(), a, &UncachedConfig).unwrap();
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
        let n = do_new(tmp.path(), a, &UncachedConfig).unwrap();
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
        let n = do_new(tmp.path(), a, &UncachedConfig).unwrap();
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
        let n = do_new(tmp.path(), a, &UncachedConfig).unwrap();
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
        let n = do_new(tmp.path(), a, &UncachedConfig).unwrap();
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
        let n = do_new(tmp.path(), a, &UncachedConfig).unwrap();
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
    fn parse_days_accepts_bare_and_suffix() {
        assert_eq!(parse_days("90").unwrap(), 90);
        assert_eq!(parse_days("90d").unwrap(), 90);
        assert_eq!(parse_days("0").unwrap(), 0);
        assert!(parse_days("-5").is_err());
        assert!(parse_days("7days").is_err());
        assert!(parse_days("d").is_err());
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
    fn parse_non_empty_rejects_empty_and_padded() {
        assert!(parse_non_empty("").is_err());
        assert!(parse_non_empty("  ").is_err());
        assert!(parse_non_empty(" a").is_err());
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

    fn write_raw_issue(root: &Path, slug: &str, fm: &str, body: &str) {
        let dir = root.join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), format!("---\n{fm}---\n{body}")).unwrap();
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

    // ── parse_apply_patch (round-2 #4) ────────────────────────────

    #[test]
    fn parse_apply_patch_rejects_null_expected_version_under_json() {
        let yaml = "slug: some-issue\nexpected_version: null\npriority: high\n";
        let err = parse_apply_patch(yaml, true).unwrap_err();
        assert!(
            err.to_string().contains("expected_version"),
            "expected expected_version error, got {err}"
        );
    }

    #[test]
    fn parse_apply_patch_rejects_missing_expected_version_under_json() {
        let yaml = "slug: some-issue\npriority: high\n";
        let err = parse_apply_patch(yaml, true).unwrap_err();
        assert!(err.to_string().contains("expected_version"));
    }

    #[test]
    fn parse_apply_patch_rejects_empty_and_padded_expected_version_under_json() {
        for v in [
            "expected_version: \"\"",
            "expected_version: \"   \"",
            "expected_version: \" sha256:abc \"",
        ] {
            let yaml = format!("slug: some-issue\n{v}\npriority: high\n");
            let err = parse_apply_patch(&yaml, true).unwrap_err();
            assert!(
                err.to_string().contains("expected_version"),
                "expected expected_version error for {v:?}, got {err}"
            );
        }
    }

    #[test]
    fn parse_apply_patch_rejects_user_supplied_dry_run_field() {
        let yaml = "slug: some-issue\ndry_run: true\npriority: high\n";
        let err = parse_apply_patch(yaml, false).unwrap_err();
        assert!(
            err.to_string().contains("dry_run") && err.to_string().contains("CLI flag"),
            "expected dry_run CLI-flag error, got {err}"
        );
    }

    #[test]
    fn parse_apply_patch_accepts_well_formed_json_patch() {
        let yaml = "slug: well-formed-issue\nexpected_version: sha256:abc123\npriority: high\n";
        let (slug, req) = parse_apply_patch(yaml, true).unwrap();
        assert_eq!(slug, "well-formed-issue");
        assert_eq!(req.expected_version.as_deref(), Some("sha256:abc123"));
    }

    #[test]
    fn parse_apply_patch_allows_missing_expected_version_when_not_json() {
        // Non-JSON callers may opt into blind clobber: `flock` still
        // serializes writes, but no version check is required.
        let yaml = "slug: some-issue\npriority: high\n";
        let (slug, req) = parse_apply_patch(yaml, false).unwrap();
        assert_eq!(slug, "some-issue");
        assert!(req.expected_version.is_none());
    }

    // ── bulk ──────────────────────────────────────────────────────

    fn bulk_spec(set: &[(&str, &str)], add_labels: &[&str]) -> BulkSpec {
        BulkSpec {
            set: set
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            add_labels: add_labels.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn status_of(root: &Path, slug: &str) -> String {
        repo::load_issues(root)
            .into_iter()
            .find(|i| i.slug == slug)
            .unwrap_or_else(|| panic!("issue {slug} not found"))
            .status
    }

    #[test]
    fn parse_bulk_set_accepts_built_ins_and_custom() {
        assert_eq!(
            parse_bulk_set("status=done").unwrap(),
            ("status".to_string(), "done".to_string())
        );
        assert_eq!(
            parse_bulk_set("team=payments").unwrap(),
            ("team".to_string(), "payments".to_string())
        );
        assert!(parse_bulk_set("status").is_err());
        assert!(parse_bulk_set("status=").is_err());
        assert!(parse_bulk_set("=done").is_err());
        assert!(parse_bulk_set(" status=done").is_err());
        assert!(parse_bulk_set("status =done").is_err());
        assert!(parse_bulk_set("bad key=done").is_err());
    }

    #[test]
    fn parse_bulk_set_rejects_unroutable_built_ins_with_hint() {
        // List-shaped and auto-managed built-ins can't go through --set;
        // the error points at the right flag instead of landing in the
        // custom-field slot and erroring late.
        let err = parse_bulk_set("labels=foo").unwrap_err();
        assert!(err.contains("--add-label"), "got {err:?}");
        let err = parse_bulk_set("related=foo").unwrap_err();
        assert!(err.contains("--add-related"), "got {err:?}");
        for k in ["title", "slug", "commits", "closed", "created"] {
            assert!(
                parse_bulk_set(&format!("{k}=foo")).is_err(),
                "{k} must be rejected"
            );
        }
        // Routable built-ins and genuine custom fields still pass.
        assert!(parse_bulk_set("priority=high").is_ok());
        assert!(parse_bulk_set("team=payments").is_ok());
    }

    #[test]
    fn parse_bulk_clear_rejects_unroutable_built_ins() {
        assert!(parse_bulk_clear_key("labels").is_err());
        assert!(parse_bulk_clear_key("title").is_err());
        assert!(parse_bulk_clear_key("epic").is_ok());
        assert!(parse_bulk_clear_key("team").is_ok());
    }

    #[test]
    fn validate_bulk_spec_rejects_empty_and_dups() {
        assert!(validate_bulk_spec(&BulkSpec::default()).is_err());
        let dup_set = BulkSpec {
            set: vec![
                ("priority".into(), "high".into()),
                ("priority".into(), "low".into()),
            ],
            ..Default::default()
        };
        assert!(validate_bulk_spec(&dup_set).is_err());
        let overlap = BulkSpec {
            set: vec![("epic".into(), "some-epic".into())],
            clear: vec!["epic".into()],
            ..Default::default()
        };
        assert!(validate_bulk_spec(&overlap).is_err());
        assert!(validate_bulk_spec(&bulk_spec(&[("priority", "high")], &[])).is_ok());
    }

    #[test]
    fn bulk_applies_set_to_every_match() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: normal\nassignee: alice\n",
            "# One\n",
        );
        write_raw_issue(
            tmp.path(),
            "calm-bright-newt",
            "type: bug\nstatus: open\npriority: normal\nassignee: alice\n",
            "# Two\n",
        );
        write_raw_issue(
            tmp.path(),
            "eager-silent-mole",
            "type: feature\nstatus: open\npriority: normal\nassignee: bob\n",
            "# Three\n",
        );

        let q = query::parse("assignee:alice").unwrap();
        let spec = bulk_spec(&[("priority", "high")], &[]);
        let results = bulk_apply(tmp.path(), &q, &spec, false).unwrap();

        let mut slugs: Vec<_> = results.iter().map(|r| r.slug.clone()).collect();
        slugs.sort();
        assert_eq!(slugs, vec!["amber-loud-fox", "calm-bright-newt"]);

        let issues = repo::load_issues(tmp.path());
        let by = |s: &str| {
            issues
                .iter()
                .find(|i| i.slug == s)
                .unwrap()
                .priority
                .clone()
        };
        assert_eq!(by("amber-loud-fox"), "high");
        assert_eq!(by("calm-bright-newt"), "high");
        // The non-matching issue is untouched.
        assert_eq!(by("eager-silent-mole"), "normal");
    }

    #[test]
    fn bulk_set_status_routes_through_typed_slot() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: normal\nassignee: alice\n",
            "# One\n",
        );
        let q = query::parse("status:open").unwrap();
        let spec = bulk_spec(&[("status", "done")], &[]);
        bulk_apply(tmp.path(), &q, &spec, false).unwrap();
        assert_eq!(status_of(tmp.path(), "amber-loud-fox"), "done");
        // A closing status routes through the typed slot, so `closed:`
        // is stamped (and the issue lands in the closed folder).
        let issue = repo::load_issues(tmp.path())
            .into_iter()
            .find(|i| i.slug == "amber-loud-fox")
            .unwrap();
        assert_eq!(issue.folder, "closed");
        assert!(issue.closed.is_some());
    }

    #[test]
    fn bulk_dry_run_writes_nothing_and_returns_diffs() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: normal\n",
            "# One\n",
        );
        let q = query::parse("status:open").unwrap();
        let spec = bulk_spec(&[("priority", "high")], &[]);
        let results = bulk_apply(tmp.path(), &q, &spec, true).unwrap();
        assert_eq!(results.len(), 1);
        let diff = results[0].diff.as_deref().unwrap();
        assert!(diff.contains("priority"), "diff should mention the change");
        // Nothing written: on-disk priority is unchanged.
        let issues = repo::load_issues(tmp.path());
        assert_eq!(issues[0].priority, "normal");
    }

    #[test]
    fn bulk_dry_run_status_change_writes_nothing_but_shows_diff() {
        // Flat layout: the directory is `issues/<slug>/` regardless of
        // status, so a status change shows up in the diff (frontmatter +
        // a stamped `closed:`), not as a directory move. Dry-run must
        // write nothing.
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: normal\nassignee: alice\n",
            "# One\n",
        );
        let q = query::parse("status:open").unwrap();
        let spec = bulk_spec(&[("status", "done")], &[]);
        let results = bulk_apply(tmp.path(), &q, &spec, true).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0]
            .final_dir
            .to_string_lossy()
            .ends_with("issues/amber-loud-fox"));
        let diff = results[0].diff.as_deref().unwrap();
        assert!(diff.contains("status: done"), "diff: {diff}");
        assert!(diff.contains("closed:"), "diff should stamp closed: {diff}");
        // Still a dry run: on-disk status is unchanged.
        assert_eq!(status_of(tmp.path(), "amber-loud-fox"), "open");
    }

    #[test]
    fn bulk_no_match_is_empty_not_error() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\n",
            "# One\n",
        );
        let q = query::parse("assignee:nobody").unwrap();
        let spec = bulk_spec(&[("priority", "high")], &[]);
        let results = bulk_apply(tmp.path(), &q, &spec, false).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn bulk_preflight_aborts_all_on_one_invalid_target() {
        // Two issues match; the priority value is invalid, so the
        // dry-run pre-flight must reject before any write lands.
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: normal\n",
            "# One\n",
        );
        write_raw_issue(
            tmp.path(),
            "calm-bright-newt",
            "type: bug\nstatus: open\npriority: normal\n",
            "# Two\n",
        );
        let q = query::parse("status:open").unwrap();
        let spec = bulk_spec(&[("priority", "bogus")], &[]);
        let err = bulk_apply(tmp.path(), &q, &spec, false).unwrap_err();
        assert!(
            err.to_string().contains("priority"),
            "expected a priority validation error, got {err}"
        );
        // No file was rewritten — both keep their original priority.
        let issues = repo::load_issues(tmp.path());
        for i in &issues {
            assert_eq!(i.priority, "normal", "{} must be untouched", i.slug);
        }
    }

    #[test]
    fn bulk_adds_label_to_matches() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: normal\nlabels: [frontend]\n",
            "# One\n",
        );
        let q = query::parse("status:open").unwrap();
        let spec = bulk_spec(&[], &["triaged"]);
        bulk_apply(tmp.path(), &q, &spec, false).unwrap();
        let issue = repo::load_issues(tmp.path())
            .into_iter()
            .find(|i| i.slug == "amber-loud-fox")
            .unwrap();
        let labels = issue.labels.unwrap_or_default();
        assert!(labels.contains(&"triaged".to_string()));
        assert!(labels.contains(&"frontend".to_string()));
    }

    #[test]
    fn read_message_arg_prefers_positional() {
        let got = read_message_arg(Some("hello".into()), false, None).unwrap();
        assert_eq!(got, "hello");
    }

    #[test]
    fn read_message_arg_reads_from_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("note.md");
        fs::write(&path, "from a file\n").unwrap();
        let got = read_message_arg(None, false, Some(path)).unwrap();
        assert_eq!(got, "from a file");
    }

    #[test]
    fn read_message_arg_requires_a_source() {
        assert!(read_message_arg(None, false, None).is_err());
    }

    #[test]
    fn read_message_arg_rejects_blank_text() {
        assert!(read_message_arg(Some("   \n".into()), false, None).is_err());
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("blank.md");
        fs::write(&path, "\n\n").unwrap();
        assert!(read_message_arg(None, false, Some(path)).is_err());
    }

    // Small limit so cap tests stay sub-millisecond instead of allocating
    // the real 10 MiB bound.
    const TEST_LIMIT: u64 = 16;

    #[test]
    fn read_capped_accepts_input_at_the_limit() {
        let data = vec![b'a'; TEST_LIMIT as usize];
        let got = read_capped(data.as_slice(), TEST_LIMIT, "note", "test").unwrap();
        assert_eq!(got.len(), TEST_LIMIT as usize);
    }

    #[test]
    fn read_capped_rejects_input_over_the_limit() {
        let data = vec![b'a'; TEST_LIMIT as usize + 1];
        let err = read_capped(data.as_slice(), TEST_LIMIT, "note", "test").unwrap_err();
        assert!(err.to_string().contains("exceeds"), "got: {err}");
    }

    #[test]
    fn read_capped_short_circuits_an_unbounded_source() {
        // `io::repeat` is an infinite reader (a `/dev/zero` stand-in). If the
        // `take(limit + 1)` guard regressed to reading everything, this test
        // would hang / OOM instead of returning a prompt "exceeds" error.
        let err = read_capped(std::io::repeat(b'a'), TEST_LIMIT, "body", "test").unwrap_err();
        assert!(err.to_string().contains("exceeds"), "got: {err}");
    }

    #[test]
    fn read_capped_rejects_invalid_utf8_with_offset() {
        let data: &[u8] = &[b'o', b'k', 0xff, 0xfe];
        let err = read_capped(data, TEST_LIMIT, "body", "test").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("UTF-8"), "got: {msg}");
        assert!(msg.contains("offset 2"), "got: {msg}");
    }

    #[test]
    fn read_capped_file_reads_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("body.md");
        fs::write(&path, "hello body\n").unwrap();
        let got = read_capped_file(&path, "body").unwrap();
        assert_eq!(got, "hello body\n");
    }

    #[test]
    fn read_capped_file_missing_path_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist.md");
        assert!(read_capped_file(&missing, "body").is_err());
    }
}
