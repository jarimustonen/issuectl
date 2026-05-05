//! `issuectl doctor` — repository health check (read-only).
//!
//! Reports invalid slugs, duplicate slugs, directories missing `item.md`,
//! and orphan epic references. The previous legacy-migration mode is gone:
//! the repository format is slug-only.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::parser;
use crate::slug;

#[derive(Debug, Default)]
struct DoctorReport {
    invalid_slugs: Vec<String>,
    duplicate_slugs: Vec<String>,
    missing_item_md: Vec<String>,
    orphan_epic_refs: Vec<(String, String)>,
}

pub fn run(repo_root: &Path, json: bool) -> Result<()> {
    let report = scan(repo_root)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&render_json(&report))?);
    } else {
        render_text(&report);
    }
    Ok(())
}

fn scan(repo_root: &Path) -> Result<DoctorReport> {
    let mut report = DoctorReport::default();
    let issues_dir = repo_root.join("issues");
    let mut all_slugs: BTreeMap<String, usize> = BTreeMap::new();

    for folder in &["open", "closed"] {
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
            let path = entry.path();

            if !path.join("item.md").is_file() {
                report.missing_item_md.push(format!("{folder}/{dir_name}"));
                continue;
            }

            if !slug::is_valid(&dir_name) {
                report.invalid_slugs.push(format!("{folder}/{dir_name}"));
            }
            *all_slugs.entry(dir_name.clone()).or_insert(0) += 1;
        }
    }

    for (s, n) in &all_slugs {
        if *n > 1 {
            report.duplicate_slugs.push(s.clone());
        }
    }

    detect_orphan_epic_refs(repo_root, &mut report)?;
    Ok(report)
}

fn detect_orphan_epic_refs(repo_root: &Path, report: &mut DoctorReport) -> Result<()> {
    let issues_dir = repo_root.join("issues");
    let mut existing_slugs: BTreeSet<String> = BTreeSet::new();
    for folder in &["open", "closed"] {
        let folder_path = issues_dir.join(folder);
        if !folder_path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&folder_path)?.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                existing_slugs.insert(entry.file_name().to_string_lossy().to_string());
            }
        }
    }

    for folder in &["open", "closed"] {
        let folder_path = issues_dir.join(folder);
        if !folder_path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&folder_path)?.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let item = entry.path().join("item.md");
            if !item.is_file() {
                continue;
            }
            let slug_id = entry.file_name().to_string_lossy().to_string();
            let issue = parser::parse_item_md(&item, &slug_id, folder);
            if let Some(epic) = issue.epic.as_deref() {
                let stripped = epic.strip_prefix('@').unwrap_or(epic);
                if !existing_slugs.contains(stripped) {
                    report
                        .orphan_epic_refs
                        .push((slug_id.clone(), epic.to_string()));
                }
            }
        }
    }
    Ok(())
}

fn render_text(report: &DoctorReport) {
    if report.invalid_slugs.is_empty()
        && report.duplicate_slugs.is_empty()
        && report.missing_item_md.is_empty()
        && report.orphan_epic_refs.is_empty()
    {
        println!("Repository OK.");
        return;
    }
    if !report.invalid_slugs.is_empty() {
        println!("Slugs failing is_valid():");
        for s in &report.invalid_slugs {
            println!("  {s}");
        }
        println!();
    }
    if !report.duplicate_slugs.is_empty() {
        println!("Duplicate slugs:");
        for s in &report.duplicate_slugs {
            println!("  {s}");
        }
        println!();
    }
    if !report.missing_item_md.is_empty() {
        println!("Directories missing item.md:");
        for s in &report.missing_item_md {
            println!("  {s}");
        }
        println!();
    }
    if !report.orphan_epic_refs.is_empty() {
        println!("Orphan epic references:");
        for (slug_id, epic) in &report.orphan_epic_refs {
            println!("  {slug_id} → epic: {epic}");
        }
        println!();
    }
}

fn render_json(report: &DoctorReport) -> serde_json::Value {
    let orphans: Vec<serde_json::Value> = report
        .orphan_epic_refs
        .iter()
        .map(|(s, e)| serde_json::json!({"slug": s, "epic": e}))
        .collect();
    serde_json::json!({
        "invalid_slugs": report.invalid_slugs,
        "duplicate_slugs": report.duplicate_slugs,
        "missing_item_md": report.missing_item_md,
        "orphan_epic_refs": orphans,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        fs::create_dir_all(tmp.path().join("issues/closed")).unwrap();
        tmp
    }

    #[test]
    fn scan_ok_for_clean_slug_repo() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\nstatus: open\n---\n# T\n").unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r.invalid_slugs.is_empty());
        assert!(r.duplicate_slugs.is_empty());
    }

    #[test]
    fn scan_flags_invalid_slugs() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/UPPER_NOT_KEBAB");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\n---\n").unwrap();
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.invalid_slugs.len(), 1);
    }

    #[test]
    fn scan_flags_missing_item_md() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/open/foo-bar")).unwrap();
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.missing_item_md.len(), 1);
    }

    #[test]
    fn scan_flags_orphan_epic_refs() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/foo-bar");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\nstatus: open\nepic: missing-epic\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.orphan_epic_refs.len(), 1);
        assert_eq!(r.orphan_epic_refs[0].0, "foo-bar");
    }
}
