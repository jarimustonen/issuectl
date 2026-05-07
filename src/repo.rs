use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::models::Issue;
use crate::parser;

const ISSUES_DIR: &str = "issues";
/// Names that look like slugs but are actually legacy status folders or
/// other reserved subdirectories under `issues/`. Walked separately by
/// the loader; never treated as slug-named issue directories.
const RESERVED_SUBDIRS: &[&str] = &["open", "closed", "archive"];

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
}

impl From<Issue> for IssueSummary {
    fn from(i: Issue) -> Self {
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
    /// Stable machine-readable warning code. `None` for ad-hoc parse
    /// messages; populated for warnings the UI may want to special-case
    /// (currently only `legacy_layout`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
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
pub fn folder_for_status(status: &str) -> &'static str {
    if crate::is_closing_status(status) {
        "closed"
    } else {
        "open"
    }
}

/// Iterate over every slug-named issue directory in the repo, including
/// legacy `issues/open/<slug>` and `issues/closed/<slug>` paths. Yields
/// `(slug, item_path, legacy_folder)` where `legacy_folder` is `Some("open")`
/// or `Some("closed")` for legacy reads and `None` for flat-layout reads.
fn walk_issue_dirs(
    repo_root: &Path,
) -> Vec<(String, PathBuf, Option<&'static str>)> {
    let mut out: Vec<(String, PathBuf, Option<&'static str>)> = Vec::new();
    let issues_dir = repo_root.join(ISSUES_DIR);

    // Flat: issues/<slug>/
    if let Ok(rd) = std::fs::read_dir(&issues_dir) {
        let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if RESERVED_SUBDIRS.contains(&name.as_str()) {
                continue;
            }
            out.push((name, entry.path(), None));
        }
    }

    // Legacy compat: issues/open/<slug>, issues/closed/<slug>
    for legacy in ["open", "closed"] {
        let legacy_dir = issues_dir.join(legacy);
        let Ok(rd) = std::fs::read_dir(&legacy_dir) else {
            continue;
        };
        let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // If a flat copy already exists, skip the legacy one — the
            // caller surfaces the conflict via the dedicated ambiguous
            // path check (`legacy_paths_for`).
            if issues_dir.join(&name).is_dir() {
                continue;
            }
            out.push((name, entry.path(), Some(if legacy == "open" { "open" } else { "closed" })));
        }
    }
    out
}

/// Load all issues from the flat layout (and legacy compat paths).
pub fn load_issues(repo_root: &Path) -> Vec<Issue> {
    let mut result = Vec::new();
    for (slug, dir, legacy) in walk_issue_dirs(repo_root) {
        let item_path = dir.join("item.md");
        if !item_path.is_file() {
            continue;
        }
        let mut issue = parser::parse_item_md(&item_path, &slug, "open");
        // Folder is derived from frontmatter status post-flat-layout.
        issue.folder = folder_for_status(&issue.status).to_string();
        if legacy.is_some() {
            eprintln!(
                "Warning: {slug} found at legacy path {} — run `issuectl migrate layout`",
                dir.display()
            );
        }
        result.push(issue);
    }
    result.sort_by(|a, b| a.slug.cmp(&b.slug));
    result
}

/// Load all issues plus per-file parse warnings.
pub fn load_issues_with_warnings(repo_root: &Path) -> (Vec<Issue>, Vec<LoadWarning>) {
    let mut issues = Vec::new();
    let mut warnings = Vec::new();

    for (slug, dir, legacy) in walk_issue_dirs(repo_root) {
        let item_path = dir.join("item.md");
        let folder_label = legacy.unwrap_or("open"); // placeholder, overwritten below
        if !item_path.is_file() {
            warnings.push(LoadWarning {
                slug: slug.clone(),
                folder: folder_label.to_string(),
                message: format!("missing {}", item_path.display()),
                code: None,
            });
            continue;
        }
        let parsed = parser::parse_item_md_with_warnings(&item_path, &slug, folder_label);
        let derived_folder = folder_for_status(&parsed.issue.status);
        for w in parsed.warnings {
            warnings.push(LoadWarning {
                slug: slug.clone(),
                folder: derived_folder.to_string(),
                message: w,
                code: None,
            });
        }
        if let Some(legacy_kind) = legacy {
            warnings.push(LoadWarning {
                slug: slug.clone(),
                folder: derived_folder.to_string(),
                message: format!(
                    "found at legacy path issues/{legacy_kind}/{slug}/ — run `issuectl migrate layout`"
                ),
                code: Some("legacy_layout".to_string()),
            });
        }
        let mut issue = parsed.issue;
        issue.folder = derived_folder.to_string();
        issues.push(issue);
    }

    issues.sort_by(|a, b| a.slug.cmp(&b.slug));
    (issues, warnings)
}

/// Load only frontmatter + title summaries for every issue.
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

