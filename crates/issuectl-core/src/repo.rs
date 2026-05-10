use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::models::Issue;
use crate::parser;

const ISSUES_DIR: &str = "issues";
/// Legacy status-folder names walked separately as compat-read paths.
/// Cold-storage convention is `.archive/<slug>/` (leading dot keeps it
/// out of `slug::is_valid` shape, so no explicit reservation needed).
const LEGACY_FOLDERS: &[&str] = &["open", "closed"];

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
) -> std::sync::Arc<crate::schema::Schema> {
    match crate::schema::load(repo_root) {
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

/// Discover every slug appearing under `issues/` — flat, legacy-open, or
/// legacy-closed. Returns each slug exactly once; the resolver determines
/// the slug's `LayoutState`.
///
/// Reserved-subdir filter: `open` and `closed` are walked as legacy parents,
/// not as slug directories. Anything else under `issues/` (including
/// `.archive/`) is surfaced as a slug candidate — the resolver will
/// reject leading-dot or otherwise invalid shapes via `slug::is_valid`.
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
    slugs
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
    let issues_root = repo_root.join(ISSUES_DIR);
    let canon_root = match std::fs::canonicalize(&issues_root) {
        Ok(p) => Some(p),
        Err(_) => None,
    };

    let candidates: [(PathBuf, Option<&'static str>); 3] = [
        (issues_root.join(slug), None),
        (issues_root.join("open").join(slug), Some("open")),
        (issues_root.join("closed").join(slug), Some("closed")),
    ];

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
    let mut result = Vec::new();
    let schema = load_schema_or_warn(repo_root, None);
    for slug in discover_slugs(repo_root) {
        match resolve_layout(repo_root, &slug) {
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

/// Load all issues plus per-file parse warnings.
pub fn load_issues_with_warnings(repo_root: &Path) -> (Vec<Issue>, Vec<LoadWarning>) {
    let mut issues = Vec::new();
    let mut warnings = Vec::new();
    let schema = load_schema_or_warn(repo_root, Some(&mut warnings));

    for slug in discover_slugs(repo_root) {
        match resolve_layout(repo_root, &slug) {
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
    let schema = load_schema_or_warn(repo_root, None);
    let folder = folder_for_status(&schema, &issue.status).to_string();
    Ok((folder, located.item_path))
}

/// Load a single issue by slug. O(1) instead of O(N).
pub fn load_issue(repo_root: &Path, slug: &str) -> Result<Issue> {
    let located = locate_issue_full(repo_root, slug)?;
    let mut issue = parser::parse_item_md(&located.item_path, slug, "open");
    let schema = load_schema_or_warn(repo_root, None);
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
