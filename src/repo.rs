use std::path::{Path, PathBuf};

use crate::models::Issue;
use crate::parser;

const ISSUES_DIR: &str = "issues";

/// Walk up from start (default: cwd) to find repo root.
/// Looks for `issues/` directory first, then falls back to `.git`.
pub fn find_repo_root(start: Option<&Path>) -> PathBuf {
    let start = start.unwrap_or_else(|| &Path::new("."));
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
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let Some((number, slug)) = parser::parse_issue_dir(&dir_name) else {
                continue;
            };
            let item_path = entry.path().join("item.md");
            if !item_path.is_file() {
                continue;
            }
            let issue = parser::parse_item_md(&item_path, number, &slug, folder);
            result.push(issue);
        }
    }

    result.sort_by_key(|i| i.number);
    result
}

/// Find the highest issue/epic number across open/ and closed/. Returns 0
/// if there are no numbered directories in either folder.
pub fn find_highest_number(repo_root: &Path) -> u32 {
    let issues_dir = repo_root.join(ISSUES_DIR);
    let mut max_num = 0u32;

    for folder in &["open", "closed"] {
        let folder_path = issues_dir.join(folder);
        if !folder_path.is_dir() {
            continue;
        }
        let entries = match std::fs::read_dir(&folder_path) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let dir_name = entry.file_name().to_string_lossy().to_string();
            if let Some((num, _)) = parser::parse_issue_dir(&dir_name) {
                if num > max_num {
                    max_num = num;
                }
            }
        }
    }

    max_num
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
    fn find_highest_number_empty_repo() {
        let tmp = fresh_repo();
        assert_eq!(find_highest_number(tmp.path()), 0);
    }

    #[test]
    fn find_highest_number_no_issues_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(find_highest_number(tmp.path()), 0);
    }

    #[test]
    fn find_highest_number_picks_max_across_folders() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/open/3-foo")).unwrap();
        fs::create_dir_all(tmp.path().join("issues/closed/7-bar")).unwrap();
        fs::create_dir_all(tmp.path().join("issues/open/12-baz")).unwrap();
        assert_eq!(find_highest_number(tmp.path()), 12);
    }

    #[test]
    fn find_highest_number_ignores_non_numbered_dirs() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/open/no-number-here")).unwrap();
        fs::create_dir_all(tmp.path().join("issues/open/foo-3-bar")).unwrap();
        assert_eq!(find_highest_number(tmp.path()), 0);
    }

    #[test]
    fn load_issues_returns_sorted_by_number() {
        let tmp = fresh_repo();
        for (folder, num, slug) in [
            ("open", 5, "five"),
            ("open", 1, "one"),
            ("closed", 3, "three"),
        ] {
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
        let issues = load_issues(tmp.path());
        let numbers: Vec<u32> = issues.iter().map(|i| i.number).collect();
        assert_eq!(numbers, vec![1, 3, 5]);
    }

    #[test]
    fn load_issues_skips_dirs_without_item_md() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/open/1-no-item")).unwrap();
        fs::create_dir_all(tmp.path().join("issues/open/2-has-item")).unwrap();
        fs::write(
            tmp.path().join("issues/open/2-has-item/item.md"),
            "---\nstatus: open\n---\n\n# T\n",
        )
        .unwrap();
        let issues = load_issues(tmp.path());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].number, 2);
    }
}
