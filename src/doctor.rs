//! `issuectl doctor` — repository health-check and one-shot migration from
//! legacy `<NN>-<slug>/` directory layout to slug-only layout.
//!
//! Read-only by default; `--fix` applies migrations and fixes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::{Captures, Regex};

use crate::parser;
use crate::slug;
use crate::write;

/// Parse a legacy item.md and return the numeric `number:` from the
/// frontmatter, if present. The presence of `number:` is the gate for
/// "this is a legacy issue" — directory-name pattern alone is not
/// trustworthy because user-overridden slugs like `100-things-to-fix`
/// look identical.
fn legacy_number_from_frontmatter(item_path: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(item_path).ok()?;
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let yaml = &rest[..end];
    let fm: parser::Frontmatter = serde_yaml::from_str(yaml).ok()?;
    fm.number
}

#[derive(Debug, Clone)]
struct LegacyMigration {
    folder: String,
    old_dir_name: String,
    old_path: PathBuf,
    new_slug: String,
    new_path: PathBuf,
    /// Legacy numeric prefix (the `NN` in `NN-foo`).
    old_number: u32,
}

#[derive(Debug, Default)]
struct DoctorReport {
    legacy_dirs: Vec<LegacyMigration>,
    invalid_slugs: Vec<String>,
    duplicate_slugs: Vec<String>,
    missing_item_md: Vec<String>,
    orphan_epic_refs: Vec<(String, String)>,
    fix_applied: bool,
    files_rewritten: usize,
}

pub fn run(repo_root: &Path, fix: bool, json: bool) -> Result<()> {
    let mut report = scan(repo_root)?;

    if fix {
        apply(repo_root, &mut report)?;
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&render_json(&report, fix))?);
    } else {
        render_text(&report, fix);
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

            // A directory is "legacy" only when its item.md frontmatter
            // contains a numeric `number:` field. The dirname pattern
            // `<NN>-<slug>` alone is not enough — a user-supplied
            // `--slug 100-things-to-fix` would match the pattern but is
            // not legacy and must not be migrated.
            let item_path = path.join("item.md");
            if let Some(number) = legacy_number_from_frontmatter(&item_path) {
                let new_slug = slug::generate_unique(repo_root);
                let new_path = issues_dir.join(folder).join(&new_slug);
                report.legacy_dirs.push(LegacyMigration {
                    folder: folder.to_string(),
                    old_dir_name: dir_name.clone(),
                    old_path: path.clone(),
                    new_slug: new_slug.clone(),
                    new_path,
                    old_number: number,
                });
                *all_slugs.entry(new_slug).or_insert(0) += 1;
            } else {
                if !slug::is_valid(&dir_name) {
                    report.invalid_slugs.push(format!("{folder}/{dir_name}"));
                }
                *all_slugs.entry(dir_name.clone()).or_insert(0) += 1;
            }
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
                let dir = entry.file_name().to_string_lossy().to_string();
                existing_slugs.insert(dir.clone());
                if let Some((_, rest)) = parser::parse_legacy_dir(&dir) {
                    existing_slugs.insert(rest);
                }
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
                let exists = existing_slugs.contains(stripped) || stripped.parse::<u32>().is_ok();
                if !exists {
                    report
                        .orphan_epic_refs
                        .push((slug_id.clone(), epic.to_string()));
                }
            }
        }
    }
    Ok(())
}

