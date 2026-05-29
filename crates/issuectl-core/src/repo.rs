use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::models::Issue;
use crate::parser;

const ISSUES_DIR: &str = "issues";
/// Legacy status-folder names walked separately as compat-read paths.
const LEGACY_FOLDERS: &[&str] = &["open", "closed"];
/// Cold-storage root under `issues/`. Closed issues are archived to
/// `issues/archive/YYYY/MM/<slug>/`. The name fails `slug::is_valid`
/// (single segment) so the flat-layout walk never mistakes it for an
/// issue; the loader walks it explicitly as a second read root.
pub const ARCHIVE_DIR: &str = "archive";

/// Stable machine-readable warning codes surfaced via `LoadWarning.code`.
/// The wire format serialises as snake_case strings — clients dispatch on
/// these without scraping `message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadWarningCode {
    /// Issue is at a legacy `issues/{open,closed}/<slug>/` path.
    LegacyLayout,
    /// Slug is present at multiple paths simultaneously (flat + legacy,
    /// or both legacy folders).
    AmbiguousSlug,
    /// `item.md` is missing from an otherwise-existing issue dir.
    MissingItem,
    /// `item.md` exceeds the parser size cap.
    TooLarge,
    /// `item.md` parsed with warnings (bad YAML, partial write, etc.).
    ParseWarning,
    /// `issues/.schema.yaml` failed to parse; readers fell back to the
    /// built-in default schema, which means custom `status_classes`
    /// entries are silently dropped (a custom `archived: closing`
    /// status would behave like an unknown active status until the
    /// schema is fixed).
    SchemaParseError,
}

/// Load the schema for read-side lifecycle classification, or fall
/// back to the built-in default after recording a `SchemaParseError`
/// warning. The fallback path keeps the reader functional (issues
/// still load, the kanban board still renders) but the warning makes
/// the silent feature regression visible — without it, a typo in
/// `.schema.yaml` would invisibly bucket custom-closing statuses as
/// open. Mutations correctly hard-fail on the same error; readers
/// only soft-warn so the UI can keep rendering.
fn load_schema_or_warn(
    repo_root: &Path,
    warnings: Option<&mut Vec<LoadWarning>>,
    config: &dyn crate::repo_config::ConfigSource,
) -> std::sync::Arc<crate::schema::Schema> {
    match config.schema(repo_root) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!(
                "schema parse failed; lifecycle classification falls back to built-in defaults until fixed: {e:#}"
            );
            if let Some(ws) = warnings {
                ws.push(LoadWarning {
                    slug: String::new(),
                    folder: String::new(),
                    message: msg,
                    code: Some(LoadWarningCode::SchemaParseError),
                });
            } else {
                eprintln!("Warning: {msg}");
            }
            std::sync::Arc::new(crate::schema::default_schema())
        }
    }
}

/// Per-slug filesystem state classification. One source of truth used by
/// the loader, locator, watcher, mutate layer, and migrate command — see
/// `resolve_layout`.
#[derive(Debug, Clone)]
pub enum LayoutState {
    /// Slug not present at any candidate path.
    Absent,
    /// Slug at the canonical `issues/<slug>/item.md` path. `item_path` is
    /// validated (no symlinks, contained under issues_root).
    Flat { item_path: PathBuf },
    /// Slug at a legacy `issues/{open,closed}/<slug>/item.md` path.
    /// `legacy_folder` is the kanban-bucket label of where it was found.
    Legacy {
        folder: &'static str,
        item_path: PathBuf,
    },
    /// Slug exists at >1 candidate path; refuse to pick a side.
    Ambiguous { paths: Vec<PathBuf> },
    /// Path-shape rejection — symlinked dir, dir contained outside
    /// issues_root, missing item.md, symlinked item.md, etc. Surfaced as
    /// a warning to the UI but treated as not-loadable.
    Invalid {
        item_path: Option<PathBuf>,
        reason: String,
    },
}

/// Slimmer projection of `Issue` for list endpoints — same fields minus the
/// markdown body. The web board renders cards from frontmatter + title, so
/// shipping bodies in `/api/issues` is wasted bandwidth and parse cost.
#[derive(Debug, Clone, Serialize)]
pub struct IssueSummary {
    pub slug: String,
    pub folder: String,
    pub created: Option<String>,
    pub status: String,
    pub updated: Option<String>,
    pub priority: String,
    #[serde(rename = "type")]
    pub issue_type: String,
    pub reporter: Option<String>,
    pub assignee: Option<String>,
    pub owner: Option<String>,
    pub epic: Option<String>,
    pub related: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,
    pub closed: Option<String>,
    pub commits: Option<Vec<crate::models::Commit>>,
    pub title: String,
    /// Canonical content hash. Lets the web client send `expected_version`
    /// on PATCHes (drag-and-drop kanban writes) without a per-card GET to
    /// fetch the version separately.
    pub version: String,
}

impl From<Issue> for IssueSummary {
    fn from(i: Issue) -> Self {
        let version = crate::canonical::canonical_hash(&i);
        IssueSummary {
            slug: i.slug,
            folder: i.folder,
            created: i.created,
            status: i.status,
            updated: i.updated,
            priority: i.priority,
            issue_type: i.issue_type,
            reporter: i.reporter,
            assignee: i.assignee,
            owner: i.owner,
            epic: i.epic,
            related: i.related,
            labels: i.labels,
            closed: i.closed,
            commits: i.commits,
            title: i.title,
            version,
        }
    }
}

/// Per-file warning collected during a load (e.g., malformed frontmatter,
/// or a slug found at the legacy `issues/open|closed/<slug>/` path).
/// The CLI still emits these to stderr; the web API surfaces them in the
/// listing payload so the UI can flag broken issues rather than silently
/// rendering zombies with default fields.
#[derive(Debug, Clone, Serialize)]
pub struct LoadWarning {
    pub slug: String,
    pub folder: String,
    pub message: String,
    /// Stable machine-readable warning code. `None` for ad-hoc messages
    /// without a stable category; populated when the UI may want to
    /// special-case (e.g. `LegacyLayout`, `AmbiguousSlug`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<LoadWarningCode>,
}

/// Walk up from start (default: cwd) to find repo root.
/// Looks for `issues/` directory first, then falls back to `.git`.
pub fn find_repo_root(start: Option<&Path>) -> PathBuf {
    let start = start.unwrap_or_else(|| Path::new("."));
    let cwd = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());

    for parent in
        std::iter::once(cwd.clone()).chain(cwd.ancestors().skip(1).map(|p| p.to_path_buf()))
    {
        if parent.join(ISSUES_DIR).is_dir() || parent.join(".git").is_dir() {
            return parent;
        }
    }

    eprintln!("Error: cannot find repo root (no issues/ or .git found in any parent directory)");
    std::process::exit(1);
}