/// Locate a single issue's `item.md` by slug.
///
/// Search order:
///   1. `issues/<slug>/item.md`            (flat — canonical)
///   2. `issues/open/<slug>/item.md`       (legacy compat read)
///   3. `issues/closed/<slug>/item.md`     (legacy compat read)
///
/// Refuses symlinked issue directories outright and verifies (via canonical
/// path comparison) that the resolved directory stays under
/// `<repo_root>/issues/`. Without this, a symlinked entry could escape
/// containment when the slug is reached via direct `/api/issues/<slug>`
/// lookup.
///
/// Returns the canonical legacy folder hint for the compat-read warning;
/// callers writing under a flock should call `migrate_to_flat_inplace`
/// before mutating, so the compat path never gets a write.
pub fn locate_issue_full(repo_root: &Path, slug: &str) -> Result<Located> {
    let issues_root = repo_root.join(ISSUES_DIR);
    let issues_root_canon = std::fs::canonicalize(&issues_root)
        .with_context(|| format!("cannot canonicalize {}", issues_root.display()))?;

    let candidates: [(PathBuf, Option<&'static str>); 3] = [
        (issues_root.join(slug), None),
        (issues_root.join("open").join(slug), Some("open")),
        (issues_root.join("closed").join(slug), Some("closed")),
    ];

    for (dir, legacy) in candidates {
        let meta = match std::fs::symlink_metadata(&dir) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            bail!(
                "issue directory is a symlink (refusing to follow): {}",
                dir.display()
            );
        }
        if !meta.is_dir() {
            continue;
        }
        let dir_canon = std::fs::canonicalize(&dir)
            .with_context(|| format!("cannot canonicalize {}", dir.display()))?;
        if !dir_canon.starts_with(&issues_root_canon) {
            bail!(
                "issue directory escapes repository: {} → {}",
                dir.display(),
                dir_canon.display()
            );
        }
        let item = dir_canon.join("item.md");
        let item_meta = std::fs::symlink_metadata(&item)
            .with_context(|| format!("{slug} directory has no item.md: {}", item.display()))?;
        if item_meta.file_type().is_symlink() || !item_meta.is_file() {
            bail!("{slug} item.md is missing or symlinked: {}", item.display());
        }
        return Ok(Located {
            item_path: item,
            legacy_folder: legacy,
        });
    }
    bail!("issue {slug} not found under {}", issues_root.display())
}

/// Backwards-compatible shim that returns a `(folder, item_path)` tuple
/// where `folder` is the kanban bucket derived from on-disk content (the
/// caller may overwrite via `parser::parse_item_md` for status-aware
/// folder labels). Kept so existing CLI code paths keep compiling.
pub fn locate_issue(repo_root: &Path, slug: &str) -> Result<(String, PathBuf)> {
    let located = locate_issue_full(repo_root, slug)?;
    let folder = match located.legacy_folder {
        Some(f) => f.to_string(),
        None => {
            // Flat-layout read: peek at frontmatter status to derive the
            // kanban folder. Cheap because parse_item_md only re-reads
            // the file once.
            let issue = parser::parse_item_md(&located.item_path, slug, "open");
            folder_for_status(&issue.status).to_string()
        }
    };
    Ok((folder, located.item_path))
}

/// Load a single issue by slug. O(1) instead of O(N).
pub fn load_issue(repo_root: &Path, slug: &str) -> Result<Issue> {
    let located = locate_issue_full(repo_root, slug)?;
    let mut issue = parser::parse_item_md(&located.item_path, slug, "open");
    issue.folder = folder_for_status(&issue.status).to_string();
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
/// Refuses to overwrite an existing flat path — the caller is expected
/// to surface that as an ambiguous-slug error rather than silently
/// merging directories.
pub fn migrate_to_flat_inplace(repo_root: &Path, slug: &str) -> Result<PathBuf> {
    let (flat, legacy_open, legacy_closed) = paths_for(repo_root, slug);
    let legacy_present = if real_dir(&legacy_open) {
        Some(legacy_open)
    } else if real_dir(&legacy_closed) {
        Some(legacy_closed)
    } else {
        None
    };
    let Some(src) = legacy_present else {
        // Already flat (or absent); nothing to do.
        return Ok(flat);
    };
    if real_dir(&flat) {
        bail!(
            "ambiguous slug {slug}: both flat ({}) and legacy ({}) layouts exist",
            flat.display(),
            src.display()
        );
    }
    if let Some(parent) = flat.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::rename(&src, &flat)
        .with_context(|| format!("cannot rename {} → {}", src.display(), flat.display()))?;
    Ok(flat)
}

fn real_dir(p: &Path) -> bool {
    match std::fs::symlink_metadata(p) {
        Ok(m) => m.is_dir() && !m.file_type().is_symlink(),
        Err(_) => false,
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
        assert!(warnings.iter().any(|w| w.code.as_deref() == Some("legacy_layout")));
    }

    #[test]
    fn flat_takes_precedence_over_legacy_walk() {
        let tmp = fresh_repo();
        seed_flat(&tmp, "shared-slug-here", "open");
        seed_legacy(&tmp, "open", "shared-slug-here", "open");
        // walk_issue_dirs must skip the legacy copy when a flat copy exists
        // so load_issues doesn't return a duplicate.
        let issues = load_issues(tmp.path());
        assert_eq!(issues.len(), 1);
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
        let tmp = fresh_repo();
        seed_flat(&tmp, "already-flat-here", "open");
        let r = migrate_to_flat_inplace(tmp.path(), "already-flat-here");
        assert!(r.is_ok());
    }
}