fn apply(repo_root: &Path, report: &mut DoctorReport) -> Result<()> {
    if report.legacy_dirs.is_empty() {
        report.fix_applied = true;
        return Ok(());
    }

    // Pre-flight: bail if scan generated overlapping slugs. With ~105M
    // combinations and tens of legacy dirs the chance is negligible, but
    // proceeding into a partial rename and discovering it halfway leaves
    // the repo in a much worse state than refusing up-front.
    if !report.duplicate_slugs.is_empty() {
        bail!(
            "doctor: scan produced colliding new slugs ({:?}); rerun to regenerate",
            report.duplicate_slugs
        );
    }

    // Build maps for reference rewriting.
    let mut number_to_slug: BTreeMap<u32, String> = BTreeMap::new();
    let mut dir_to_slug: BTreeMap<String, String> = BTreeMap::new();
    for m in &report.legacy_dirs {
        let _prev = number_to_slug.insert(m.old_number, m.new_slug.clone());
        // Duplicate legacy numbers are flagged via build_ambiguous below;
        // rewrites for those numbers will be skipped.
        dir_to_slug.insert(m.old_dir_name.clone(), m.new_slug.clone());
    }

    let ambiguous_numbers = build_ambiguous(&report.legacy_dirs);

    // Single-phase atomic rename: old dirname (`<NN>-<slug>`) and new
    // slug (`<intensifier-adj-noun>`) cannot collide, so the temp-suffix
    // shuffle that the previous version did is unnecessary — and worse,
    // an interruption mid-shuffle would leave `*.issuectl-doctor-<pid>`
    // dirs that no subsequent doctor run could recognize.
    for m in &report.legacy_dirs {
        if m.new_path.exists() {
            bail!("target slug dir already exists: {}", m.new_path.display());
        }
        fs::rename(&m.old_path, &m.new_path).with_context(|| {
            format!(
                "cannot rename {} to {}",
                m.old_path.display(),
                m.new_path.display()
            )
        })?;
    }

    for m in &report.legacy_dirs {
        let item_path = m.new_path.join("item.md");
        rewrite_item_frontmatter(&item_path, &m.new_slug, &number_to_slug, &ambiguous_numbers)?;
    }

    // Body-ref rewrites are scoped to `issues/` by default. Documentation
    // outside the issue tree (CHANGELOG, README, design docs) commonly
    // contains literal `#NN` strings that are not issue references, and
    // rewriting them silently is data loss. Users who want a wider sweep
    // can run grep + a one-time replace themselves.
    let issues_path = repo_root.join("issues");
    let scopes = vec![issues_path];
    let files_rewritten =
        rewrite_markdown_in_scopes(&scopes, &number_to_slug, &dir_to_slug, &ambiguous_numbers)?;
    report.files_rewritten = files_rewritten;

    report.fix_applied = true;
    Ok(())
}

fn build_ambiguous(migrations: &[LegacyMigration]) -> BTreeSet<u32> {
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for m in migrations {
        *counts.entry(m.old_number).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .filter_map(|(n, c)| if c > 1 { Some(n) } else { None })
        .collect()
}

fn rewrite_item_frontmatter(
    item_path: &Path,
    new_slug: &str,
    number_to_slug: &BTreeMap<u32, String>,
    ambiguous_numbers: &BTreeSet<u32>,
) -> Result<()> {
    let mut item = write::read_item(item_path)?;

    // Drop legacy `number`, write `slug`.
    write::remove_key(&mut item.frontmatter, "number");
    write::set_string(&mut item.frontmatter, "slug", new_slug);

    // Migrate `epic: NN` (numeric) → `epic: <new_slug>` (string) when unambiguous.
    let epic_key = serde_yaml::Value::String("epic".into());
    if let Some(val) = item.frontmatter.get(&epic_key).cloned() {
        let migrated = match val {
            serde_yaml::Value::Number(n) => n
                .as_u64()
                .and_then(|u| u32::try_from(u).ok())
                .filter(|n| !ambiguous_numbers.contains(n))
                .and_then(|n| number_to_slug.get(&n).cloned()),
            serde_yaml::Value::String(s) => {
                let bare = s.strip_prefix('@').unwrap_or(&s).to_string();
                if let Ok(n) = bare.parse::<u32>() {
                    if !ambiguous_numbers.contains(&n) {
                        number_to_slug.get(&n).cloned()
                    } else {
                        None
                    }
                } else {
                    Some(bare)
                }
            }
            _ => None,
        };
        if let Some(s) = migrated {
            write::set_string(&mut item.frontmatter, "epic", &s);
        }
    }

    // Migrate `related: ["#NN", ...]` → `["@<slug>", ...]` when unambiguous.
    let related_key = serde_yaml::Value::String("related".into());
    if let Some(serde_yaml::Value::Sequence(seq)) = item.frontmatter.get(&related_key).cloned() {
        let mut new_seq: Vec<serde_yaml::Value> = Vec::with_capacity(seq.len());
        for v in seq {
            let migrated = match v {
                serde_yaml::Value::String(ref s) => {
                    if let Some(rest) = s.strip_prefix('#') {
                        if let Ok(n) = rest.parse::<u32>() {
                            if !ambiguous_numbers.contains(&n) {
                                number_to_slug
                                    .get(&n)
                                    .map(|sl| format!("@{sl}"))
                                    .unwrap_or_else(|| s.clone())
                            } else {
                                s.clone()
                            }
                        } else {
                            s.clone()
                        }
                    } else if s.starts_with('@') {
                        s.clone()
                    } else {
                        format!("@{s}")
                    }
                }
                _ => continue,
            };
            new_seq.push(serde_yaml::Value::String(migrated));
        }
        item.frontmatter
            .insert(related_key, serde_yaml::Value::Sequence(new_seq));
    }

    write::write_item(item_path, &item)?;
    Ok(())
}

fn rewrite_markdown_in_scopes(
    scopes: &[PathBuf],
    number_to_slug: &BTreeMap<u32, String>,
    dir_to_slug: &BTreeMap<String, String>,
    ambiguous_numbers: &BTreeSet<u32>,
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
            collect_markdown_files(scope)?
        };
        for path in files {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !seen.insert(canonical) {
                continue;
            }
            let original = fs::read_to_string(&path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            let rewritten =
                rewrite_text(&original, number_to_slug, dir_to_slug, ambiguous_numbers);
            if rewritten != original {
                fs::write(&path, rewritten)
                    .with_context(|| format!("cannot write {}", path.display()))?;
                changed += 1;
            }
        }
    }
    Ok(changed)
}

