mod models;
mod parser;
mod repo;
mod skill;
mod write;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::builder::PossibleValuesParser;
use clap::{Parser, Subcommand};
use regex::{Captures, Regex};

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
  issuectl show 12                         Full details of issue #12
  issuectl search redirect                 Keyword search
  issuectl new --type bug --title \"...\"    Create a new issue (slug auto-derived)
  issuectl update 12 --status testing      Change status
  issuectl close 12 --status fixed         Move to closed/ with closing status
  issuectl renumber                        Renumber and rewrite cross-refs
  issuectl skill install                   Install /issue skill in current repo
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

        /// Filter by parent epic number
        #[arg(short = 'e', long)]
        epic: Option<u32>,

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
        /// Issue number
        number: u32,
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

    /// Create a new issue or epic
    New {
        /// Item type
        #[arg(short = 't', long = "type", value_parser = PossibleValuesParser::new(ISSUE_TYPES))]
        issue_type: String,

        /// Item title (markdown heading and slug source)
        #[arg(long, value_parser = parse_non_empty)]
        title: String,

        /// Override the auto-generated slug
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

        /// Parent epic number
        #[arg(short = 'e', long)]
        epic: Option<u32>,

        /// Add a label (repeatable)
        #[arg(short = 'l', long = "label", value_parser = parse_non_empty)]
        labels: Vec<String>,

        /// Add a related issue reference like "#12" (repeatable)
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
        /// Issue number
        number: u32,

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

        /// Set parent epic number
        #[arg(short = 'e', long)]
        epic: Option<u32>,

        /// Remove the parent epic reference
        #[arg(long, conflicts_with = "epic")]
        no_epic: bool,

        /// Add a label (repeatable)
        #[arg(long = "add-label", value_parser = parse_non_empty)]
        add_labels: Vec<String>,

        /// Remove a label (repeatable)
        #[arg(long = "remove-label", value_parser = parse_non_empty)]
        remove_labels: Vec<String>,

        /// Add a related reference like "#12" (repeatable)
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
        /// Issue number
        number: u32,

        /// Closing status (default: `fixed` for bugs, `done` otherwise)
        #[arg(short = 's', long, value_parser = PossibleValuesParser::new(CLOSING_STATUSES))]
        status: Option<String>,

        /// Record a commit, format HASH:summary (repeatable)
        #[arg(long = "commit", value_parser = parse_non_empty)]
        commits: Vec<String>,
    },

    /// Renumber duplicate issues and fix cross-references after merges.
    /// Unique numbers are preserved; only duplicate numbers (multiple dirs
    /// sharing one number) are renumbered, with the first kept and the rest
    /// spilled above the current max.
    Renumber {
        /// Print the plan without modifying anything
        #[arg(long)]
        dry_run: bool,

        /// Path(s) under which to rewrite #NN references in markdown files.
        /// Repeatable. Defaults to the entire repo root (recursively, .md
        /// files only, skipping .git/target/node_modules/.cargo/dist/build).
        #[arg(long = "scope", value_name = "PATH")]
        scopes: Vec<PathBuf>,
    },

    /// Install or preview the /issue skill template (Claude Code or Codex)
    Skill {
        #[command(subcommand)]
        action: SkillAction,
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
        Command::Show { number } => cmd_show(json_output, number),
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
        } => cmd_new(NewArgs {
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
        }),
        Command::Update {
            number,
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
        } => cmd_update(UpdateArgs {
            number,
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
        }),
        Command::Close {
            number,
            status,
            commits,
        } => cmd_close(number, status, commits),
        Command::Renumber { dry_run, scopes } => cmd_renumber(dry_run, scopes),
        Command::Skill { action } => match action {
            SkillAction::Install { agent, force } => cmd_skill_install(&agent, force),
            SkillAction::Print { agent } => cmd_skill_print(&agent),
        },
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

fn cmd_list(
    json: bool,
    assignee: Option<String>,
    issue_type: Option<String>,
    priority: Option<String>,
    status: Option<String>,
    epic: Option<u32>,
    label: Option<String>,
    all: bool,
    closed: bool,
) -> Result<()> {
    let issues = load();
    let mut filtered = issues;

    // Folder filtering
    if closed && !all {
        filtered = filtered
            .into_iter()
            .filter(|i| i.folder == "closed")
            .collect();
    } else if !all && !closed {
        filtered = filtered
            .into_iter()
            .filter(|i| i.folder == "open")
            .collect();
    }

    // Field filters
    if let Some(a) = assignee {
        let a_lower = a.to_lowercase();
        filtered = filtered
            .into_iter()
            .filter(|i| i.effective_assignee().to_lowercase() == a_lower)
            .collect();
    }
    if let Some(t) = issue_type {
        filtered = filtered.into_iter().filter(|i| i.issue_type == t).collect();
    }
    if let Some(p) = priority {
        filtered = filtered.into_iter().filter(|i| i.priority == p).collect();
    }
    if let Some(s) = status {
        filtered = filtered.into_iter().filter(|i| i.status == s).collect();
    }
    if let Some(e) = epic {
        filtered = filtered.into_iter().filter(|i| i.epic == Some(e)).collect();
    }
    if let Some(l) = label {
        let l_lower = l.to_lowercase();
        filtered = filtered
            .into_iter()
            .filter(|i| {
                i.labels
                    .as_ref()
                    .map(|lbs| lbs.iter().any(|lb| lb.to_lowercase() == l_lower))
                    .unwrap_or(false)
            })
            .collect();
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
    } else {
        print_issue_table(&filtered);
    }

    Ok(())
}