/// Folder bucket derived from frontmatter status. The on-disk layout is
/// flat (`issues/<slug>/item.md`); `folder` survives in payloads as a
/// computed kanban-bucket label so existing CLI/web filters keep working.
///
/// Schema-aware: a project that declares a custom closing status (e.g.
/// `archived`) via `status_classes:` in `issues/.schema.yaml` gets that
/// status bucketed as `closed`. Built-in statuses fall back to
/// `issue_fields::is_closing_status` when the schema is silent.
pub fn folder_for_status(schema: &crate::schema::Schema, status: &str) -> &'static str {
    if crate::schema::is_closing(schema, status) {
        "closed"
    } else {
        "open"
    }
}

/// Discover every slug appearing under `issues/` — flat, legacy-open,
/// legacy-closed, or archived (`archive/YYYY/MM/<slug>/`). Returns each
/// slug exactly once; the resolver determines the slug's `LayoutState`.
///
/// Reserved-subdir filter: `open`, `closed`, and `archive` are walked as
/// parents, not as slug directories (`archive` fails `slug::is_valid`
/// anyway, being a single segment). Anything else under `issues/` is
/// surfaced as a slug candidate — the resolver rejects leading-dot or
/// otherwise invalid shapes via `slug::is_valid`.
fn discover_slugs(repo_root: &Path) -> std::collections::BTreeSet<String> {
    use std::collections::BTreeSet;
    let mut slugs = BTreeSet::new();
    let issues_dir = repo_root.join(ISSUES_DIR);

    if let Ok(rd) = std::fs::read_dir(&issues_dir) {
        for entry in rd.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if LEGACY_FOLDERS.contains(&name.as_str()) {
                continue;
            }
            if !crate::slug::is_valid(&name) {
                continue;
            }
            slugs.insert(name);
        }
    }
    for legacy in LEGACY_FOLDERS {
        let dir = issues_dir.join(legacy);
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !crate::slug::is_valid(&name) {
                continue;
            }
            slugs.insert(name);
        }
    }
    for dir in archive_slug_dirs(&issues_dir) {
        if let Some(name) = dir.file_name().map(|n| n.to_string_lossy().to_string()) {
            if crate::slug::is_valid(&name) {
                slugs.insert(name);
            }
        }
    }
    slugs
}

/// Every `issues/archive/YYYY/MM/<slug>/` directory present on disk.
/// Returns the slug-level directories (one level below `MM`). Tolerant of
/// a missing archive root and of stray files at any level — non-dir
/// entries are skipped. The slug shape itself is validated by the caller.
fn archive_slug_dirs(issues_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let archive_root = issues_dir.join(ARCHIVE_DIR);
    let Ok(years) = std::fs::read_dir(&archive_root) else {
        return out;
    };
    for year in years.flatten() {
        if !year.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let Ok(months) = std::fs::read_dir(year.path()) else {
            continue;
        };
        for month in months.flatten() {
            if !month.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let Ok(slug_dirs) = std::fs::read_dir(month.path()) else {
                continue;
            };
            for entry in slug_dirs.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    out.push(entry.path());
                }
            }
        }
    }
    out
}

/// Slug → archive directories map, built by a single walk of the archive
/// tree. Callers that resolve many slugs in a loop (`load_issues`,
/// `rename_issue`, `archive`) build this once and pass it to
/// [`resolve_layout_in`], avoiding an O(N·archive) re-walk per slug.
pub type ArchiveIndex = std::collections::BTreeMap<String, Vec<PathBuf>>;

/// Build the [`ArchiveIndex`] for a repo with one archive-tree walk.
pub fn archive_index(repo_root: &Path) -> ArchiveIndex {
    let issues_dir = repo_root.join(ISSUES_DIR);
    let mut idx: ArchiveIndex = ArchiveIndex::new();
    for dir in archive_slug_dirs(&issues_dir) {
        if let Some(name) = dir.file_name().map(|n| n.to_string_lossy().to_string()) {
            idx.entry(name).or_default().push(dir);
        }
    }
    idx
}

/// All on-disk `issues/archive/*/*/<slug>/` directories for one slug.
/// Normally at most one; more than one means the slug was archived into
/// two month buckets, which `resolve_layout` surfaces as `Ambiguous`.
fn archive_dirs_for(issues_dir: &Path, slug: &str) -> Vec<PathBuf> {
    archive_slug_dirs(issues_dir)
        .into_iter()
        .filter(|d| d.file_name().map(|n| n == slug).unwrap_or(false))
        .collect()
}

/// Relative archive path (`archive/YYYY/MM/<slug>`) for a slug filed
/// under a given `YYYY-MM-DD` date. Falls back to `unknown/unknown`
/// buckets when the date can't be split into year/month — keeps an
/// undated issue archivable rather than refusing the move.
pub fn archive_relpath(slug: &str, date: &str) -> PathBuf {
    let mut parts = date.splitn(3, '-');
    let year = parts.next().filter(|s| !s.is_empty()).unwrap_or("unknown");
    let month = parts.next().filter(|s| !s.is_empty()).unwrap_or("unknown");
    Path::new(ARCHIVE_DIR).join(year).join(month).join(slug)
}

/// Single source of truth for per-slug filesystem-state classification.
/// All five callers (loader, locator, watcher, mutate, migrate) consume
/// `LayoutState` and apply their own policy on top — never re-implement
/// the tri-path detection.
///
/// The classification is path-shape only: it does not parse `item.md`
/// content. The caller does that after `LayoutState::Flat` /
/// `LayoutState::Legacy`.
pub fn resolve_layout(repo_root: &Path, slug: &str) -> LayoutState {
    let archive_dirs = archive_dirs_for(&repo_root.join(ISSUES_DIR), slug);
    resolve_layout_with_archive(repo_root, slug, &archive_dirs)
}

/// Like [`resolve_layout`] but reuses a prebuilt [`ArchiveIndex`] instead
/// of walking the archive tree. Use this in loops that resolve many
/// slugs — build the index once with [`archive_index`].
pub fn resolve_layout_in(repo_root: &Path, slug: &str, index: &ArchiveIndex) -> LayoutState {
    let empty: Vec<PathBuf> = Vec::new();
    let archive_dirs = index.get(slug).unwrap_or(&empty);
    resolve_layout_with_archive(repo_root, slug, archive_dirs)
}

