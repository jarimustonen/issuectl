use std::path::{Path, PathBuf};

use crate::models::Issue;
use crate::parser;

const ISSUES_DIR: &str = "issues";

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
/// Each subdirectory is treated as a slug-named issue. Legacy `<NN>-<slug>`
/// directories are still loaded (their slug becomes the full directory name)
/// so that `issuectl doctor` can detect and migrate them. The `slug` field
/// for legacy dirs reflects the on-disk dirname verbatim.
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

    #[test]
    fn load_issues_includes_legacy_numbered_dirs() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/open/1-old-style")).unwrap();
        fs::write(
            tmp.path().join("issues/open/1-old-style/item.md"),
            "---\nstatus: open\n---\n\n# Old\n",
        )
        .unwrap();
        let issues = load_issues(tmp.path());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].slug, "1-old-style");
    }
}