fn cmd_show(json: bool, number: u32) -> Result<()> {
    let issues = load();
    let issue = issues.iter().find(|i| i.number == number);

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
            eprintln!("Error: issue #{number} not found");
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

    filtered.sort_by_key(|i| i.number);

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
    epic: Option<u32>,
    labels: Vec<String>,
    related: Vec<String>,
    source: Option<String>,
    description: Option<String>,
}

struct NewOutcome {
    number: u32,
    title: String,
    item_path: PathBuf,
}

fn cmd_new(args: NewArgs) -> Result<()> {
    let root = find_root();
    let out = do_new(&root, args)?;
    println!("Created #{}: {}", out.number, out.title);
    println!("  {}", out.item_path.display());
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

    let highest = repo::find_highest_number(root);
    let number = highest + 1;

    let slug = match &args.slug {
        Some(s) => write::slugify(s, 10),
        None => write::slugify(&args.title, 6),
    };
    if slug.is_empty() {
        bail!(
            "could not derive a slug from title {:?}; pass --slug to override",
            args.title
        );
    }

    let dir = write::issue_dir(root, "open", number, &slug);
    if dir.exists() {
        bail!("target directory already exists: {}", dir.display());
    }

    let render = write::render_new_item(&write::NewIssueArgs {
        title: &args.title,
        issue_type: &args.issue_type,
        priority: &args.priority,
        reporter: args.reporter.as_deref(),
        assignee: args.assignee.as_deref(),
        owner: args.owner.as_deref(),
        epic: args.epic,
        labels: &args.labels,
        related: &related,
        source: args.source.as_deref(),
        description: args.description.as_deref(),
    });

    fs::create_dir_all(&dir)
        .with_context(|| format!("cannot create {}", dir.display()))?;
    let item_path = dir.join("item.md");
    fs::write(&item_path, render)
        .with_context(|| format!("cannot write {}", item_path.display()))?;

    Ok(NewOutcome {
        number,
        title: args.title,
        item_path,
    })
}

