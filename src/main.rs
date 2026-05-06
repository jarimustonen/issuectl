mod docs;
mod doctor;
mod models;
mod parser;
mod repo;
mod server;
mod skill;
mod slug;
mod write;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::builder::PossibleValuesParser;
use clap::{Parser, Subcommand};

const ISSUE_TYPES: &[&str] = &["bug", "task", "feature", "improvement", "chore", "epic"];
const PRIORITIES: &[&str] = &["normal", "high"];
const ACTIVE_STATUSES: &[&str] = &["open", "in-progress", "testing"];
const CLOSING_STATUSES: &[&str] = &[
    "done",
    "fixed",
    "wontfix",
    "duplicate",
    "cannot-reproduce",
    "obsolete",
];

fn all_statuses() -> Vec<&'static str> {
    ACTIVE_STATUSES
        .iter()
        .chain(CLOSING_STATUSES.iter())
        .copied()
        .collect()
}

fn is_closing_status(status: &str) -> bool {
    CLOSING_STATUSES.contains(&status)
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
  issuectl close <slug> --status fixed     Move to closed/ with closing status
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
    },

    /// Update fields of an existing issue or epic
    Update {
        /// Issue slug
        #[arg(value_parser = parse_slug_arg)]
        slug: String,

        /// New status (active or closing — closing also moves to closed/)
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
    },

    /// Set a closing status and move the issue to closed/
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
    },

    /// Health-check the repo and (with --fix) migrate legacy numbered issues to slugs
    Doctor {
        /// Apply migrations and fixes (otherwise read-only report)
        #[arg(long)]
        fix: bool,
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
            },
        ),
        Command::Close {
            slug,
            status,
            commits,
        } => cmd_close(json_output, &slug, status, commits),
        Command::Doctor { fix } => doctor::run(&find_root(), fix, json_output),
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
        } => server::run(
            find_root(),
            host,
            port,
            server::ServeOptions {
                watch_enabled: !no_watch,
                watch_bulk_threshold: watch_bulk_threshold as usize,
            },
        ),
    }
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
    assignee: Option<String>,
    issue_type: Option<String>,
    priority: Option<String>,
    status: Option<String>,
    epic: Option<String>,
    label: Option<String>,
    all: bool,
    closed: bool,
) -> Result<()> {
    let issues = load();
    let mut filtered = issues;

    if closed && !all {
        filtered.retain(|i| i.folder == "closed");
    } else if !all && !closed {
        filtered.retain(|i| i.folder == "open");
    }

    if let Some(a) = assignee {
        let a_lower = a.to_lowercase();
        filtered.retain(|i| i.effective_assignee().to_lowercase() == a_lower);
    }
    if let Some(t) = issue_type {
        filtered.retain(|i| i.issue_type == t);
    }
    if let Some(p) = priority {
        filtered.retain(|i| i.priority == p);
    }
    if let Some(s) = status {
        filtered.retain(|i| i.status == s);
    }
    if let Some(e) = epic {
        filtered.retain(|i| i.epic.as_deref() == Some(e.as_str()));
    }
    if let Some(l) = label {
        let l_lower = l.to_lowercase();
        filtered.retain(|i| {
            i.labels
                .as_ref()
                .map(|lbs| lbs.iter().any(|lb| lb.to_lowercase() == l_lower))
                .unwrap_or(false)
        });
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
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
                println!("{}", serde_json::to_string_pretty(i)?);
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

fn cmd_search(json: bool, query: &str, all: bool) -> Result<()> {
    let issues = load();
    let query_lower = query.to_lowercase();

    let mut filtered: Vec<_> = issues
        .into_iter()
        .filter(|i| {
            if !all && i.folder != "open" {
                return false;
            }
            i.title.to_lowercase().contains(&query_lower)
                || i.slug.to_lowercase().contains(&query_lower)
                || i.body.to_lowercase().contains(&query_lower)
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

struct NewArgs {
    issue_type: String,
    title: String,
    slug: Option<String>,
    reporter: Option<String>,
    assignee: Option<String>,
    owner: Option<String>,
    priority: String,
    epic: Option<String>,
    labels: Vec<String>,
    related: Vec<String>,
    source: Option<String>,
    description: Option<String>,
}

struct NewOutcome {
    slug: String,
    title: String,
    item_path: PathBuf,
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

fn do_new(root: &Path, args: NewArgs) -> Result<NewOutcome> {
    if args.issue_type == "epic" {
        if args.assignee.is_some() || args.reporter.is_some() {
            bail!("epics use --owner, not --reporter/--assignee");
        }
    } else if args.owner.is_some() {
        bail!("--owner is only valid with --type epic");
    }

    let related = normalize_related_refs(&args.related)?;

    let render = write::render_new_item(&write::NewIssueArgs {
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
    });

    let open_parent = root.join("issues").join("open");
    fs::create_dir_all(&open_parent)
        .with_context(|| format!("cannot create {}", open_parent.display()))?;

    // Pick a slug atomically: try `fs::create_dir` (which fails on
    // EEXIST) so two concurrent `issuectl new` invocations cannot race
    // through `dir.exists()` then both call `fs::create_dir_all` (the
    // latter is idempotent and does not detect a pre-existing dir).
    // Returns the slug actually claimed and the open path.
    let (slug, dir) = match &args.slug {
        Some(s) => {
            let normalized = write::slugify(s, 10);
            if !slug::is_valid(&normalized) {
                bail!(
                    "--slug {:?} normalized to {:?}, which is not a valid slug \
                     (need ≥2 lowercase ASCII kebab segments, optional digits)",
                    s,
                    normalized
                );
            }
            // Detect a pre-existing closed issue with the same slug
            // before attempting `create_dir` in open/, so the error
            // message is precise.
            let closed_dir = write::issue_dir(root, "closed", &normalized);
            if closed_dir.exists() {
                bail!(
                    "slug {normalized} already used by closed issue at {}",
                    closed_dir.display()
                );
            }
            let dir = open_parent.join(&normalized);
            match fs::create_dir(&dir) {
                Ok(()) => (normalized, dir),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    bail!("target directory already exists: {}", dir.display())
                }
                Err(e) => {
                    return Err(
                        anyhow::Error::from(e).context(format!("cannot create {}", dir.display()))
                    )
                }
            }
        }
        None => claim_random_slug(root, &open_parent)?,
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
            .with_context(|| format!("cannot create {}", item_path.display()))?;
        f.write_all(render.as_bytes())
            .with_context(|| format!("cannot write {}", item_path.display()))?;
    }

    Ok(NewOutcome {
        slug,
        title: args.title,
        item_path,
    })
}

/// Generate a random slug and atomically claim its open/ directory.
/// Loops on `EEXIST` so that two concurrent processes that happen to
/// pick the same slug both retry rather than silently overwriting.
fn claim_random_slug(root: &Path, open_parent: &Path) -> Result<(String, PathBuf)> {
    for _ in 0..16 {
        let candidate = slug::generate();
        // Cheap pre-check: skip slugs that already exist on disk to
        // avoid burning a random pick when the answer is obvious.
        if write::issue_dir(root, "closed", &candidate).exists() {
            continue;
        }
        let dir = open_parent.join(&candidate);
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
struct UpdateArgs {
    slug: String,
    status: Option<String>,
    assignee: Option<String>,
    owner: Option<String>,
    priority: Option<String>,
    epic: Option<String>,
    no_epic: bool,
    add_labels: Vec<String>,
    remove_labels: Vec<String>,
    add_related: Vec<String>,
    remove_related: Vec<String>,
    add_commits: Vec<String>,
}

struct UpdateOutcome {
    final_dir: PathBuf,
    moved_to_closed: bool,
    moved_to_open: bool,
}

fn cmd_update(json: bool, args: UpdateArgs) -> Result<()> {
    let root = find_root();
    let slug = args.slug.clone();
    let out = do_update(&root, args)?;
    if json {
        let report = serde_json::json!({
            "slug": slug,
            "final_dir": out.final_dir.to_string_lossy(),
            "moved_to_closed": out.moved_to_closed,
            "moved_to_open": out.moved_to_open,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if out.moved_to_closed {
        println!("Updated {slug}: moved to {}", out.final_dir.display());
        println!("  status set to closing — moved to closed/");
    } else if out.moved_to_open {
        println!(
            "Updated {slug}: re-opened, moved to {}",
            out.final_dir.display()
        );
    } else {
        println!("Updated {slug}");
    }
    Ok(())
}

fn do_update(root: &Path, args: UpdateArgs) -> Result<UpdateOutcome> {
    let (folder, item_path) = locate_issue(root, &args.slug)?;
    let mut item = write::read_item(&item_path)?;

    let mut new_folder = folder.clone();
    let mut moved_to_closed = false;
    let mut moved_to_open = false;

    if let Some(ref status) = args.status {
        write::set_string(&mut item.frontmatter, "status", status);
        if is_closing_status(status) {
            new_folder = "closed".to_string();
            write::set_string(&mut item.frontmatter, "closed", &write::today());
            if folder == "open" {
                moved_to_closed = true;
            }
        } else if folder == "closed" {
            write::remove_key(&mut item.frontmatter, "closed");
            new_folder = "open".to_string();
            moved_to_open = true;
        }
    }

    if let Some(a) = args.assignee {
        write::set_string(&mut item.frontmatter, "assignee", &a);
    }
    if let Some(o) = args.owner {
        write::set_string(&mut item.frontmatter, "owner", &o);
    }
    if let Some(p) = args.priority {
        write::set_string(&mut item.frontmatter, "priority", &p);
    }
    if let Some(e) = args.epic {
        write::set_string(&mut item.frontmatter, "epic", &e);
    } else if args.no_epic {
        write::remove_key(&mut item.frontmatter, "epic");
    }

    for label in &args.add_labels {
        write::add_to_string_list(&mut item.frontmatter, "labels", label)?;
    }
    for label in &args.remove_labels {
        write::remove_from_string_list(&mut item.frontmatter, "labels", label)?;
    }

    let add_related = normalize_related_refs(&args.add_related)?;
    let remove_related = normalize_related_refs(&args.remove_related)?;
    for r in &add_related {
        write::add_to_string_list(&mut item.frontmatter, "related", r)?;
    }
    for r in &remove_related {
        write::remove_from_string_list(&mut item.frontmatter, "related", r)?;
    }

    for spec in &args.add_commits {
        let (hash, summary) = parse_commit_spec(spec)?;
        write::add_commit(&mut item.frontmatter, &hash, &summary)?;
    }

    write::set_string(&mut item.frontmatter, "updated", &write::today());

    write::write_item(&item_path, &item)?;

    let final_dir = if new_folder != folder {
        let new_dir = write::issue_dir(root, &new_folder, &args.slug);
        let old_dir = item_path
            .parent()
            .expect("item.md must have a parent")
            .to_path_buf();
        if new_dir.exists() {
            bail!("target directory already exists: {}", new_dir.display());
        }
        if let Some(parent) = new_dir.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        fs::rename(&old_dir, &new_dir).with_context(|| {
            format!("cannot move {} to {}", old_dir.display(), new_dir.display())
        })?;
        new_dir
    } else {
        item_path
            .parent()
            .expect("item.md must have a parent")
            .to_path_buf()
    };

    Ok(UpdateOutcome {
        final_dir,
        moved_to_closed,
        moved_to_open,
    })
}

fn cmd_close(json: bool, slug: &str, status: Option<String>, commits: Vec<String>) -> Result<()> {
    let root = find_root();
    let out = do_close(&root, slug, status, commits)?;
    if json {
        let report = serde_json::json!({
            "slug": slug,
            "final_dir": out.final_dir.to_string_lossy(),
            "moved_to_closed": out.moved_to_closed,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if out.moved_to_closed {
        println!("Closed {slug}: moved to {}", out.final_dir.display());
    } else {
        println!("Updated {slug}");
    }
    Ok(())
}

fn do_close(
    root: &Path,
    slug: &str,
    status: Option<String>,
    commits: Vec<String>,
) -> Result<UpdateOutcome> {
    let (folder, item_path) = locate_issue(root, slug)?;
    if folder == "closed" {
        bail!("issue {slug} is already in closed/ (use `update` to change status)");
    }
    let item = write::read_item(&item_path)?;
    let issue_type = item
        .frontmatter
        .get(serde_yaml::Value::String("type".into()))
        .and_then(|v| v.as_str())
        .unwrap_or("bug")
        .to_string();
    let resolved_status = status.unwrap_or_else(|| {
        if issue_type == "bug" {
            "fixed".to_string()
        } else {
            "done".to_string()
        }
    });

    do_update(
        root,
        UpdateArgs {
            slug: slug.to_string(),
            status: Some(resolved_status),
            add_commits: commits,
            ..Default::default()
        },
    )
}

/// Locate an issue by slug. Returns (folder, item.md path).
pub fn locate_issue(root: &Path, slug: &str) -> Result<(String, PathBuf)> {
    for folder in &["open", "closed"] {
        let folder_path = root.join("issues").join(folder).join(slug);
        if folder_path.is_dir() {
            let item = folder_path.join("item.md");
            if !item.is_file() {
                bail!("{slug} directory has no item.md: {}", item.display());
            }
            return Ok((folder.to_string(), item));
        }
    }
    bail!("issue {slug} not found in issues/open/ or issues/closed/")
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
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        fs::create_dir_all(tmp.path().join("issues/closed")).unwrap();
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
        fs::create_dir_all(tmp.path().join("issues/open/taken")).unwrap();
        fs::write(
            tmp.path().join("issues/open/taken/item.md"),
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
    fn update_with_closing_status_moves_to_closed() {
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
        let content = read(&outcome.final_dir.join("item.md"));
        assert!(content.contains("status: fixed"));
        assert!(content.contains("closed:"));
        assert!(!n.item_path.exists());
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
        let outcome = do_close(tmp.path(), &n.slug, None, vec![]).unwrap();
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
        let outcome = do_close(tmp.path(), &n.slug, None, vec![]).unwrap();
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
        do_close(tmp.path(), &n.slug, None, vec![]).unwrap();
        assert!(do_close(tmp.path(), &n.slug, None, vec![]).is_err());
    }

    #[test]
    fn locate_issue_finds_in_open_and_closed() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/open/foo-bar")).unwrap();
        fs::write(tmp.path().join("issues/open/foo-bar/item.md"), "---\n---\n").unwrap();
        let (folder, _) = locate_issue(tmp.path(), "foo-bar").unwrap();
        assert_eq!(folder, "open");
        assert!(locate_issue(tmp.path(), "missing").is_err());
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
    fn is_closing_status_classifies_correctly() {
        for s in CLOSING_STATUSES {
            assert!(is_closing_status(s));
        }
        for s in ACTIVE_STATUSES {
            assert!(!is_closing_status(s));
        }
    }
}
