use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::models::Issue;
use crate::parser;

const ISSUES_DIR: &str = "issues";

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

/// Per-file warning collected during a load (e.g., malformed frontmatter).
/// The CLI still emits these to stderr; the web API surfaces them in the
/// listing payload so the UI can flag broken issues rather than silently
/// rendering zombies with default fields.
#[derive(Debug, Clone, Serialize)]
pub struct LoadWarning {
    pub slug: String,
    pub folder: String,
    pub message: String,
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

/// Load all issues from open/ and closed/ directories.
///
/// Each subdirectory is treated as a slug-named issue. Directories whose
/// names don't pass `slug::is_valid` are still loaded (so `issuectl doctor`
/// can flag them) but the web/CLI surfaces only canonical slugs in URLs.
pub fn load_issues(repo_root: &Path) -> Vec<Issue> {
    let issues_dir = repo_root.join(ISSUES_DIR);
    let mut result = Vec::new();

    for folder in &["open", "closed"] {
        let folder_path = issues_dir.join(folder);
        if !folder_path.is_dir() {
            continue;
        }
        let mut entries: Vec<_> = match std::fs::read_dir(&folder_path) {
            Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
            Err(_) => continue,
        };
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let slug = entry.file_name().to_string_lossy().to_string();
            let item_path = entry.path().join("item.md");
            if !item_path.is_file() {
                continue;
            }
            let issue = parser::parse_item_md(&item_path, &slug, folder);
            result.push(issue);
        }
    }

    result.sort_by(|a, b| a.slug.cmp(&b.slug));
    result
}

/// Load all issues plus per-file parse warnings. Mirrors `load_issues` but
/// returns warnings instead of printing to stderr — the web API surfaces
/// these in the response so users see broken issues rather than silently
/// rendering zombies.
pub fn load_issues_with_warnings(repo_root: &Path) -> (Vec<Issue>, Vec<LoadWarning>) {
    let issues_dir = repo_root.join(ISSUES_DIR);
    let mut issues = Vec::new();
    let mut warnings = Vec::new();

    for folder in &["open", "closed"] {
        let folder_path = issues_dir.join(folder);
        if !folder_path.is_dir() {
            continue;
        }
        let mut entries: Vec<_> = match std::fs::read_dir(&folder_path) {
            Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
            Err(e) => {
                warnings.push(LoadWarning {
                    slug: String::new(),
                    folder: folder.to_string(),
                    message: format!("cannot read {}: {}", folder_path.display(), e),
                });
                continue;
            }
        };
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let slug = entry.file_name().to_string_lossy().to_string();
            let item_path = entry.path().join("item.md");
            if !item_path.is_file() {
                warnings.push(LoadWarning {
                    slug: slug.clone(),
                    folder: folder.to_string(),
                    message: format!("missing {}", item_path.display()),
                });
                continue;
            }
            let parsed = parser::parse_item_md_with_warnings(&item_path, &slug, folder);
            for w in parsed.warnings {
                warnings.push(LoadWarning {
                    slug: slug.clone(),
                    folder: folder.to_string(),
                    message: w,
                });
            }
            issues.push(parsed.issue);
        }
    }

    issues.sort_by(|a, b| a.slug.cmp(&b.slug));
    (issues, warnings)
}

/// Load only frontmatter + title summaries for every issue. Used by the web
/// list endpoint so card metadata can be served without allocating or
/// shipping markdown bodies.
pub fn load_issue_summaries(repo_root: &Path) -> (Vec<IssueSummary>, Vec<LoadWarning>) {
    let (issues, warnings) = load_issues_with_warnings(repo_root);
    (
        issues.into_iter().map(IssueSummary::from).collect(),
        warnings,
    )
}

/// Locate a single issue's `item.md` by slug. Returns `(folder, item_path)`.
///
/// Refuses symlinked issue directories outright and verifies (via canonical
/// path comparison) that the resolved directory stays under
/// `<repo_root>/issues/{open,closed}/`. Without this, a symlinked entry like
/// `issues/open/escaped -> /etc` would be invisible in `load_issues` (which
/// uses `DirEntry::file_type` and so doesn't follow symlinks) but reachable
/// via direct `/api/issues/<slug>` lookups — escape-by-asymmetry.
pub fn locate_issue(repo_root: &Path, slug: &str) -> Result<(String, PathBuf)> {
    let issues_root = repo_root.join(ISSUES_DIR);
    let issues_root_canon = std::fs::canonicalize(&issues_root)
        .with_context(|| format!("cannot canonicalize {}", issues_root.display()))?;

    for folder in &["open", "closed"] {
        let dir = issues_root.join(folder).join(slug);
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
        return Ok((folder.to_string(), item));
    }
    bail!("issue {slug} not found in issues/open/ or issues/closed/")
}

/// Load a single issue by slug. O(1) instead of O(N).
pub fn load_issue(repo_root: &Path, slug: &str) -> Result<Issue> {
    let (folder, item_path) = locate_issue(repo_root, slug)?;
    Ok(parser::parse_item_md(&item_path, slug, &folder))
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

    #[test]
    fn load_issues_returns_sorted_by_slug() {
        let tmp = fresh_repo();
        for (folder, slug) in [
            ("open", "quiet-brave-otter"),
            ("open", "amber-loud-fox"),
            ("closed", "tiny-wild-comet"),
        ] {
            let dir = tmp.path().join("issues").join(folder).join(slug);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                format!("---\nstatus: open\n---\n\n# {slug}\n"),
            )
            .unwrap();
        }
        let issues = load_issues(tmp.path());
        let slugs: Vec<&str> = issues.iter().map(|i| i.slug.as_str()).collect();
        assert_eq!(
            slugs,
            vec!["amber-loud-fox", "quiet-brave-otter", "tiny-wild-comet"]
        );
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
        symlink(
            outside.path(),
            tmp.path().join("issues/open/escaped-not-otter"),
        )
        .unwrap();
        let r = locate_issue(tmp.path(), "escaped-not-otter");
        assert!(r.is_err(), "symlinked issue dir must be rejected");
        let msg = r.unwrap_err().to_string();
        assert!(msg.contains("symlink"), "error should explain why: {msg}");
    }

    #[test]
    fn load_issues_skips_dirs_without_item_md() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/open/quiet-brave-otter")).unwrap();
        fs::create_dir_all(tmp.path().join("issues/open/amber-loud-fox")).unwrap();
        fs::write(
            tmp.path().join("issues/open/amber-loud-fox/item.md"),
            "---\nstatus: open\n---\n\n# T\n",
        )
        .unwrap();
        let issues = load_issues(tmp.path());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].slug, "amber-loud-fox");
    }
}