fn collect_markdown_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk_md(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_md(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
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
            walk_md(&path, out)?;
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

fn rewrite_text(
    text: &str,
    number_to_slug: &BTreeMap<u32, String>,
    dir_to_slug: &BTreeMap<String, String>,
    ambiguous_numbers: &BTreeSet<u32>,
) -> String {
    // 1. Markdown legacy heading `# E10. Title` → `# Title`.
    let heading_re = Regex::new(r"^(# )E?\d+\.\s+(.+)$").expect("valid heading");
    // 2. `#NN` body refs → `@<slug>` (best-effort, skip ambiguous).
    let ref_re = Regex::new(r"#(\d+)\b").expect("valid ref");
    // 3. Path components: `issues/{open,closed}/<NN>-<slug>/` → `issues/.../<new>/`.
    let dir_regexes: Vec<(Regex, String)> = dir_to_slug
        .iter()
        .map(|(old, new)| {
            let pat = format!(
                r"(^|[^A-Za-z0-9_-]){}($|[^A-Za-z0-9_-])",
                regex::escape(old)
            );
            (Regex::new(&pat).expect("valid dir regex"), new.clone())
        })
        .collect();
    let fence_re = Regex::new(r"^\s{0,3}(```+|~~~+)").expect("valid fence");

    let mut rewritten = Vec::new();
    let mut in_fence: Option<String> = None;
    for line in text.lines() {
        // Track fenced code-block state and pass through unchanged
        // inside fences. The fence marker line itself is also passed
        // through (no rewrites apply to ` ```rust` either).
        if let Some(fence) = fence_re.captures(line).and_then(|c| c.get(1)) {
            let marker = fence.as_str().chars().next().unwrap();
            let len = fence.as_str().len();
            let opening = marker.to_string().repeat(len);
            in_fence = match in_fence {
                None => Some(opening.clone()),
                Some(open) if line.trim_start().starts_with(&open) && len >= open.len() => None,
                Some(open) => Some(open),
            };
            rewritten.push(line.to_string());
            continue;
        }
        if in_fence.is_some() {
            rewritten.push(line.to_string());
            continue;
        }

        let line = heading_re.replace(line, "$1$2").to_string();
        let line = ref_re
            .replace_all(&line, |caps: &Captures| {
                let n: u32 = match caps[1].parse() {
                    Ok(v) => v,
                    Err(_) => return caps[0].to_string(),
                };
                if ambiguous_numbers.contains(&n) {
                    return caps[0].to_string();
                }
                match number_to_slug.get(&n) {
                    Some(s) => format!("@{s}"),
                    None => caps[0].to_string(),
                }
            })
            .to_string();
        let mut line = line;
        for (re, new) in &dir_regexes {
            line = re
                .replace_all(&line, |caps: &Captures| {
                    format!("{}{}{}", &caps[1], new, &caps[2])
                })
                .to_string();
        }
        rewritten.push(line);
    }
    let mut out = rewritten.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

// ── Output rendering ────────────────────────────────────────────────────────

fn render_text(report: &DoctorReport, fix: bool) {
    if report.legacy_dirs.is_empty()
        && report.invalid_slugs.is_empty()
        && report.duplicate_slugs.is_empty()
        && report.missing_item_md.is_empty()
        && report.orphan_epic_refs.is_empty()
    {
        println!("Repository OK — no migrations or fixes needed.");
        return;
    }

    if !report.legacy_dirs.is_empty() {
        let title = if fix {
            "Migrated legacy numbered issues:"
        } else {
            "Legacy numbered issues to migrate:"
        };
        println!("{title}");
        for m in &report.legacy_dirs {
            println!(
                "  {}/{}  →  {}/{}",
                m.folder, m.old_dir_name, m.folder, m.new_slug
            );
        }
        println!();
    }
    if !report.invalid_slugs.is_empty() {
        println!("Slugs failing is_valid():");
        for s in &report.invalid_slugs {
            println!("  {s}");
        }
        println!();
    }
    if !report.duplicate_slugs.is_empty() {
        println!("Duplicate slugs (would-be after migration):");
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
        for (slug, epic) in &report.orphan_epic_refs {
            println!("  {slug} → epic: {epic}");
        }
        println!();
    }
    if fix {
        println!(
            "Applied. {} dir(s) migrated, {} markdown file(s) rewritten.",
            report.legacy_dirs.len(),
            report.files_rewritten
        );
    } else {
        println!("Read-only — re-run with --fix to apply.");
    }
}

fn render_json(report: &DoctorReport, fix: bool) -> serde_json::Value {
    let migrations: Vec<serde_json::Value> = report
        .legacy_dirs
        .iter()
        .map(|m| {
            serde_json::json!({
                "folder": m.folder,
                "old_dir": m.old_dir_name,
                "old_number": m.old_number,
                "new_slug": m.new_slug,
            })
        })
        .collect();

    let orphans: Vec<serde_json::Value> = report
        .orphan_epic_refs
        .iter()
        .map(|(s, e)| serde_json::json!({"slug": s, "epic": e}))
        .collect();

    serde_json::json!({
        "fix_applied": fix && report.fix_applied,
        "migrations": migrations,
        "invalid_slugs": report.invalid_slugs,
        "duplicate_slugs": report.duplicate_slugs,
        "missing_item_md": report.missing_item_md,
        "orphan_epic_refs": orphans,
        "files_rewritten": report.files_rewritten,
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

    fn put_legacy(tmp: &TempDir, folder: &str, n: u32, slug: &str, body: &str) {
        let dir = tmp
            .path()
            .join("issues")
            .join(folder)
            .join(format!("{n}-{slug}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), body).unwrap();
    }

    #[test]
    fn scan_detects_legacy_numbered_dirs() {
        let tmp = fresh_repo();
        put_legacy(
            &tmp,
            "open",
            1,
            "alpha",
            "---\nnumber: 1\nstatus: open\n---\n# A\n",
        );
        put_legacy(
            &tmp,
            "closed",
            2,
            "beta",
            "---\nnumber: 2\nstatus: done\n---\n# B\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.legacy_dirs.len(), 2);
    }

    #[test]
    fn scan_does_not_migrate_user_slug_starting_with_digits() {
        // Regression: a user-overridden slug `100-things-to-fix` looks like
        // legacy `<NN>-<slug>` but lacks `number:` in frontmatter — must
        // not be migrated.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/100-things-to-fix");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\nstatus: open\n---\n# T\n").unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r.legacy_dirs.is_empty(), "should not detect as legacy");
    }

    #[test]
    fn fix_renames_dirs_and_rewrites_frontmatter() {
        let tmp = fresh_repo();
        put_legacy(
            &tmp,
            "open",
            1,
            "first",
            "---\nnumber: 1\nstatus: open\nepic: 2\nrelated: [\"#3\"]\n---\n# E1. First\n",
        );
        put_legacy(
            &tmp,
            "open",
            2,
            "epic-one",
            "---\nnumber: 2\nstatus: open\ntype: epic\n---\n# Epic\n",
        );
        put_legacy(
            &tmp,
            "open",
            3,
            "third",
            "---\nnumber: 3\nstatus: open\n---\n# Third\n",
        );
        let mut r = scan(tmp.path()).unwrap();
        apply(tmp.path(), &mut r).unwrap();
        assert!(r.fix_applied);
        // Find the migrated 1-first directory.
        let mig1 = r.legacy_dirs.iter().find(|m| m.old_number == 1).unwrap();
        let item = mig1.new_path.join("item.md");
        let content = fs::read_to_string(&item).unwrap();
        assert!(content.contains(&format!("slug: {}", mig1.new_slug)));
        assert!(!content.contains("number:"));
        assert!(content.contains("# First"), "heading rewritten: {content}");
        // epic: 2 → epic: <slug-of-2>
        let mig2 = r.legacy_dirs.iter().find(|m| m.old_number == 2).unwrap();
        assert!(content.contains(&format!("epic: {}", mig2.new_slug)));
        // related: ['#3'] → ['@<slug-of-3>']
        let mig3 = r.legacy_dirs.iter().find(|m| m.old_number == 3).unwrap();
        assert!(content.contains(&format!("@{}", mig3.new_slug)));
    }

    #[test]
    fn scan_ok_for_clean_slug_repo() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\nstatus: open\n---\n# T\n").unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r.legacy_dirs.is_empty());
        assert!(r.invalid_slugs.is_empty());
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
    fn rewrite_text_swaps_refs_and_paths() {
        let mut nm = BTreeMap::new();
        nm.insert(7, "amber-loud-fox".to_string());
        let mut dm = BTreeMap::new();
        dm.insert("7-something".to_string(), "amber-loud-fox".to_string());
        let amb = BTreeSet::new();
        let text = "See #7 in [link](../7-something/item.md) and #99.\n";
        let out = rewrite_text(text, &nm, &dm, &amb);
        assert!(out.contains("@amber-loud-fox"));
        assert!(out.contains("../amber-loud-fox/item.md"));
        assert!(out.contains("#99"), "unknown number left as-is");
    }

    #[test]
    fn rewrite_text_skips_fenced_code_blocks() {
        let mut nm = BTreeMap::new();
        nm.insert(7, "amber-loud-fox".to_string());
        let dm = BTreeMap::new();
        let amb = BTreeSet::new();
        let text = "Outside #7.\n```rust\n// inside #7\n```\nAfter #7.\n";
        let out = rewrite_text(text, &nm, &dm, &amb);
        assert!(out.contains("Outside @amber-loud-fox"));
        assert!(out.contains("// inside #7"), "code block content untouched");
        assert!(out.contains("After @amber-loud-fox"));
    }

    #[test]
    fn fix_does_not_touch_files_outside_issues() {
        let tmp = fresh_repo();
        put_legacy(
            &tmp,
            "open",
            1,
            "alpha",
            "---\nnumber: 1\nstatus: open\n---\n# A\n",
        );
        // CHANGELOG references `#1` legitimately (release note style).
        let changelog = tmp.path().join("CHANGELOG.md");
        fs::write(&changelog, "# CHANGELOG\n\n- Fixed #1 regression\n").unwrap();
        let mut r = scan(tmp.path()).unwrap();
        apply(tmp.path(), &mut r).unwrap();
        let after = fs::read_to_string(&changelog).unwrap();
        assert!(
            after.contains("Fixed #1 regression"),
            "CHANGELOG outside issues/ must not be rewritten, got: {after}"
        );
    }
}