#[derive(Default)]
struct UpdateArgs {
    number: u32,
    status: Option<String>,
    assignee: Option<String>,
    owner: Option<String>,
    priority: Option<String>,
    epic: Option<u32>,
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

fn cmd_update(args: UpdateArgs) -> Result<()> {
    let root = find_root();
    let number = args.number;
    let out = do_update(&root, args)?;
    if out.moved_to_closed {
        println!("Updated #{}: moved to {}", number, out.final_dir.display());
        println!("  status set to closing — moved to closed/");
    } else if out.moved_to_open {
        println!(
            "Updated #{}: re-opened, moved to {}",
            number,
            out.final_dir.display()
        );
    } else {
        println!("Updated #{number}");
    }
    Ok(())
}

fn do_update(root: &Path, args: UpdateArgs) -> Result<UpdateOutcome> {
    let (folder, slug, item_path) = locate_issue(root, args.number)?;
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
        write::set_u32(&mut item.frontmatter, "epic", e);
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
        let new_dir = write::issue_dir(root, &new_folder, args.number, &slug);
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
            format!(
                "cannot move {} to {}",
                old_dir.display(),
                new_dir.display()
            )
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

fn cmd_close(number: u32, status: Option<String>, commits: Vec<String>) -> Result<()> {
    let root = find_root();
    let out = do_close(&root, number, status, commits)?;
    if out.moved_to_closed {
        println!("Closed #{}: moved to {}", number, out.final_dir.display());
    } else {
        println!("Updated #{number}");
    }
    Ok(())
}

fn do_close(
    root: &Path,
    number: u32,
    status: Option<String>,
    commits: Vec<String>,
) -> Result<UpdateOutcome> {
    let (folder, _slug, item_path) = locate_issue(root, number)?;
    if folder == "closed" {
        bail!("issue #{number} is already in closed/ (use `update` to change status)");
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
            number,
            status: Some(resolved_status),
            add_commits: commits,
            ..Default::default()
        },
    )
}

fn locate_issue(root: &Path, number: u32) -> Result<(String, String, PathBuf)> {
    for folder in &["open", "closed"] {
        let folder_path = root.join("issues").join(folder);
        if !folder_path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&folder_path)
            .with_context(|| format!("cannot read {}", folder_path.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let Some((num, slug)) = parser::parse_issue_dir(&name) else {
                continue;
            };
            if num == number {
                let item = entry.path().join("item.md");
                if !item.is_file() {
                    bail!("#{number} directory has no item.md: {}", item.display());
                }
                return Ok((folder.to_string(), slug, item));
            }
        }
    }
    bail!("issue #{number} not found in issues/open/ or issues/closed/")
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

fn normalize_related_refs(refs: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        let trimmed = r.trim();
        let stripped = trimmed.strip_prefix('#').unwrap_or(trimmed);
        if stripped.is_empty() || !stripped.chars().all(|c| c.is_ascii_digit()) {
            bail!(
                "related reference must look like #NN or NN, got {:?}",
                r
            );
        }
        out.push(format!("#{stripped}"));
    }
    Ok(out)
}

fn cmd_renumber(dry_run: bool, scopes: Vec<PathBuf>) -> Result<()> {
    let root = find_root();
    let plans = build_renumber_plan(&root)?;

    if plans.is_empty() {
        println!("No issues found.");
        return Ok(());
    }

    let number_map = build_number_map(&plans);
    let dir_map = build_dir_map(&plans);
    let ambiguous = ambiguous_numbers(&number_map);

    let scopes = if scopes.is_empty() {
        default_renumber_scopes(&root)
    } else {
        scopes
            .into_iter()
            .map(|p| if p.is_absolute() { p } else { root.join(p) })
            .collect()
    };

    print_renumber_plan(&plans, &dir_map, &ambiguous, &scopes, &root);

    if dry_run {
        println!();
        println!("Dry run — no files modified. Re-run without --dry-run to apply.");
        return Ok(());
    }

    let changed_files = rewrite_markdown_in_scopes(&scopes, &number_map, &dir_map)?;
    rename_issue_dirs_changed_only(&plans)?;

    let changed_dirs = plans
        .iter()
        .filter(|p| p.old_number != p.new_number)
        .count();
    println!();
    println!(
        "Done. {} item(s) renumbered ({} kept), {} dir(s) renamed, {} markdown file(s) rewritten.",
        changed_dirs,
        plans.len() - changed_dirs,
        changed_dirs,
        changed_files,
    );

    if !ambiguous.is_empty() {
        println!();
        println!("Manual cleanup needed for ambiguous references:");
        for old in &ambiguous {
            let new_numbers: Vec<u32> = number_map
                .get(old)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default();
            let mapping = new_numbers
                .iter()
                .map(|n| if n == old { format!("#{n} (kept)") } else { format!("#{n}") })
                .collect::<Vec<_>>()
                .join(" + ");
            println!("  #{old} now maps to: {mapping}");
        }
        println!();
        println!(
            "  Body-text and frontmatter references to these old numbers were left\n  \
             unchanged. Find them with: rg -n '#({})\\b'",
            ambiguous
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join("|")
        );
    }

    Ok(())
}

fn default_renumber_scopes(root: &Path) -> Vec<PathBuf> {
    vec![root.to_path_buf()]
}

fn print_renumber_plan(
    plans: &[RenumberPlan],
    dir_map: &BTreeMap<String, String>,
    ambiguous: &BTreeSet<u32>,
    scopes: &[PathBuf],
    root: &Path,
) {
    let changed: Vec<_> = plans.iter().filter(|p| p.old_number != p.new_number).collect();
    let kept = plans.len() - changed.len();

    println!("Plan ({} items: {} keep their numbers, {} will be renumbered):", plans.len(), kept, changed.len());
    if changed.is_empty() {
        println!("  (no duplicate numbers found — nothing to renumber)");
    } else {
        for plan in &changed {
            println!(
                "  #{:<4} → #{:<4}  {}",
                plan.old_number, plan.new_number, plan.new_dir_name
            );
        }
    }

    if !ambiguous.is_empty() {
        println!();
        println!(
            "Ambiguous: {} old number(s) had multiple dirs; references to these may need manual review.",
            ambiguous.len()
        );
    }

    println!();
    println!("Scopes for reference rewriting (.md files only):");
    for s in scopes {
        let display = s.strip_prefix(root).unwrap_or(s);
        let display_str = if display.as_os_str().is_empty() {
            ".".to_string()
        } else {
            display.display().to_string()
        };
        println!("  {display_str}");
    }
    if !dir_map.is_empty() {
        println!("Directory paths to rewrite: {}", dir_map.len());
    }
}

fn rewrite_markdown_in_scopes(
    scopes: &[PathBuf],
    number_map: &BTreeMap<u32, BTreeSet<u32>>,
    dir_map: &BTreeMap<String, String>,
) -> Result<usize> {
    let mut changed = 0usize;
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    for scope in scopes {
        if !scope.exists() {
            continue;
        }
        let files = if scope.is_file() {
            vec![scope.clone()]
        } else {
            collect_markdown_files_filtered(scope)?
        };
        for path in files {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !seen.insert(canonical) {
                continue;
            }
            let original = fs::read_to_string(&path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            let rewritten = rewrite_issue_text(&original, number_map, dir_map);
            if rewritten != original {
                fs::write(&path, rewritten)
                    .with_context(|| format!("cannot write {}", path.display()))?;
                changed += 1;
            }
        }
    }
    Ok(changed)
}

fn collect_markdown_files_filtered(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk_markdown(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_markdown(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type()?.is_dir() {
            if matches!(
                name.as_str(),
                ".git" | "target" | "node_modules" | ".cargo" | "dist" | "build"
            ) {
                continue;
            }
            walk_markdown(&path, out)?;
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("md"))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
    Ok(())
}

fn rename_issue_dirs_changed_only(plans: &[RenumberPlan]) -> Result<()> {
    let changed: Vec<&RenumberPlan> = plans
        .iter()
        .filter(|p| p.old_dir_name != p.new_dir_name)
        .collect();
    if changed.is_empty() {
        return Ok(());
    }
    for plan in &changed {
        if plan.temp_path.exists() {
            anyhow::bail!(
                "temporary path already exists: {}",
                plan.temp_path.display()
            );
        }
        fs::rename(&plan.path, &plan.temp_path).with_context(|| {
            format!(
                "cannot move {} to {}",
                plan.path.display(),
                plan.temp_path.display()
            )
        })?;
    }
    for plan in &changed {
        if plan.target_path.exists() {
            anyhow::bail!("target path already exists: {}", plan.target_path.display());
        }
        fs::rename(&plan.temp_path, &plan.target_path).with_context(|| {
            format!(
                "cannot move {} to {}",
                plan.temp_path.display(),
                plan.target_path.display()
            )
        })?;
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

#[derive(Debug, Clone)]
struct RenumberPlan {
    old_number: u32,
    new_number: u32,
    old_dir_name: String,
    new_dir_name: String,
    path: PathBuf,
    target_path: PathBuf,
    temp_path: PathBuf,
}

fn build_renumber_plan(root: &Path) -> Result<Vec<RenumberPlan>> {
    let issues_dir = root.join("issues");
    let mut items = Vec::new();

    for folder in ["open", "closed"] {
        let folder_path = issues_dir.join(folder);
        if !folder_path.is_dir() {
            continue;
        }

        for entry in fs::read_dir(&folder_path)
            .with_context(|| format!("cannot read {}", folder_path.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let Some((old_number, slug)) = parser::parse_issue_dir(&dir_name) else {
                continue;
            };
            items.push((old_number, folder.to_string(), slug, entry.path()));
        }
    }

    items.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
    });

    // Preserve-unique + spill-duplicates algorithm:
    //   - Each unique old_number keeps its number (no rename)
    //   - For duplicates (multiple dirs with the same old_number), the first
    //     by sort order keeps the number; subsequent ones get fresh numbers
    //     above the current max.
    // This minimizes churn: only conflicting directories move, and references
    // to non-duplicate numbers stay valid.
    let mut max_number: u32 = items.iter().map(|i| i.0).max().unwrap_or(0);
    let temp_suffix = format!(".issuectl-renumber-{}", std::process::id());

    let mut groups: BTreeMap<u32, Vec<(String, String, PathBuf)>> = BTreeMap::new();
    for (old_number, folder, slug, path) in items {
        groups
            .entry(old_number)
            .or_default()
            .push((folder, slug, path));
    }

    let mut plans = Vec::new();
    for (old_number, group) in groups {
        for (i, (folder, slug, path)) in group.into_iter().enumerate() {
            let new_number = if i == 0 {
                old_number
            } else {
                max_number += 1;
                max_number
            };
            let folder_path = issues_dir.join(&folder);
            let old_dir_name = format!("{old_number}-{slug}");
            let new_dir_name = format!("{new_number}-{slug}");
            plans.push(RenumberPlan {
                old_number,
                new_number,
                old_dir_name,
                new_dir_name: new_dir_name.clone(),
                target_path: folder_path.join(new_dir_name),
                temp_path: folder_path.join(format!("{old_number}-{slug}{temp_suffix}")),
                path,
            });
        }
    }

    plans.sort_by_key(|p| (p.old_number, p.new_number));
    Ok(plans)
}

fn build_number_map(plans: &[RenumberPlan]) -> BTreeMap<u32, BTreeSet<u32>> {
    let mut number_map: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for plan in plans {
        number_map
            .entry(plan.old_number)
            .or_default()
            .insert(plan.new_number);
    }
    number_map
}

fn build_dir_map(plans: &[RenumberPlan]) -> BTreeMap<String, String> {
    plans
        .iter()
        .filter(|plan| plan.old_dir_name != plan.new_dir_name)
        .map(|plan| (plan.old_dir_name.clone(), plan.new_dir_name.clone()))
        .collect()
}

fn ambiguous_numbers(number_map: &BTreeMap<u32, BTreeSet<u32>>) -> BTreeSet<u32> {
    number_map
        .iter()
        .filter_map(|(old, new)| if new.len() > 1 { Some(*old) } else { None })
        .collect()
}

fn rewrite_issue_text(
    text: &str,
    number_map: &BTreeMap<u32, BTreeSet<u32>>,
    dir_map: &BTreeMap<String, String>,
) -> String {
    let heading_re = Regex::new(r"^(# )E?\d+\.\s+(.+)$").expect("valid heading regex");
    let epic_re = Regex::new(r"^(\s*epic:\s*)(\d+)(.*)$").expect("valid epic regex");
    let ref_re = Regex::new(r"#(\d+)\b").expect("valid reference regex");
    let dir_regexes = compile_dir_regexes(dir_map);

    let mut rewritten = Vec::new();
    for line in text.lines() {
        let line = heading_re.replace(line, "$1$2").to_string();
        let line = epic_re
            .replace(&line, |caps: &Captures| {
                let old = caps[2].parse::<u32>().ok();
                if let Some(new_number) = old.and_then(|n| mapped_number(n, number_map)) {
                    format!("{}{}{}", &caps[1], new_number, &caps[3])
                } else {
                    caps[0].to_string()
                }
            })
            .to_string();
        let line = ref_re
            .replace_all(&line, |caps: &Captures| {
                let old = caps[1].parse::<u32>().ok();
                if let Some(new_number) = old.and_then(|n| mapped_number(n, number_map)) {
                    format!("#{new_number}")
                } else {
                    caps[0].to_string()
                }
            })
            .to_string();
        let line = rewrite_issue_dir_paths(&line, &dir_regexes);
        rewritten.push(line);
    }

    let mut output = rewritten.join("\n");
    if text.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn compile_dir_regexes(dir_map: &BTreeMap<String, String>) -> Vec<(Regex, String)> {
    dir_map
        .iter()
        .map(|(old_dir, new_dir)| {
            let pattern = format!(
                r"(^|[^A-Za-z0-9_-]){}($|[^A-Za-z0-9_-])",
                regex::escape(old_dir)
            );
            (
                Regex::new(&pattern).expect("valid directory path regex"),
                new_dir.clone(),
            )
        })
        .collect()
}

fn rewrite_issue_dir_paths(text: &str, regexes: &[(Regex, String)]) -> String {
    let mut rewritten = text.to_string();
    for (dir_re, new_dir) in regexes {
        rewritten = dir_re
            .replace_all(&rewritten, |caps: &Captures| {
                format!("{}{}{}", &caps[1], new_dir, &caps[2])
            })
            .to_string();
    }
    rewritten
}

fn mapped_number(old: u32, number_map: &BTreeMap<u32, BTreeSet<u32>>) -> Option<u32> {
    let numbers = number_map.get(&old)?;
    if numbers.len() == 1 {
        numbers.iter().next().copied()
    } else {
        None
    }
}

// ── Display helpers ─────────────────────────────────────────────────────────

const TABLE_HEADERS: &[&str] = &["#", "Title", "Type", "Status", "Pri", "Assignee"];

fn print_issue_table(issues: &[models::Issue]) {
    if issues.is_empty() {
        println!("No issues found.");
        return;
    }

    let rows: Vec<Vec<String>> = issues
        .iter()
        .map(|i| {
            vec![
                i.number.to_string(),
                truncate(&i.title, 50),
                i.issue_type.clone(),
                i.status.clone(),
                i.priority.clone(),
                i.effective_assignee().to_string(),
            ]
        })
        .collect();

    // Calculate column widths
    let mut widths: Vec<usize> = TABLE_HEADERS.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (j, cell) in row.iter().enumerate() {
            widths[j] = widths[j].max(cell.len());
        }
    }

    // Header
    let header: String = TABLE_HEADERS
        .iter()
        .enumerate()
        .map(|(j, h)| format!("{:width$}", h, width = widths[j] + 1))
        .collect::<Vec<_>>()
        .join("");
    println!("{}", header.trim_end());

    // Separator
    let sep: String = widths
        .iter()
        .map(|w| "─".repeat(*w + 1))
        .collect::<Vec<_>>()
        .join("");
    println!("{}", sep.trim_end());

    // Rows
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
    println!("#{}  {}", issue.number, issue.title);
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
    if let Some(e) = issue.epic {
        println!("Epic:     #{}", e);
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
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
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
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    for (key, count) in sorted {
        println!("  {:20} {}", key, count);
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(u32, &[u32])]) -> BTreeMap<u32, BTreeSet<u32>> {
        pairs
            .iter()
            .map(|(old, new)| (*old, new.iter().copied().collect()))
            .collect()
    }

    fn dirs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(old, new)| (old.to_string(), new.to_string()))
            .collect()
    }

    #[test]
    fn rewrites_frontmatter_references_and_legacy_heading() {
        let number_map = map(&[(10, &[1]), (12, &[2]), (13, &[3])]);
        let text = "\
---
epic: 10
related: [\"#12\", \"#13\"]
---

# E10. Epic title

Continues #12 and blocks **#13**.
";

        let rewritten = rewrite_issue_text(text, &number_map, &BTreeMap::new());

        assert!(rewritten.contains("epic: 1\n"));
        assert!(rewritten.contains("related: [\"#2\", \"#3\"]"));
        assert!(rewritten.contains("# Epic title\n"));
        assert!(rewritten.contains("Continues #2 and blocks **#3**."));
    }

    #[test]
    fn leaves_ambiguous_duplicate_references_unchanged() {
        let number_map = map(&[(7, &[1, 2])]);
        let text = "epic: 7\nrelated: [\"#7\"]\n# 7. Title\nSee #7.\n";

        let rewritten = rewrite_issue_text(text, &number_map, &BTreeMap::new());

        assert!(rewritten.contains("epic: 7\n"));
        assert!(rewritten.contains("related: [\"#7\"]"));
        assert!(rewritten.contains("# Title\n"));
        assert!(rewritten.contains("See #7."));
    }

    #[test]
    fn rewrites_markdown_link_display_and_issue_dir_path() {
        let number_map = map(&[(75, &[93])]);
        let dir_map = dirs(&[(
            "57-auditoitavuus-asiantuntijanakyma",
            "72-auditoitavuus-asiantuntijanakyma",
        )]);
        let text = "[#75](../57-auditoitavuus-asiantuntijanakyma/item.md) not 157-auditoitavuus-asiantuntijanakyma";

        let rewritten = rewrite_issue_text(text, &number_map, &dir_map);

        assert_eq!(
            rewritten,
            "[#93](../72-auditoitavuus-asiantuntijanakyma/item.md) not 157-auditoitavuus-asiantuntijanakyma"
        );
    }

    use std::fs;
    use tempfile::TempDir;

    fn make_repo_with_dirs(specs: &[(&str, u32, &str)]) -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        fs::create_dir_all(tmp.path().join("issues/closed")).unwrap();
        for (folder, num, slug) in specs {
            let dir = tmp
                .path()
                .join("issues")
                .join(folder)
                .join(format!("{num}-{slug}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                format!("---\nstatus: open\n---\n\n# {slug}\n"),
            )
            .unwrap();
        }
        tmp
    }

    #[test]
    fn renumber_plan_preserves_unique_numbers() {
        let tmp = make_repo_with_dirs(&[
            ("open", 1, "first"),
            ("open", 5, "fifth"),
            ("closed", 10, "tenth"),
        ]);
        let plans = build_renumber_plan(tmp.path()).unwrap();
        for plan in &plans {
            assert_eq!(plan.old_number, plan.new_number, "{plan:?}");
        }
    }

    #[test]
    fn renumber_plan_spills_duplicates_above_max() {
        // Three issues share #14, max number is 50.
        let tmp = make_repo_with_dirs(&[
            ("open", 14, "alpha"),
            ("open", 14, "beta"),
            ("closed", 14, "gamma"),
            ("open", 50, "max-issue"),
        ]);
        let plans = build_renumber_plan(tmp.path()).unwrap();

        let kept_14: Vec<_> = plans
            .iter()
            .filter(|p| p.old_number == 14 && p.new_number == 14)
            .collect();
        assert_eq!(kept_14.len(), 1, "exactly one #14 should keep its number");

        let spilled: Vec<_> = plans
            .iter()
            .filter(|p| p.old_number == 14 && p.new_number != 14)
            .collect();
        assert_eq!(spilled.len(), 2, "two #14 sources should spill");
        for plan in &spilled {
            assert!(
                plan.new_number > 50,
                "spilled number {} should be above max 50",
                plan.new_number
            );
        }

        let max_plan = plans.iter().find(|p| p.old_number == 50).unwrap();
        assert_eq!(max_plan.new_number, 50, "max should be unchanged");
    }

    #[test]
    fn renumber_plan_no_renames_when_all_unique() {
        let tmp = make_repo_with_dirs(&[
            ("open", 1, "a"),
            ("open", 2, "b"),
            ("closed", 3, "c"),
        ]);
        let plans = build_renumber_plan(tmp.path()).unwrap();
        let dir_map = build_dir_map(&plans);
        assert!(
            dir_map.is_empty(),
            "no renames expected when no duplicates: {dir_map:?}"
        );
    }

    #[test]
    fn renumber_dry_run_does_not_modify_filesystem() {
        let tmp = make_repo_with_dirs(&[
            ("open", 1, "a"),
            ("open", 1, "b"),
        ]);
        let before: Vec<String> = fs::read_dir(tmp.path().join("issues/open"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();

        // Drive cmd_renumber through ROOT_OVERRIDE.
        let _ = ROOT_OVERRIDE.set(Some(tmp.path().to_path_buf()));
        cmd_renumber(true, vec![]).unwrap();

        let after: Vec<String> = fs::read_dir(tmp.path().join("issues/open"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        let mut before_sorted = before.clone();
        let mut after_sorted = after.clone();
        before_sorted.sort();
        after_sorted.sort();
        assert_eq!(before_sorted, after_sorted, "dry-run must not change anything");
    }
}

#[cfg(test)]
mod helper_tests {
    use super::*;

    // ── parse_non_empty ────────────────────────────────────────────────────

    #[test]
    fn parse_non_empty_accepts_normal_value() {
        assert_eq!(parse_non_empty("hello").unwrap(), "hello");
        assert_eq!(parse_non_empty("a").unwrap(), "a");
    }

    #[test]
    fn parse_non_empty_rejects_empty() {
        assert!(parse_non_empty("").is_err());
    }

    #[test]
    fn parse_non_empty_rejects_whitespace_only() {
        assert!(parse_non_empty("   ").is_err());
        assert!(parse_non_empty("\t").is_err());
    }

    #[test]
    fn parse_non_empty_rejects_padded_value() {
        assert!(parse_non_empty(" hello").is_err());
        assert!(parse_non_empty("hello ").is_err());
        assert!(parse_non_empty(" hello ").is_err());
    }

    // ── is_closing_status / all_statuses ───────────────────────────────────

    #[test]
    fn is_closing_status_classifies_correctly() {
        for s in CLOSING_STATUSES {
            assert!(is_closing_status(s), "{s} should be closing");
        }
        for s in ACTIVE_STATUSES {
            assert!(!is_closing_status(s), "{s} should not be closing");
        }
        assert!(!is_closing_status("nonsense"));
    }

    #[test]
    fn all_statuses_includes_active_and_closing() {
        let s = all_statuses();
        for v in ACTIVE_STATUSES.iter().chain(CLOSING_STATUSES.iter()) {
            assert!(s.contains(v), "{v} should be in all_statuses");
        }
    }

    // ── parse_commit_spec ──────────────────────────────────────────────────

    #[test]
    fn parse_commit_spec_basic() {
        assert_eq!(
            parse_commit_spec("abc123:fix login").unwrap(),
            ("abc123".to_string(), "fix login".to_string())
        );
    }

    #[test]
    fn parse_commit_spec_trims_components() {
        assert_eq!(
            parse_commit_spec("  abc123  :  fix login  ").unwrap(),
            ("abc123".to_string(), "fix login".to_string())
        );
    }

    #[test]
    fn parse_commit_spec_rejects_no_colon() {
        assert!(parse_commit_spec("abc123 fix").is_err());
    }

    #[test]
    fn parse_commit_spec_rejects_empty_hash() {
        assert!(parse_commit_spec(":fix").is_err());
        assert!(parse_commit_spec("  :fix").is_err());
    }

    #[test]
    fn parse_commit_spec_rejects_empty_summary() {
        assert!(parse_commit_spec("abc:").is_err());
        assert!(parse_commit_spec("abc:  ").is_err());
    }

    #[test]
    fn parse_commit_spec_keeps_subsequent_colons_in_summary() {
        // First colon splits; rest is part of summary
        assert_eq!(
            parse_commit_spec("abc:fix: nested colon").unwrap(),
            ("abc".to_string(), "fix: nested colon".to_string())
        );
    }

    // ── normalize_related_refs ─────────────────────────────────────────────

    #[test]
    fn normalize_related_adds_hash_prefix() {
        assert_eq!(
            normalize_related_refs(&["12".to_string()]).unwrap(),
            vec!["#12".to_string()]
        );
    }

    #[test]
    fn normalize_related_preserves_existing_hash() {
        assert_eq!(
            normalize_related_refs(&["#7".to_string()]).unwrap(),
            vec!["#7".to_string()]
        );
    }

    #[test]
    fn normalize_related_handles_multiple() {
        assert_eq!(
            normalize_related_refs(&["#3".to_string(), "9".to_string()]).unwrap(),
            vec!["#3".to_string(), "#9".to_string()]
        );
    }

    #[test]
    fn normalize_related_rejects_non_numeric() {
        assert!(normalize_related_refs(&["foo".to_string()]).is_err());
        assert!(normalize_related_refs(&["3a".to_string()]).is_err());
        assert!(normalize_related_refs(&["#abc".to_string()]).is_err());
    }

    #[test]
    fn normalize_related_rejects_empty() {
        assert!(normalize_related_refs(&["".to_string()]).is_err());
        assert!(normalize_related_refs(&["#".to_string()]).is_err());
    }

    #[test]
    fn normalize_related_empty_input_is_ok() {
        assert_eq!(normalize_related_refs(&[]).unwrap(), Vec::<String>::new());
    }
}

#[cfg(test)]
mod cmd_tests {
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

    // ── do_new ─────────────────────────────────────────────────────────────

    #[test]
    fn new_creates_first_issue_with_number_one() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "First bug");
        args.reporter = Some("alice".into());
        args.assignee = Some("bob".into());
        let out = do_new(tmp.path(), args).unwrap();
        assert_eq!(out.number, 1);
        assert!(out.item_path.exists());
        let content = read(&out.item_path);
        assert!(content.contains("type: bug"));
        assert!(content.contains("reporter: alice"));
        assert!(content.contains("assignee: bob"));
        assert!(content.contains("status: open"));
        assert!(content.contains("priority: normal"));
        assert!(content.contains("# First bug"));
    }

    #[test]
    fn new_increments_number_across_open_and_closed() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/closed/5-foo")).unwrap();
        fs::create_dir_all(tmp.path().join("issues/open/2-bar")).unwrap();
        let out = do_new(tmp.path(), new_args("task", "Next thing")).unwrap();
        assert_eq!(out.number, 6);
    }

    #[test]
    fn new_uses_auto_slug_from_title() {
        let tmp = fresh_repo();
        let out = do_new(tmp.path(), new_args("bug", "Login Redirect Loops")).unwrap();
        assert!(out.item_path.to_string_lossy().contains("1-login-redirect-loops"));
    }

    #[test]
    fn new_honors_explicit_slug_override() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "Some Long And Detailed Title");
        args.slug = Some("custom".into());
        let out = do_new(tmp.path(), args).unwrap();
        assert!(out.item_path.to_string_lossy().contains("1-custom"));
    }

    #[test]
    fn new_rejects_unsluggable_title() {
        let tmp = fresh_repo();
        let result = do_new(tmp.path(), new_args("bug", "!!!"));
        assert!(result.is_err());
    }

    #[test]
    fn new_rejects_epic_with_reporter() {
        let tmp = fresh_repo();
        let mut args = new_args("epic", "API v2");
        args.reporter = Some("alice".into());
        assert!(do_new(tmp.path(), args).is_err());
    }

    #[test]
    fn new_rejects_epic_with_assignee() {
        let tmp = fresh_repo();
        let mut args = new_args("epic", "API v2");
        args.assignee = Some("alice".into());
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
        args.priority = "high".into();
        let out = do_new(tmp.path(), args).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("type: epic"));
        assert!(content.contains("owner: cara"));
        assert!(content.contains("priority: high"));
        assert!(!content.contains("reporter:"));
        assert!(!content.contains("assignee:"));
    }

    #[test]
    fn new_normalizes_related_without_hash() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "B");
        args.related = vec!["12".into(), "#7".into()];
        let out = do_new(tmp.path(), args).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("'#12'") || content.contains("\"#12\""));
        assert!(content.contains("'#7'") || content.contains("\"#7\""));
    }

    #[test]
    fn new_rejects_invalid_related_ref() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "B");
        args.related = vec!["not-a-number".into()];
        assert!(do_new(tmp.path(), args).is_err());
    }

    #[test]
    fn new_writes_source_and_description() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "B");
        args.source = Some("frontend/login".into());
        args.description = Some("Stuck in a loop.".into());
        let out = do_new(tmp.path(), args).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("_Source: frontend/login_"));
        assert!(content.contains("Stuck in a loop."));
    }

    // ── do_update ──────────────────────────────────────────────────────────

    fn make_one(tmp: &TempDir, t: &str, title: &str) -> NewOutcome {
        let mut a = new_args(t, title);
        if t != "epic" {
            a.reporter = Some("rep".into());
            a.assignee = Some("ass".into());
        } else {
            a.owner = Some("own".into());
        }
        do_new(tmp.path(), a).unwrap()
    }

    #[test]
    fn update_sets_status_and_bumps_updated() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                status: Some("in-progress".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(content.contains("status: in-progress"));
    }

    #[test]
    fn update_with_closing_status_moves_to_closed_and_stamps_date() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        let outcome = do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                status: Some("fixed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(outcome.moved_to_closed);
        assert!(!outcome.moved_to_open);
        let new_path = outcome.final_dir.join("item.md");
        assert!(new_path.exists());
        assert!(!n.item_path.exists());
        let content = read(&new_path);
        assert!(content.contains("status: fixed"));
        assert!(content.contains("closed: "));
    }

    #[test]
    fn update_active_status_on_closed_reopens_and_clears_closed_date() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        // Close first
        do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                status: Some("fixed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        // Re-open
        let outcome = do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                status: Some("in-progress".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(outcome.moved_to_open);
        assert!(!outcome.moved_to_closed);
        let new_path = outcome.final_dir.join("item.md");
        assert!(new_path.exists());
        let content = read(&new_path);
        assert!(content.contains("status: in-progress"));
        assert!(!content.contains("closed:"));
    }

    #[test]
    fn update_closing_status_on_already_closed_stays_in_closed() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                status: Some("fixed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let outcome = do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                status: Some("wontfix".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!outcome.moved_to_closed);
        assert!(!outcome.moved_to_open);
        let content = read(&outcome.final_dir.join("item.md"));
        assert!(content.contains("status: wontfix"));
    }

    #[test]
    fn update_no_epic_clears_field() {
        let tmp = fresh_repo();
        let mut a = new_args("task", "T");
        a.epic = Some(99);
        let n = do_new(tmp.path(), a).unwrap();
        do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                no_epic: true,
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(!content.contains("epic:"));
    }

    #[test]
    fn update_set_epic_replaces_value() {
        let tmp = fresh_repo();
        let mut a = new_args("task", "T");
        a.epic = Some(5);
        let n = do_new(tmp.path(), a).unwrap();
        do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                epic: Some(11),
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(content.contains("epic: 11"));
        assert!(!content.contains("epic: 5"));
    }

    #[test]
    fn update_label_add_remove_and_drop_when_empty() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                add_labels: vec!["frontend".into(), "auth".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(read(&n.item_path).contains("labels: [frontend, auth]"));
        do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                remove_labels: vec!["frontend".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(read(&n.item_path).contains("labels: [auth]"));
        do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                remove_labels: vec!["auth".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(!content.contains("labels:"));
    }

    #[test]
    fn update_add_related_normalizes_and_dedupes() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                add_related: vec!["3".into(), "#3".into(), "7".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(content.contains("related: ['#3', '#7']") || content.contains("related: [\"#3\", \"#7\"]"));
    }

    #[test]
    fn update_add_commit_appends() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                add_commits: vec!["abc:fix login".into(), "def:tests".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(content.contains("hash: abc"));
        assert!(content.contains("summary: fix login"));
        assert!(content.contains("hash: def"));
        assert!(content.contains("summary: tests"));
    }

    #[test]
    fn update_rejects_bad_commit_spec() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        let result = do_update(
            tmp.path(),
            UpdateArgs {
                number: n.number,
                add_commits: vec!["nohash".into()],
                ..Default::default()
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn update_rejects_unknown_issue() {
        let tmp = fresh_repo();
        let result = do_update(
            tmp.path(),
            UpdateArgs {
                number: 999,
                status: Some("done".into()),
                ..Default::default()
            },
        );
        assert!(result.is_err());
    }

    // ── do_close ───────────────────────────────────────────────────────────

    #[test]
    fn close_defaults_to_fixed_for_bug() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        let outcome = do_close(tmp.path(), n.number, None, vec![]).unwrap();
        assert!(outcome.moved_to_closed);
        let content = read(&outcome.final_dir.join("item.md"));
        assert!(content.contains("status: fixed"));
    }

    #[test]
    fn close_defaults_to_done_for_task() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "task", "Task");
        let outcome = do_close(tmp.path(), n.number, None, vec![]).unwrap();
        let content = read(&outcome.final_dir.join("item.md"));
        assert!(content.contains("status: done"));
    }

    #[test]
    fn close_defaults_to_done_for_epic() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "epic", "Epic");
        let outcome = do_close(tmp.path(), n.number, None, vec![]).unwrap();
        let content = read(&outcome.final_dir.join("item.md"));
        assert!(content.contains("status: done"));
    }

    #[test]
    fn close_honors_explicit_status() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        let outcome =
            do_close(tmp.path(), n.number, Some("wontfix".into()), vec![]).unwrap();
        let content = read(&outcome.final_dir.join("item.md"));
        assert!(content.contains("status: wontfix"));
    }

    #[test]
    fn close_records_commit() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        do_close(
            tmp.path(),
            n.number,
            None,
            vec!["abc:final fix".into()],
        )
        .unwrap();
        let closed = tmp
            .path()
            .join(format!("issues/closed/{}-bug", n.number));
        let content = read(&closed.join("item.md"));
        assert!(content.contains("hash: abc"));
        assert!(content.contains("summary: final fix"));
    }

    #[test]
    fn close_rejects_already_closed() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        do_close(tmp.path(), n.number, None, vec![]).unwrap();
        let result = do_close(tmp.path(), n.number, None, vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn close_rejects_unknown_issue() {
        let tmp = fresh_repo();
        let result = do_close(tmp.path(), 999, None, vec![]);
        assert!(result.is_err());
    }

    // ── locate_issue ───────────────────────────────────────────────────────

    #[test]
    fn locate_issue_finds_in_open() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        let (folder, slug, path) = locate_issue(tmp.path(), n.number).unwrap();
        assert_eq!(folder, "open");
        assert_eq!(slug, "bug");
        assert_eq!(path, n.item_path);
    }

    #[test]
    fn locate_issue_finds_in_closed() {
        let tmp = fresh_repo();
        let n = make_one(&tmp, "bug", "Bug");
        do_close(tmp.path(), n.number, None, vec![]).unwrap();
        let (folder, _, _) = locate_issue(tmp.path(), n.number).unwrap();
        assert_eq!(folder, "closed");
    }

    #[test]
    fn locate_issue_returns_error_for_missing() {
        let tmp = fresh_repo();
        assert!(locate_issue(tmp.path(), 999).is_err());
    }

    #[test]
    fn locate_issue_errors_when_item_md_missing() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/open/3-no-item")).unwrap();
        assert!(locate_issue(tmp.path(), 3).is_err());
    }
}