fn resolve_layout_with_archive(
    repo_root: &Path,
    slug: &str,
    archive_dirs: &[PathBuf],
) -> LayoutState {
    let issues_root = repo_root.join(ISSUES_DIR);
    let canon_root = match std::fs::canonicalize(&issues_root) {
        Ok(p) => Some(p),
        Err(_) => None,
    };

    // Archived issues resolve as `Flat` at their cold-storage path: the
    // on-disk depth differs but they behave like any other flat issue
    // (status-derived folder, in-place edits). Treating them as a
    // `legacy: None` candidate means a slug present both active and
    // archived correctly surfaces as `Ambiguous`.
    let mut candidates: Vec<(PathBuf, Option<&'static str>)> = vec![
        (issues_root.join(slug), None),
        (issues_root.join("open").join(slug), Some("open")),
        (issues_root.join("closed").join(slug), Some("closed")),
    ];
    for dir in archive_dirs {
        candidates.push((dir.clone(), None));
    }

    let mut hits: Vec<(PathBuf, Option<&'static str>, ItemCheck)> = Vec::new();
    for (dir, legacy) in candidates {
        match check_dir(&dir, canon_root.as_deref()) {
            Some(check) => hits.push((dir, legacy, check)),
            None => continue,
        }
    }

    match hits.len() {
        0 => LayoutState::Absent,
        1 => {
            let (dir, legacy, check) = hits.into_iter().next().unwrap();
            match check {
                ItemCheck::Ok(item) => match legacy {
                    None => LayoutState::Flat { item_path: item },
                    Some(folder) => LayoutState::Legacy {
                        folder,
                        item_path: item,
                    },
                },
                ItemCheck::Invalid(reason) => LayoutState::Invalid {
                    item_path: Some(dir.join("item.md")),
                    reason,
                },
            }
        }
        _ => LayoutState::Ambiguous {
            paths: hits.into_iter().map(|(d, _, _)| d).collect(),
        },
    }
}

enum ItemCheck {
    Ok(PathBuf),
    Invalid(String),
}

/// Inspect a candidate issue directory. Returns `None` if the directory
/// doesn't exist (so `resolve_layout` can count present-paths cleanly);
/// returns `Some(Invalid)` for symlinked or escaping dirs / bad item.md.
fn check_dir(dir: &Path, canon_root: Option<&Path>) -> Option<ItemCheck> {
    let meta = std::fs::symlink_metadata(dir).ok()?;
    if meta.file_type().is_symlink() {
        return Some(ItemCheck::Invalid(format!(
            "issue directory is a symlink: {}",
            dir.display()
        )));
    }
    if !meta.is_dir() {
        // A regular file at this path is treated as "not the issue"
        // for resolver counting; callers like migrate may still surface
        // it as a conflict via their own check.
        return None;
    }
    if let Some(root) = canon_root {
        match std::fs::canonicalize(dir) {
            Ok(c) if !c.starts_with(root) => {
                return Some(ItemCheck::Invalid(format!(
                    "issue directory escapes repository: {} → {}",
                    dir.display(),
                    c.display()
                )));
            }
            _ => {}
        }
    }
    let item = dir.join("item.md");
    let item_meta = match std::fs::symlink_metadata(&item) {
        Ok(m) => m,
        Err(_) => {
            return Some(ItemCheck::Invalid(format!("missing {}", item.display())));
        }
    };
    if item_meta.file_type().is_symlink() || !item_meta.is_file() {
        return Some(ItemCheck::Invalid(format!(
            "item.md is symlinked or not a regular file: {}",
            item.display()
        )));
    }
    Some(ItemCheck::Ok(item))
}

/// Load all issues from the flat layout (and legacy compat paths).
pub fn load_issues(repo_root: &Path) -> Vec<Issue> {
    load_issues_with_config(repo_root, &crate::repo_config::UncachedConfig)
}

/// Same as `load_issues` but with explicit config source. Server paths
/// pass their `Arc<RepoConfigCache>`; the CLI passes `&UncachedConfig`.
pub fn load_issues_with_config(
    repo_root: &Path,
    config: &dyn crate::repo_config::ConfigSource,
) -> Vec<Issue> {
    let mut result = Vec::new();
    let schema = load_schema_or_warn(repo_root, None, config);
    let archive = archive_index(repo_root);
    for slug in discover_slugs(repo_root) {
        match resolve_layout_in(repo_root, &slug, &archive) {
            LayoutState::Flat { item_path } => {
                let mut issue = parser::parse_item_md(&item_path, &slug, "open");
                issue.folder = folder_for_status(&schema, &issue.status).to_string();
                result.push(issue);
            }
            LayoutState::Legacy { item_path, folder } => {
                eprintln!(
                    "Warning: {slug} found at legacy path issues/{folder}/{slug}/ — run `issuectl doctor --fix`"
                );
                let mut issue = parser::parse_item_md(&item_path, &slug, "open");
                issue.folder = folder_for_status(&schema, &issue.status).to_string();
                result.push(issue);
            }
            LayoutState::Ambiguous { paths } => {
                eprintln!(
                    "Warning: {slug} present at multiple paths ({:?}) — resolve manually",
                    paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                );
            }
            LayoutState::Invalid { reason, .. } => {
                eprintln!("Warning: {slug}: {reason}");
            }
            LayoutState::Absent => {}
        }
    }
    result.sort_by(|a, b| a.slug.cmp(&b.slug));
    result
}

/// Load all issues plus per-file parse warnings. Re-parses schema each
/// call (CLI default). Server callers should use
/// [`load_issues_with_warnings_via`] and pass their `RepoConfigCache`
/// to get cross-request reuse.
pub fn load_issues_with_warnings(repo_root: &Path) -> (Vec<Issue>, Vec<LoadWarning>) {
    load_issues_with_warnings_via(repo_root, &crate::repo_config::UncachedConfig)
}

/// Same as `load_issues_with_warnings` but uses the supplied
/// `ConfigSource` for the schema parse. Server endpoints route their
/// `Arc<RepoConfigCache>` through this so the per-request schema reuse
/// the cache enforces.
pub fn load_issues_with_warnings_via(
    repo_root: &Path,
    config: &dyn crate::repo_config::ConfigSource,
) -> (Vec<Issue>, Vec<LoadWarning>) {
    let mut issues = Vec::new();
    let mut warnings = Vec::new();
    let schema = load_schema_or_warn(repo_root, Some(&mut warnings), config);
    let archive = archive_index(repo_root);

    for slug in discover_slugs(repo_root) {
        match resolve_layout_in(repo_root, &slug, &archive) {
            LayoutState::Flat { item_path } => {
                push_issue_with_parse(
                    &schema,
                    &slug,
                    &item_path,
                    false,
                    None,
                    &mut issues,
                    &mut warnings,
                );
            }
            LayoutState::Legacy { folder, item_path } => {
                push_issue_with_parse(
                    &schema,
                    &slug,
                    &item_path,
                    true,
                    Some(folder),
                    &mut issues,
                    &mut warnings,
                );
            }
            LayoutState::Ambiguous { paths } => {
                warnings.push(LoadWarning {
                    slug: slug.clone(),
                    folder: "ambiguous".to_string(),
                    message: format!(
                        "ambiguous slug — present at: {}",
                        paths
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    code: Some(LoadWarningCode::AmbiguousSlug),
                });
            }
            LayoutState::Invalid { reason, .. } => {
                warnings.push(LoadWarning {
                    slug: slug.clone(),
                    folder: "open".to_string(),
                    message: reason,
                    code: Some(LoadWarningCode::MissingItem),
                });
            }
            LayoutState::Absent => {}
        }
    }

    issues.sort_by(|a, b| a.slug.cmp(&b.slug));
    (issues, warnings)
}

fn push_issue_with_parse(
    schema: &crate::schema::Schema,
    slug: &str,
    item_path: &Path,
    legacy: bool,
    legacy_folder: Option<&'static str>,
    issues: &mut Vec<Issue>,
    warnings: &mut Vec<LoadWarning>,
) {
    let parsed = parser::parse_item_md_with_warnings(item_path, slug, "open");
    let derived_folder = folder_for_status(schema, &parsed.issue.status);
    for w in parsed.warnings {
        warnings.push(LoadWarning {
            slug: slug.to_string(),
            folder: derived_folder.to_string(),
            message: w,
            code: Some(LoadWarningCode::ParseWarning),
        });
    }
    if legacy {
        let folder = legacy_folder.unwrap_or("open");
        warnings.push(LoadWarning {
            slug: slug.to_string(),
            folder: derived_folder.to_string(),
            message: format!(
                "found at legacy path issues/{folder}/{slug}/ — run `issuectl doctor --fix`"
            ),
            code: Some(LoadWarningCode::LegacyLayout),
        });
    }
    let mut issue = parsed.issue;
    issue.folder = derived_folder.to_string();
    issues.push(issue);
}

/// Body-free projection used by `GET /api/issues` when the query
/// has no `text:` term — saves a per-issue body read+allocate.
pub fn load_issue_summaries(repo_root: &Path) -> (Vec<IssueSummary>, Vec<LoadWarning>) {
    let (issues, warnings) = load_issues_with_warnings(repo_root);
    (
        issues.into_iter().map(IssueSummary::from).collect(),
        warnings,
    )
}

/// Result of `locate_issue`: where the slug currently lives on disk and
/// (for legacy paths) which compat folder it was found under.
#[derive(Debug, Clone)]
pub struct Located {
    pub item_path: PathBuf,
    pub legacy_folder: Option<&'static str>,
}

/// Locate a single issue's `item.md` by slug. Delegates classification to
/// `resolve_layout`. Returns `Err` for `Absent`, `Ambiguous`, and
/// `Invalid` — callers that need to disambiguate use `resolve_layout`
/// directly.
pub fn locate_issue_full(repo_root: &Path, slug: &str) -> Result<Located> {
    if !crate::slug::is_valid(slug) {
        bail!("invalid slug shape: {slug:?}");
    }
    match resolve_layout(repo_root, slug) {
        LayoutState::Flat { item_path } => Ok(Located {
            item_path,
            legacy_folder: None,
        }),
        LayoutState::Legacy { folder, item_path } => Ok(Located {
            item_path,
            legacy_folder: Some(folder),
        }),
        LayoutState::Absent => bail!(
            "issue {slug} not found under {}",
            repo_root.join(ISSUES_DIR).display()
        ),
        LayoutState::Ambiguous { paths } => bail!(
            "issue {slug} is ambiguous (present at: {})",
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        LayoutState::Invalid { reason, .. } => bail!("{reason}"),
    }
}

/// Backwards-compatible shim returning `(folder, item_path)` where
/// `folder` is the kanban bucket derived from frontmatter status — never
/// the legacy on-disk folder name. Kept so existing CLI code paths keep
/// compiling.
pub fn locate_issue(repo_root: &Path, slug: &str) -> Result<(String, PathBuf)> {
    let located = locate_issue_full(repo_root, slug)?;
    // Always derive folder from content — never return the legacy
    // on-disk folder name. A `status: fixed` issue at `issues/open/foo/`
    // surfaces as folder = "closed".
    let issue = parser::parse_item_md(&located.item_path, slug, "open");
    let schema = load_schema_or_warn(repo_root, None, &crate::repo_config::UncachedConfig);
    let folder = folder_for_status(&schema, &issue.status).to_string();
    Ok((folder, located.item_path))
}

/// Load a single issue by slug. O(1) instead of O(N).
pub fn load_issue(repo_root: &Path, slug: &str) -> Result<Issue> {
    let located = locate_issue_full(repo_root, slug)?;
    let mut issue = parser::parse_item_md(&located.item_path, slug, "open");
    let schema = load_schema_or_warn(repo_root, None, &crate::repo_config::UncachedConfig);
    issue.folder = folder_for_status(&schema, &issue.status).to_string();
    Ok(issue)
}

/// Returns `(flat_dir, legacy_open_dir, legacy_closed_dir)` for a slug —
/// regardless of whether they exist. Callers use this to detect ambiguous
/// states (e.g. flat AND legacy both present).
pub fn paths_for(repo_root: &Path, slug: &str) -> (PathBuf, PathBuf, PathBuf) {
    let issues = repo_root.join(ISSUES_DIR);
    (
        issues.join(slug),
        issues.join("open").join(slug),
        issues.join("closed").join(slug),
    )
}

/// Migrate a single legacy `issues/{open,closed}/<slug>/` directory to
/// the canonical flat `issues/<slug>/`. Caller MUST hold the repo
/// `flock`. Returns the new flat path.
///
/// Defensive: rejects ambiguous state (flat+legacy or both-legacy) even
/// though `mutate::locate_and_migrate` pre-checks. The function is `pub`
/// so tests / future callers may bypass the pre-check; the helper must
/// enforce its own invariants.
pub fn migrate_to_flat_inplace(repo_root: &Path, slug: &str) -> Result<PathBuf> {
    match resolve_layout(repo_root, slug) {
        LayoutState::Flat { item_path } => {
            // Already flat — return parent dir.
            Ok(item_path.parent().unwrap_or(repo_root).to_path_buf())
        }
        LayoutState::Absent => Ok(repo_root.join(ISSUES_DIR).join(slug)),
        LayoutState::Legacy { item_path, .. } => {
            let src = item_path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("legacy item.md has no parent"))?
                .to_path_buf();
            let flat = repo_root.join(ISSUES_DIR).join(slug);
            if let Some(parent) = flat.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("cannot create {}", parent.display()))?;
            }
            std::fs::rename(&src, &flat)
                .with_context(|| format!("cannot rename {} → {}", src.display(), flat.display()))?;
            Ok(flat)
        }
        LayoutState::Ambiguous { paths } => bail!(
            "ambiguous slug {slug}: cannot migrate, present at {}",
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        LayoutState::Invalid { reason, .. } => bail!("cannot migrate {slug}: {reason}"),
    }
}

/// One issue file whose references were (or would be) rewritten by a
/// rename. `field` is `epic` / `related` / `blocked_by` / `body`;
/// `occurrences` counts how many refs in that field were retargeted.
#[derive(Debug, Clone, Serialize)]
pub struct RefChange {
    pub slug: String,
    pub field: String,
    pub occurrences: usize,
}

/// An issue dir that the rename scan could not read (parse error,
/// ambiguous, or invalid layout). Its references — if any — were left
/// untouched, so the user must inspect it manually; `doctor` will flag
/// any dangling refs it still carries.
#[derive(Debug, Clone, Serialize)]
pub struct SkippedFile {
    pub slug: String,
    pub reason: String,
}

/// Outcome of `rename_issue`. In `dry_run` no files are touched and
/// `changes` reports what *would* be rewritten.
#[derive(Debug, Clone, Serialize)]
pub struct RenameOutcome {
    pub old_slug: String,
    pub new_slug: String,
    pub new_dir: PathBuf,
    pub dry_run: bool,
    pub changes: Vec<RefChange>,
    /// Issue dirs skipped because they couldn't be read; their refs were
    /// not rewritten. Empty on a clean repo.
    pub skipped: Vec<SkippedFile>,
}

/// Rename an issue's slug, rewriting every reference across the store:
/// the on-disk directory, plus `epic:` / `related:` / `blocked_by:`
/// frontmatter refs and `@slug` body mentions in all other issues
/// (including the renamed issue itself, in case it self-referenced).
///
/// Only those three frontmatter fields are retargeted — the same set
/// `doctor` validates. A project that stores slug refs in a custom field
/// must fix those by hand; `doctor` will surface them as dangling.
///
/// Holds the repo-wide `flock` for the whole operation so a concurrent
/// writer can't observe the half-renamed state. References are rewritten
/// *before* the directory is moved, so a failure mid-rewrite leaves the
/// source in place and the command is safe to re-run (already-rewritten
/// files simply no longer match `old`). A legacy-layout source is
/// migrated to flat at the new path as a side effect of the move.
/// `dry_run` performs no disk writes and only reports the would-be
/// changes. Note: each touched file is re-serialized through the normal
/// write path, so unrelated frontmatter is reformatted (comments/key
/// order normalized) exactly as any other mutation would do.
pub fn rename_issue(
    repo_root: &Path,
    old: &str,
    new: &str,
    dry_run: bool,
) -> Result<RenameOutcome> {
    if !crate::slug::is_valid(old) {
        bail!("invalid slug shape: {old:?}");
    }
    if !crate::slug::is_valid(new) {
        bail!("invalid slug shape: {new:?}");
    }
    if old == new {
        bail!("old and new slug are identical: {old:?}");
    }

    let _lock = crate::mutate::WriteLock::acquire(repo_root)?;
    let archive = archive_index(repo_root);

    // Source must exist and be loadable (not ambiguous / invalid). Capture
    // its on-disk dir now so we can move it last. `src_is_legacy` selects
    // the destination: a legacy source migrates to the active flat root,
    // while a flat source (active OR archived) keeps its current parent —
    // so renaming an archived issue stays inside its archive bucket
    // rather than silently un-archiving it.
    let (src_dir, src_is_legacy) = match resolve_layout_in(repo_root, old, &archive) {
        LayoutState::Flat { item_path } => (parent_dir(&item_path)?, false),
        LayoutState::Legacy { item_path, .. } => (parent_dir(&item_path)?, true),
        LayoutState::Absent => bail!(
            "issue {old} not found under {}",
            repo_root.join(ISSUES_DIR).display()
        ),
        LayoutState::Ambiguous { paths } => bail!(
            "issue {old} is ambiguous (present at: {})",
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        LayoutState::Invalid { reason, .. } => bail!("cannot rename {old}: {reason}"),
    };
    // Target slug must be free at every candidate path. `Absent` already
    // implies no `issues/<new>/` dir exists, so no separate dir check.
    if !matches!(resolve_layout_in(repo_root, new, &archive), LayoutState::Absent) {
        bail!("target slug {new} already exists");
    }
    let new_dir = if src_is_legacy {
        repo_root.join(ISSUES_DIR).join(new)
    } else {
        src_dir
            .parent()
            .ok_or_else(|| anyhow::anyhow!("source dir has no parent"))?
            .join(new)
    };

    // Rewrite references in every issue file (including the renamed issue
    // itself, still at its old path). The directory move happens last.
    let mut changes = Vec::new();
    let mut skipped = Vec::new();
    for slug in discover_slugs(repo_root) {
        let item_path = match resolve_layout_in(repo_root, &slug, &archive) {
            LayoutState::Flat { item_path } | LayoutState::Legacy { item_path, .. } => item_path,
            LayoutState::Absent => continue,
            LayoutState::Ambiguous { .. } => {
                skipped.push(SkippedFile {
                    slug: slug.clone(),
                    reason: "ambiguous layout".to_string(),
                });
                continue;
            }
            LayoutState::Invalid { reason, .. } => {
                skipped.push(SkippedFile {
                    slug: slug.clone(),
                    reason,
                });
                continue;
            }
        };
        let mut item = match crate::write::read_item(&item_path) {
            Ok(item) => item,
            Err(e) => {
                skipped.push(SkippedFile {
                    slug: slug.clone(),
                    reason: format!("unreadable: {e:#}"),
                });
                continue;
            }
        };
        let fm_changes = rewrite_frontmatter_refs(&mut item.frontmatter, old, new);
        let (new_body, body_n) = crate::refs::rewrite_body_refs(&item.body, old, new);
        let touched = !fm_changes.is_empty() || body_n > 0;
        if !touched {
            continue;
        }
        item.body = new_body;
        // The renamed issue's own file is reported under its post-rename
        // slug so dry-run and real-run output agree (both scan `old`).
        let report_slug = if slug == old { new } else { slug.as_str() };
        for (field, occurrences) in fm_changes {
            changes.push(RefChange {
                slug: report_slug.to_string(),
                field,
                occurrences,
            });
        }
        if body_n > 0 {
            changes.push(RefChange {
                slug: report_slug.to_string(),
                field: "body".to_string(),
                occurrences: body_n,
            });
        }
        if !dry_run {
            crate::mutate::write_item_atomic(&item_path, &item)?;
        }
    }

    // Move the directory last: now that every reference is rewritten, the
    // rename is the final, single atomic step.
    if !dry_run {
        std::fs::rename(&src_dir, &new_dir).with_context(|| {
            format!(
                "cannot rename {} → {}",
                src_dir.display(),
                new_dir.display()
            )
        })?;
    }

    Ok(RenameOutcome {
        old_slug: old.to_string(),
        new_slug: new.to_string(),
        new_dir,
        dry_run,
        changes,
        skipped,
    })
}

/// Rewrite `epic` / `related` / `blocked_by` slug refs in one frontmatter
/// mapping. Returns `(field, occurrences)` for each field that changed.
/// `related` / `blocked_by` are handled whether written as a YAML
/// sequence or as a bare scalar (`related: @old`).
/// The directory containing an `item.md`, or an error if it has no
/// parent (should never happen for a resolved layout path).
fn parent_dir(item_path: &Path) -> Result<PathBuf> {
    item_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("item.md has no parent: {}", item_path.display()))
        .map(Path::to_path_buf)
}

fn rewrite_frontmatter_refs(
    fm: &mut serde_yaml::Mapping,
    old: &str,
    new: &str,
) -> Vec<(String, usize)> {
    use serde_yaml::Value;
    let mut changes = Vec::new();
    if let Some(Value::String(s)) = fm.get_mut(Value::String("epic".into())) {
        if let Some(nv) = crate::refs::rewrite_slug_ref(s, old, new) {
            *s = nv;
            changes.push(("epic".to_string(), 1));
        }
    }
    for key in ["related", "blocked_by"] {
        match fm.get_mut(Value::String(key.into())) {
            Some(Value::Sequence(seq)) => {
                let mut n = 0;
                for item in seq.iter_mut() {
                    if let Value::String(s) = item {
                        if let Some(nv) = crate::refs::rewrite_slug_ref(s, old, new) {
                            *s = nv;
                            n += 1;
                        }
                    }
                }
                if n > 0 {
                    changes.push((key.to_string(), n));
                }
            }
            Some(Value::String(s)) => {
                if let Some(nv) = crate::refs::rewrite_slug_ref(s, old, new) {
                    *s = nv;
                    changes.push((key.to_string(), 1));
                }
            }
            _ => {}
        }
    }
    changes
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

    fn seed_flat(tmp: &TempDir, slug: &str, status: &str) {
        let dir = tmp.path().join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            format!("---\nstatus: {status}\n---\n\n# {slug}\n"),
        )
        .unwrap();
    }

    fn seed_legacy(tmp: &TempDir, folder: &str, slug: &str, status: &str) {
        let dir = tmp.path().join("issues").join(folder).join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            format!("---\nstatus: {status}\n---\n\n# {slug}\n"),
        )
        .unwrap();
    }

    fn seed_with(tmp: &TempDir, slug: &str, frontmatter: &str, body: &str) {
        let dir = tmp.path().join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            format!("---\nstatus: open\n{frontmatter}---\n\n{body}"),
        )
        .unwrap();
    }

    #[test]
    fn rename_moves_dir_and_rewrites_all_refs() {
        let tmp = fresh_repo();
        seed_flat(&tmp, "old-tame-fox", "open");
        seed_with(
            &tmp,
            "child-calm-owl",
            "epic: \"@old-tame-fox\"\n",
            "# child\n\nblocks @old-tame-fox here\n",
        );
        seed_with(
            &tmp,
            "peer-bright-elk",
            "related: [\"@old-tame-fox\", \"@other-quiet-newt\"]\nblocked_by: [\"old-tame-fox\"]\n",
            "# peer\n",
        );

        let out = rename_issue(tmp.path(), "old-tame-fox", "new-wild-stag", false).unwrap();
        assert!(out.new_dir.join("item.md").is_file());
        assert!(!tmp.path().join("issues/old-tame-fox").exists());

        let child = fs::read_to_string(tmp.path().join("issues/child-calm-owl/item.md")).unwrap();
        assert!(child.contains("@new-wild-stag"));
        assert!(!child.contains("old-tame-fox"));

        let peer = fs::read_to_string(tmp.path().join("issues/peer-bright-elk/item.md")).unwrap();
        assert!(peer.contains("@new-wild-stag"));
        assert!(peer.contains("@other-quiet-newt"));
        // bare blocked_by ref retargeted without gaining an @ prefix
        assert!(peer.contains("new-wild-stag"));
        assert!(!peer.contains("old-tame-fox"));

        // doctor sees no dangling refs after a tool-driven rename
        let fields: Vec<&str> = out.changes.iter().map(|c| c.field.as_str()).collect();
        assert!(fields.contains(&"epic"));
        assert!(fields.contains(&"related"));
        assert!(fields.contains(&"blocked_by"));
        assert!(fields.contains(&"body"));
    }

    #[test]
    fn rename_dry_run_reports_without_writing() {
        let tmp = fresh_repo();
        seed_flat(&tmp, "old-tame-fox", "open");
        seed_with(
            &tmp,
            "child-calm-owl",
            "epic: \"@old-tame-fox\"\n",
            "# child\n",
        );
        let out = rename_issue(tmp.path(), "old-tame-fox", "new-wild-stag", true).unwrap();
        assert!(out.dry_run);
        assert_eq!(out.changes.len(), 1);
        // nothing moved or rewritten
        assert!(tmp.path().join("issues/old-tame-fox").exists());
        assert!(!tmp.path().join("issues/new-wild-stag").exists());
        let child = fs::read_to_string(tmp.path().join("issues/child-calm-owl/item.md")).unwrap();
        assert!(child.contains("@old-tame-fox"));
    }

    #[test]
    fn rename_rejects_existing_target_and_missing_source() {
        let tmp = fresh_repo();
        seed_flat(&tmp, "old-tame-fox", "open");
        seed_flat(&tmp, "taken-wild-stag", "open");
        assert!(rename_issue(tmp.path(), "old-tame-fox", "taken-wild-stag", false).is_err());
        assert!(rename_issue(tmp.path(), "ghost-quiet-newt", "new-wild-stag", false).is_err());
        assert!(rename_issue(tmp.path(), "old-tame-fox", "old-tame-fox", false).is_err());
    }

    #[test]
    fn rename_rewrites_scalar_related_and_blocked_by() {
        let tmp = fresh_repo();
        seed_flat(&tmp, "old-tame-fox", "open");
        seed_with(
            &tmp,
            "peer-bright-elk",
            "related: \"@old-tame-fox\"\nblocked_by: old-tame-fox\n",
            "# peer\n",
        );
        rename_issue(tmp.path(), "old-tame-fox", "new-wild-stag", false).unwrap();
        let peer = fs::read_to_string(tmp.path().join("issues/peer-bright-elk/item.md")).unwrap();
        assert!(peer.contains("new-wild-stag"));
        assert!(!peer.contains("old-tame-fox"));
    }

    #[test]
    fn rename_rewrites_self_reference_and_reports_new_slug() {
        let tmp = fresh_repo();
        seed_with(
            &tmp,
            "old-tame-fox",
            "related: [\"@old-tame-fox\"]\n",
            "# self\n\nsee @old-tame-fox\n",
        );
        let out = rename_issue(tmp.path(), "old-tame-fox", "new-wild-stag", false).unwrap();
        let body = fs::read_to_string(out.new_dir.join("item.md")).unwrap();
        assert!(body.contains("@new-wild-stag"));
        assert!(!body.contains("old-tame-fox"));
        // self-ref changes report under the post-rename slug
        assert!(out.changes.iter().all(|c| c.slug == "new-wild-stag"));
    }

    #[test]
    fn rename_does_not_touch_prefix_overlapping_slug() {
        let tmp = fresh_repo();
        seed_flat(&tmp, "old-tame-fox", "open");
        seed_with(
            &tmp,
            "peer-bright-elk",
            "related: [\"@old-tame-fox-cub\"]\n",
            "# peer\n\nsee @old-tame-fox-cub here\n",
        );
        rename_issue(tmp.path(), "old-tame-fox", "new-wild-stag", false).unwrap();
        let peer = fs::read_to_string(tmp.path().join("issues/peer-bright-elk/item.md")).unwrap();
        // the longer, unrelated slug is untouched
        assert!(peer.contains("@old-tame-fox-cub"));
        assert!(!peer.contains("new-wild-stag"));
    }

    #[test]
    fn rename_surfaces_unreadable_files_as_skipped() {
        let tmp = fresh_repo();
        seed_flat(&tmp, "old-tame-fox", "open");
        // A dir with item.md that fails to parse as frontmatter.
        let bad = tmp.path().join("issues/broken-calm-owl");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("item.md"), "---\n: : not yaml : :\n---\n# bad\n").unwrap();
        let out = rename_issue(tmp.path(), "old-tame-fox", "new-wild-stag", false).unwrap();
        assert!(out.skipped.iter().any(|s| s.slug == "broken-calm-owl"));
        // the rename still completed for the readable issue
        assert!(out.new_dir.join("item.md").is_file());
    }

    #[test]
    fn rename_migrates_legacy_source_to_flat() {
        let tmp = fresh_repo();
        seed_legacy(&tmp, "open", "old-tame-fox", "open");
        let out = rename_issue(tmp.path(), "old-tame-fox", "new-wild-stag", false).unwrap();
        assert!(out.new_dir.join("item.md").is_file());
        assert!(!tmp.path().join("issues/open/old-tame-fox").exists());
    }

    fn seed_archive(tmp: &TempDir, year: &str, month: &str, slug: &str, status: &str) {
        let dir = tmp
            .path()
            .join("issues")
            .join("archive")
            .join(year)
            .join(month)
            .join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            format!("---\nstatus: {status}\n---\n\n# {slug}\n"),
        )
        .unwrap();
    }

    #[test]
    fn archive_relpath_splits_date_into_year_month() {
        assert_eq!(
            archive_relpath("calm-wild-otter", "2026-05-06"),
            Path::new("archive/2026/05/calm-wild-otter")
        );
        // missing/short date falls back to `unknown` buckets, never panics
        assert_eq!(
            archive_relpath("calm-wild-otter", ""),
            Path::new("archive/unknown/unknown/calm-wild-otter")
        );
        assert_eq!(
            archive_relpath("calm-wild-otter", "2026"),
            Path::new("archive/2026/unknown/calm-wild-otter")
        );
    }

    #[test]
    fn load_issues_reads_archive_root() {
        let tmp = fresh_repo();
        seed_flat(&tmp, "active-quiet-otter", "open");
        seed_archive(&tmp, "2026", "05", "old-done-fox", "fixed");
        let issues = load_issues(tmp.path());
        let slugs: Vec<&str> = issues.iter().map(|i| i.slug.as_str()).collect();
        assert!(slugs.contains(&"active-quiet-otter"));
        assert!(slugs.contains(&"old-done-fox"));
        let archived = issues.iter().find(|i| i.slug == "old-done-fox").unwrap();
        // status-derived folder still works for archived issues
        assert_eq!(archived.folder, "closed");
    }

    #[test]
    fn locate_issue_finds_archived_slug() {
        let tmp = fresh_repo();
        seed_archive(&tmp, "2026", "05", "old-done-fox", "fixed");
        let located = locate_issue_full(tmp.path(), "old-done-fox").unwrap();
        assert!(located.item_path.ends_with("archive/2026/05/old-done-fox/item.md"));
        assert!(located.legacy_folder.is_none());
    }

    #[test]
    fn rename_keeps_archived_issue_in_its_bucket() {
        let tmp = fresh_repo();
        seed_archive(&tmp, "2026", "05", "old-done-fox", "fixed");
        let out = rename_issue(tmp.path(), "old-done-fox", "new-done-stag", false).unwrap();
        // stays in the same archive month bucket, not pulled to active root
        assert!(out.new_dir.ends_with("archive/2026/05/new-done-stag"));
        assert!(tmp
            .path()
            .join("issues/archive/2026/05/new-done-stag/item.md")
            .is_file());
        assert!(!tmp.path().join("issues/new-done-stag").exists());
        assert!(!tmp.path().join("issues/archive/2026/05/old-done-fox").exists());
    }

    #[test]
    fn active_plus_archived_slug_is_ambiguous() {
        let tmp = fresh_repo();
        seed_flat(&tmp, "dup-calm-owl", "open");
        seed_archive(&tmp, "2026", "05", "dup-calm-owl", "fixed");
        assert!(matches!(
            resolve_layout(tmp.path(), "dup-calm-owl"),
            LayoutState::Ambiguous { .. }
        ));
    }

    #[test]
    fn load_issues_returns_sorted_by_slug() {
        let tmp = fresh_repo();
        seed_flat(&tmp, "quiet-brave-otter", "open");
        seed_flat(&tmp, "amber-loud-fox", "open");
        seed_flat(&tmp, "tiny-wild-comet", "fixed");
        let issues = load_issues(tmp.path());
        let slugs: Vec<&str> = issues.iter().map(|i| i.slug.as_str()).collect();
        assert_eq!(
            slugs,
            vec!["amber-loud-fox", "quiet-brave-otter", "tiny-wild-comet"]
        );
        // folder derived from status
        let comet = issues.iter().find(|i| i.slug == "tiny-wild-comet").unwrap();
        assert_eq!(comet.folder, "closed");
    }

    #[test]
    fn legacy_layout_emits_warning() {
        let tmp = fresh_repo();
        seed_legacy(&tmp, "open", "legacy-issue-here", "open");
        let (issues, warnings) = load_issues_with_warnings(tmp.path());
        assert_eq!(issues.len(), 1);
        assert!(warnings
            .iter()
            .any(|w| matches!(w.code, Some(LoadWarningCode::LegacyLayout))));
    }

    #[test]
    fn ambiguous_flat_plus_legacy_emits_warning_no_duplicate() {
        let tmp = fresh_repo();
        seed_flat(&tmp, "shared-slug-here", "open");
        seed_legacy(&tmp, "open", "shared-slug-here", "open");
        let (issues, warnings) = load_issues_with_warnings(tmp.path());
        // No issue is loaded — ambiguity is unresolvable without operator action.
        assert!(issues.iter().all(|i| i.slug != "shared-slug-here"));
        assert!(warnings.iter().any(|w| w.slug == "shared-slug-here"
            && matches!(w.code, Some(LoadWarningCode::AmbiguousSlug))));
    }

    #[test]
    fn ambiguous_dual_legacy_emits_warning_no_duplicate() {
        let tmp = fresh_repo();
        seed_legacy(&tmp, "open", "dual-legacy-here", "open");
        seed_legacy(&tmp, "closed", "dual-legacy-here", "fixed");
        let (issues, warnings) = load_issues_with_warnings(tmp.path());
        let count = issues
            .iter()
            .filter(|i| i.slug == "dual-legacy-here")
            .count();
        assert_eq!(
            count, 0,
            "ambiguous dual-legacy must not produce loaded issues"
        );
        assert!(warnings.iter().any(|w| w.slug == "dual-legacy-here"
            && matches!(w.code, Some(LoadWarningCode::AmbiguousSlug))));
    }

    #[test]
    fn load_issues_does_not_duplicate_dual_legacy() {
        let tmp = fresh_repo();
        seed_legacy(&tmp, "open", "dual-legacy-here", "open");
        seed_legacy(&tmp, "closed", "dual-legacy-here", "fixed");
        let issues = load_issues(tmp.path());
        // load_issues drops ambiguous slugs entirely (with an eprintln).
        assert!(issues.iter().all(|i| i.slug != "dual-legacy-here"));
    }

    #[cfg(unix)]
    #[test]
    fn locate_issue_rejects_symlinked_dir() {
        use std::os::unix::fs::symlink;
        let tmp = fresh_repo();
        let outside = tempfile::tempdir().unwrap();
        fs::write(
            outside.path().join("item.md"),
            "---\nstatus: open\n---\n# x\n",
        )
        .unwrap();
        symlink(outside.path(), tmp.path().join("issues/escaped-not-otter")).unwrap();
        let r = locate_issue(tmp.path(), "escaped-not-otter");
        assert!(r.is_err(), "symlinked issue dir must be rejected");
        let msg = r.unwrap_err().to_string();
        assert!(msg.contains("symlink"), "error should explain why: {msg}");
    }

    #[test]
    fn load_issues_skips_dirs_without_item_md() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/quiet-brave-otter")).unwrap();
        seed_flat(&tmp, "amber-loud-fox", "open");
        let issues = load_issues(tmp.path());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].slug, "amber-loud-fox");
    }

    #[test]
    fn migrate_to_flat_inplace_moves_legacy_dir() {
        let tmp = fresh_repo();
        seed_legacy(&tmp, "open", "legacy-fox-here", "open");
        let new_path = migrate_to_flat_inplace(tmp.path(), "legacy-fox-here").unwrap();
        assert!(new_path.is_dir());
        assert!(!tmp.path().join("issues/open/legacy-fox-here").exists());
    }

    #[test]
    fn migrate_to_flat_inplace_errors_on_ambiguous() {
        let tmp = fresh_repo();
        seed_flat(&tmp, "ambig-slug-here", "open");
        seed_legacy(&tmp, "open", "ambig-slug-here", "open");
        let r = migrate_to_flat_inplace(tmp.path(), "ambig-slug-here");
        assert!(r.is_err());
    }

    #[test]
    fn migrate_to_flat_inplace_is_noop_for_flat() {
        // M14: assert idempotence — content + dir identity must be
        // preserved across a no-op migrate call.
        use std::os::unix::fs::MetadataExt;
        let tmp = fresh_repo();
        seed_flat(&tmp, "already-flat-here", "open");
        let dir = tmp.path().join("issues/already-flat-here");
        let before_inode = fs::metadata(&dir).unwrap().ino();
        let before_content = fs::read_to_string(dir.join("item.md")).unwrap();
        let r = migrate_to_flat_inplace(tmp.path(), "already-flat-here").unwrap();
        let after_inode = fs::metadata(&r).unwrap().ino();
        let after_content = fs::read_to_string(r.join("item.md")).unwrap();
        assert_eq!(before_inode, after_inode, "no-op must not move dir");
        assert_eq!(
            before_content, after_content,
            "no-op must not modify item.md"
        );
    }

    #[test]
    fn migrate_to_flat_inplace_rejects_dual_legacy() {
        // M2 defensive: even when locate_and_migrate's pre-check is
        // bypassed, the helper itself refuses to silently pick a side.
        let tmp = fresh_repo();
        seed_legacy(&tmp, "open", "dual-here", "open");
        seed_legacy(&tmp, "closed", "dual-here", "fixed");
        let err = migrate_to_flat_inplace(tmp.path(), "dual-here").unwrap_err();
        assert!(err.to_string().contains("ambiguous"));
        // Both legacy dirs must remain on disk for manual recovery.
        assert!(tmp.path().join("issues/open/dual-here").is_dir());
        assert!(tmp.path().join("issues/closed/dual-here").is_dir());
    }
}
