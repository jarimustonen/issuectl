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
use crate::{execute_migrate_layout_plan, plan_migrate_layout, MigrateConflict, MigrateMove};

/// Decide whether an issue directory is in the legacy numbered layout and,
/// if so, return its numeric id.
///
/// Two legacy variants exist in the wild:
///
/// 1. **Explicit:** frontmatter carries a numeric `number:` field. This is
///    the form `issuectl new` produced before the slug migration.
/// 2. **Implicit:** the number lives only in the dirname (`<NN>-<slug>/`)
///    and frontmatter has neither `number:` nor `slug:`. Repos that
///    pre-date the `number:` field at all (early grooveserve issues) look
///    like this.
///
/// A user-supplied slug like `--slug 100-things-to-fix` matches the
/// dirname pattern but carries `slug:` in frontmatter — so requiring the
/// absence of `slug:` keeps us from migrating those.
fn legacy_number(item_path: &Path, dir_name: &str) -> Option<u32> {
    // Try to parse frontmatter; treat missing/malformed frontmatter as
    // "no fields" rather than bailing out — pre-`number:` repos sometimes
    // have item.md without YAML at all.
    let fm: parser::Frontmatter = std::fs::read_to_string(item_path)
        .ok()
        .and_then(|text| {
            let trimmed = text.trim_start();
            let rest = trimmed.strip_prefix("---")?;
            let end = rest.find("\n---")?;
            serde_yaml::from_str(&rest[..end]).ok()
        })
        .unwrap_or_default();
    if let Some(n) = fm.number {
        return Some(n);
    }
    if fm.slug.is_some() {
        return None;
    }
    parser::parse_legacy_dir(dir_name).map(|(n, _)| n)
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
    /// Slug-shaped issues still living under `issues/{open,closed}/<slug>/`
    /// (post-flat-layout legacy). Planned moves to `issues/<slug>/`.
    flat_layout_moves: Vec<(String, PathBuf, PathBuf)>,
    flat_layout_migrated: Vec<MigrateMove>,
    flat_layout_conflicts: Vec<MigrateConflict>,
    invalid_slugs: Vec<String>,
    duplicate_slugs: Vec<String>,
    missing_item_md: Vec<String>,
    orphan_epic_refs: Vec<(String, String)>,
    /// Per-issue parse warnings (malformed YAML, unreadable file, ...).
    /// Keeps `doctor` consistent with the web `/api/issues` response,
    /// which already surfaces the same warnings.
    parse_errors: Vec<(String, String)>,
    fix_applied: bool,
    files_rewritten: usize,
    /// Slugs the read-only scan classified as safe to migrate from
    /// `## Notes` → `## Comments`. Populated in `scan()`; consumed
    /// (and emptied) by `rename_notes_to_comments()` during `--fix`.
    notes_to_rename: Vec<String>,
    /// `## Notes` body sections actually renamed during a fix pass.
    /// Subset of `notes_to_rename` after the rewrite has run.
    notes_renamed: Vec<String>,
    /// Slugs whose body has both `## Notes` and `## Comments`, or
    /// multiple `## Notes` headings — merging needs human
    /// judgement, so doctor flags them and skips.
    notes_conflicts: Vec<String>,
}

pub fn run(repo_root: &Path, fix: bool, json: bool) -> Result<()> {
    let mut report = scan(repo_root)?;

    if fix {
        // D2: hold the repo write lock through the apply pass so doctor
        // doesn't race CLI/server mutations. Re-scan under the lock to
        // ensure the plan reflects the locked-state filesystem.
        let _lock = crate::mutate::WriteLock::acquire(repo_root)?;
        report = scan(repo_root)?;
        apply(repo_root, &mut report)?;
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&render_json(&report, fix))?
        );
    } else {
        render_text(&report, fix);
    }
    Ok(())
}

fn scan(repo_root: &Path) -> Result<DoctorReport> {
    let mut report = DoctorReport::default();
    let issues_dir = repo_root.join("issues");

    let mut all_slugs: BTreeMap<String, usize> = BTreeMap::new();

    // Post-flat-layout: walk the canonical `issues/<slug>/` paths plus
    // the legacy `issues/{open,closed}/<slug>/` ones for backward-compat
    // reads. The `folder` axis fed downstream is the kanban-bucket label
    // (legacy-folder name when reading legacy paths; "flat" otherwise).
    let mut entries: Vec<(String, std::path::PathBuf, String)> = Vec::new();
    if let Ok(rd) = fs::read_dir(&issues_dir) {
        for entry in rd.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "open" || name == "closed" || name == "archive" {
                continue;
            }
            entries.push((name, entry.path(), "flat".to_string()));
        }
    }
    for legacy in ["open", "closed"] {
        let folder_path = issues_dir.join(legacy);
        if !folder_path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&folder_path)
            .with_context(|| format!("cannot read {}", folder_path.display()))?
            .flatten()
        {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            entries.push((name, entry.path(), legacy.to_string()));
        }
    }

    for (dir_name, path, folder_owned) in entries {
        let folder = folder_owned.as_str();
        {
            let item_path = path.join("item.md");
            let item_present = item_path.is_file();

            // See `legacy_number` for the detection rules.
            let legacy = if item_present {
                legacy_number(&item_path, &dir_name)
            } else {
                None
            };

            if let Some(number) = legacy {
                let new_slug = slug::generate_unique(repo_root);
                // Always migrate to the canonical flat path — even if
                // the legacy `<NN>-<slug>` dir lives under
                // `issues/{open,closed}/`, doctor `--fix` should bring
                // it forward to the post-flat-layout home in one pass.
                let new_path = issues_dir.join(&new_slug);
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
                // Report invalid slug + duplicate even when item.md is missing —
                // the directory is still a problem worth flagging in one pass.
                if !slug::is_valid(&dir_name) {
                    report.invalid_slugs.push(format!("{folder}/{dir_name}"));
                }
                *all_slugs.entry(dir_name.clone()).or_insert(0) += 1;
            }

            if !item_present {
                report.missing_item_md.push(format!("{folder}/{dir_name}"));
                continue;
            }

            // Surface parse warnings without printing them to stderr (the
            // CLI report includes them at the end). Skip for legacy dirs:
            // the migration pass rewrites their frontmatter anyway, and a
            // missing slug/number combo would be flagged as a parse warning
            // for every legacy issue otherwise.
            if legacy.is_none() {
                let parsed = parser::parse_item_md_with_warnings(&item_path, &dir_name, folder);
                for w in parsed.warnings {
                    report
                        .parse_errors
                        .push((format!("{folder}/{dir_name}"), w));
                }
            }
        }
    }

    for (s, n) in &all_slugs {
        if *n > 1 {
            report.duplicate_slugs.push(s.clone());
        }
    }

    detect_orphan_epic_refs(repo_root, &mut report)?;

    let plan = plan_migrate_layout(repo_root)?;
    report.flat_layout_moves = plan.moves;
    report.flat_layout_conflicts = plan.conflicts;

    // Round-2 finding O6: read-only `doctor` must surface pending
    // Notes migrations and conflicts so users see the work even
    // before running `--fix`. Read-only — no filesystem mutation.
    plan_notes_migration(repo_root, &mut report)?;

    Ok(report)
}

fn plan_notes_migration(repo_root: &Path, report: &mut DoctorReport) -> Result<()> {
    let issues = repo_root.join("issues");
    let Ok(rd) = fs::read_dir(&issues) else {
        return Ok(());
    };
    for entry in rd.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if matches!(name.as_str(), "open" | "closed" | "archive") {
            continue;
        }
        let item_path = entry.path().join("item.md");
        if !item_path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&item_path)
            .with_context(|| format!("cannot read {}", item_path.display()))?;
        match classify_notes(&text) {
            NotesScan::NoOp => {}
            NotesScan::SafeRename => report.notes_to_rename.push(name),
            NotesScan::Conflict => report.notes_conflicts.push(name),
        }
    }
    Ok(())
}

fn detect_orphan_epic_refs(repo_root: &Path, report: &mut DoctorReport) -> Result<()> {
    let issues_dir = repo_root.join("issues");
    let mut existing_slugs: BTreeSet<String> = BTreeSet::new();

    let mut walk = |dir: &Path| -> Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(dir)?.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "open" || name == "closed" || name == "archive" {
                continue;
            }
            existing_slugs.insert(name.clone());
            if let Some((_, rest)) = parser::parse_legacy_dir(&name) {
                existing_slugs.insert(rest);
            }
        }
        Ok(())
    };
    walk(&issues_dir)?;
    walk(&issues_dir.join("open"))?;
    walk(&issues_dir.join("closed"))?;

    let mut walk_for_refs = |dir: &Path| -> Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(dir)?.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "open" || name == "closed" || name == "archive" {
                continue;
            }
            let item = entry.path().join("item.md");
            if !item.is_file() {
                continue;
            }
            let issue = parser::parse_item_md_with_warnings(&item, &name, "open").issue;
            if let Some(epic) = issue.epic.as_deref() {
                let stripped = epic.strip_prefix('@').unwrap_or(epic);
                let exists = existing_slugs.contains(stripped) || stripped.parse::<u32>().is_ok();
                if !exists {
                    report
                        .orphan_epic_refs
                        .push((name.clone(), epic.to_string()));
                }
            }
        }
        Ok(())
    };
    walk_for_refs(&issues_dir)?;
    walk_for_refs(&issues_dir.join("open"))?;
    walk_for_refs(&issues_dir.join("closed"))?;

    Ok(())
}

fn apply(repo_root: &Path, report: &mut DoctorReport) -> Result<()> {
    // Notes → Comments migration is independent of layout migration:
    // it touches body markdown of flat-layout dirs only, never moves
    // files. Run it FIRST so layout-conflict bail-outs don't block
    // unrelated body fixes (round-2 finding O18).
    rename_notes_to_comments(repo_root, report)?;

    // Flat-layout migration runs next: any issue still under
    // `issues/{open,closed}/<slug>/` moves up to `issues/<slug>/`. The
    // pre-acquired write lock in `run` covers this — `execute_migrate_layout_plan`
    // is the lock-free body and must not re-acquire.
    if !report.flat_layout_conflicts.is_empty() {
        bail!(
            "doctor: flat-layout migration has conflicts; resolve before --fix:\n  {}",
            report
                .flat_layout_conflicts
                .iter()
                .map(|c| format!("{}: {}", c.slug, c.detail))
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    }
    if !report.flat_layout_moves.is_empty() {
        let moves = std::mem::take(&mut report.flat_layout_moves);
        report.flat_layout_migrated = execute_migrate_layout_plan(repo_root, moves)?;
        // Paths in `legacy_dirs` (collected by the initial scan) now point
        // at the pre-migration location. Re-scan so the NN-rename pass
        // operates on fresh `old_path`s and picks up any frontmatter-only
        // legacy issues that just moved into the flat layout.
        let fresh = scan(repo_root)?;
        report.legacy_dirs = fresh.legacy_dirs;
        report.duplicate_slugs = fresh.duplicate_slugs;
    }

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

/// Apply the Notes → Comments rename to every slug `scan()` flagged
/// in `notes_to_rename`. Best-effort, sequential (per round-2
/// decision: `O17` is intentionally not preflight-bail). Conflicts
/// are populated by `scan()`; this function does not re-classify.
fn rename_notes_to_comments(repo_root: &Path, report: &mut DoctorReport) -> Result<()> {
    let issues = repo_root.join("issues");
    let planned = std::mem::take(&mut report.notes_to_rename);
    for slug in planned {
        let item_path = issues.join(&slug).join("item.md");
        if !item_path.is_file() {
            continue;
        }
        let original = fs::read_to_string(&item_path)
            .with_context(|| format!("cannot read {}", item_path.display()))?;
        let (rewritten, has_conflict) = migrate_notes_heading(&original);
        if has_conflict {
            // scan() already classified — but if the file changed
            // between scan and apply (concurrent edit under flock is
            // impossible, so this means a manual edit), surface it.
            report.notes_conflicts.push(slug);
            continue;
        }
        if rewritten != original {
            fs::write(&item_path, rewritten)
                .with_context(|| format!("cannot write {}", item_path.display()))?;
            report.notes_renamed.push(slug);
        }
    }
    Ok(())
}

/// Classification of a file's `## Notes` / `## Comments` shape.
#[derive(Debug, PartialEq, Eq)]
enum NotesScan {
    /// File has neither heading, or only `## Comments` — nothing to do.
    NoOp,
    /// File has exactly one `## Notes` and no `## Comments`. Safe to
    /// rewrite to `## Comments`.
    SafeRename,
    /// File has both `## Notes` and `## Comments`, OR more than one
    /// `## Notes`. Renaming silently would produce duplicate
    /// `## Comments` sections (round-2 finding G5/O5), so we skip
    /// and surface the slug for manual merge.
    Conflict,
}

/// Classify a single item.md text. Uses the same fence-aware scanner
/// as the body_sections writer so both agree on what counts as a
/// real heading.
fn classify_notes(text: &str) -> NotesScan {
    let lines: Vec<&str> = text.split('\n').collect();
    let notes = body_sections_scan(&lines, "Notes");
    let comments = body_sections_scan(&lines, "Comments");
    if notes == 0 {
        NotesScan::NoOp
    } else if notes == 1 && comments == 0 {
        NotesScan::SafeRename
    } else {
        NotesScan::Conflict
    }
}

fn body_sections_scan(lines: &[&str], name: &str) -> usize {
    use crate::body_sections::{closes_fence, opening_fence, Fence};
    let mut fence: Option<Fence> = None;
    let mut count = 0usize;
    for l in lines {
        match fence {
            Some(open) if closes_fence(l, open) => fence = None,
            Some(_) => {}
            None => {
                if let Some(o) = opening_fence(l) {
                    fence = Some(o);
                } else if l.strip_prefix("## ").map(|r| r.trim_end()) == Some(name) {
                    count += 1;
                }
            }
        }
    }
    count
}

/// Pure function: rewrite `## Notes` → `## Comments` when there's no
/// pre-existing `## Comments`. Fence-aware so a `## Notes` line
/// inside a code block is preserved verbatim. Returns
/// `(new_text, conflict)` — `conflict=true` when both headings
/// exist or there are multiple `## Notes` headings (caller should
/// skip and surface to the user).
fn migrate_notes_heading(text: &str) -> (String, bool) {
    use crate::body_sections::{closes_fence, opening_fence, Fence};
    match classify_notes(text) {
        NotesScan::NoOp => return (text.to_string(), false),
        NotesScan::Conflict => return (text.to_string(), true),
        NotesScan::SafeRename => {}
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut fence: Option<Fence> = None;
    for l in &lines {
        match fence {
            Some(open) if closes_fence(l, open) => {
                fence = None;
                out.push((*l).to_string());
                continue;
            }
            Some(_) => {
                out.push((*l).to_string());
                continue;
            }
            None => {
                if let Some(o) = opening_fence(l) {
                    fence = Some(o);
                    out.push((*l).to_string());
                    continue;
                }
            }
        }
        if l.strip_prefix("## ").map(|r| r.trim_end()) == Some("Notes") {
            out.push("## Comments".to_string());
        } else {
            out.push((*l).to_string());
        }
    }
    (out.join("\n"), false)
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
            let rewritten = rewrite_text(&original, number_to_slug, dir_to_slug, ambiguous_numbers);
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
        && report.flat_layout_moves.is_empty()
        && report.flat_layout_migrated.is_empty()
        && report.flat_layout_conflicts.is_empty()
        && report.invalid_slugs.is_empty()
        && report.duplicate_slugs.is_empty()
        && report.missing_item_md.is_empty()
        && report.orphan_epic_refs.is_empty()
        && report.parse_errors.is_empty()
        && report.notes_renamed.is_empty()
        && report.notes_to_rename.is_empty()
        && report.notes_conflicts.is_empty()
    {
        println!("Repository OK — no migrations or fixes needed.");
        return;
    }

    if !report.flat_layout_migrated.is_empty() {
        println!("Migrated to flat layout:");
        for m in &report.flat_layout_migrated {
            println!("  {}  ({} → {})", m.slug, m.from.display(), m.to.display());
        }
        println!();
    } else if !report.flat_layout_moves.is_empty() {
        println!("Issues still in legacy `issues/{{open,closed}}/<slug>/` layout:");
        for (slug, src, dest) in &report.flat_layout_moves {
            println!("  {}  ({} → {})", slug, src.display(), dest.display());
        }
        println!();
    }
    if !report.flat_layout_conflicts.is_empty() {
        println!("Flat-layout migration conflicts:");
        for c in &report.flat_layout_conflicts {
            println!("  {}: {}", c.slug, c.detail);
        }
        println!();
    }

    if !report.legacy_dirs.is_empty() {
        let title = if fix {
            "Migrated legacy numbered issues:"
        } else {
            "Legacy numbered issues to migrate:"
        };
        println!("{title}");
        for m in &report.legacy_dirs {
            // Legacy <NN>-<slug> dirs are migrated to the canonical flat
            // path post-flat-layout; print the actual destination rather
            // than the (incorrect) "{folder}/{new}" pre-flat shape.
            println!(
                "  {}/{}  →  {}",
                m.folder,
                m.old_dir_name,
                m.new_path.display()
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
    if !report.parse_errors.is_empty() {
        println!("Parse warnings:");
        for (location, msg) in &report.parse_errors {
            println!("  {location}: {msg}");
        }
        println!();
    }
    if !report.notes_to_rename.is_empty() {
        println!("`## Notes` sections to rename to `## Comments`:");
        for s in &report.notes_to_rename {
            println!("  {s}");
        }
        println!();
    }
    if !report.notes_renamed.is_empty() {
        println!("Renamed `## Notes` → `## Comments`:");
        for s in &report.notes_renamed {
            println!("  {s}");
        }
        println!();
    }
    if !report.notes_conflicts.is_empty() {
        println!("Files with both `## Notes` and `## Comments` (manual merge required):");
        for s in &report.notes_conflicts {
            println!("  {s}");
        }
        println!();
    }
    if fix {
        println!(
            "Applied. {} dir(s) migrated, {} markdown file(s) rewritten, {} `## Notes` rename(s).",
            report.legacy_dirs.len(),
            report.files_rewritten,
            report.notes_renamed.len()
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

    let parse_errors: Vec<serde_json::Value> = report
        .parse_errors
        .iter()
        .map(|(loc, msg)| serde_json::json!({"location": loc, "message": msg}))
        .collect();

    let flat_layout_planned: Vec<serde_json::Value> = report
        .flat_layout_moves
        .iter()
        .map(|(slug, src, dest)| {
            serde_json::json!({
                "slug": slug,
                "from": src.to_string_lossy(),
                "to": dest.to_string_lossy(),
            })
        })
        .collect();
    let flat_layout_migrated: Vec<serde_json::Value> = report
        .flat_layout_migrated
        .iter()
        .map(|m| {
            serde_json::json!({
                "slug": m.slug,
                "from": m.from.to_string_lossy(),
                "to": m.to.to_string_lossy(),
            })
        })
        .collect();
    let flat_layout_conflicts: Vec<serde_json::Value> = report
        .flat_layout_conflicts
        .iter()
        .map(|c| serde_json::json!({"slug": c.slug, "detail": c.detail}))
        .collect();

    serde_json::json!({
        "fix_applied": fix && report.fix_applied,
        "migrations": migrations,
        "flat_layout_planned": flat_layout_planned,
        "flat_layout_migrated": flat_layout_migrated,
        "flat_layout_conflicts": flat_layout_conflicts,
        "invalid_slugs": report.invalid_slugs,
        "duplicate_slugs": report.duplicate_slugs,
        "missing_item_md": report.missing_item_md,
        "orphan_epic_refs": orphans,
        "parse_errors": parse_errors,
        "files_rewritten": report.files_rewritten,
        "notes_to_rename": report.notes_to_rename,
        "notes_renamed": report.notes_renamed,
        "notes_conflicts": report.notes_conflicts,
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
        // legacy `<NN>-<slug>` but is a legitimate new-format issue. The
        // presence of `slug:` in frontmatter is the discriminator —
        // `issuectl new` always writes it for new issues.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/100-things-to-fix");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\nslug: 100-things-to-fix\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r.legacy_dirs.is_empty(), "should not detect as legacy");
    }

    #[test]
    fn scan_detects_legacy_when_only_dirname_carries_number() {
        // Pre-`number:` repos (early grooveserve issues) had the number
        // only in the dirname; frontmatter has neither `number:` nor
        // `slug:`. These must still migrate.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/42-old-style");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\nstatus: open\ntype: feature\n---\n# Old\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.legacy_dirs.len(), 1);
        assert_eq!(r.legacy_dirs[0].old_number, 42);
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
    fn migrate_notes_heading_renames_outside_fences() {
        let body = "---\nstatus: open\n---\n\n# T\n\n## Notes\n\nfirst\n\n```\n## not a heading\n```\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(!conflict);
        assert!(out.contains("## Comments"));
        assert!(!out.contains("## Notes\n"), "Notes heading must be renamed");
        // The fenced `## not a heading` is content and stays put.
        assert!(out.contains("```\n## not a heading\n```"));
    }

    #[test]
    fn migrate_notes_heading_flags_conflict_when_both_exist() {
        let body = "## Notes\n\nx\n\n## Comments\n\ny\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(conflict);
        assert_eq!(out, body, "no rewrite when conflict");
    }

    #[test]
    fn migrate_notes_heading_flags_conflict_when_multiple_notes() {
        // Round-2 finding G5/O5: rewriting two `## Notes` would
        // produce two `## Comments`, leaving the second stranded.
        let body = "## Notes\n\na\n\n## Decisions\n\nx\n\n## Notes\n\nb\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(conflict, "multiple ## Notes must flag a conflict");
        assert_eq!(out, body);
    }

    #[test]
    fn doctor_scan_surfaces_pending_notes_migrations() {
        // Round-2 finding O6: read-only scan must populate
        // `notes_to_rename` and `notes_conflicts` so users see the
        // work even before running --fix.
        let tmp = fresh_repo();
        let safe = tmp.path().join("issues/safe-rename");
        fs::create_dir_all(&safe).unwrap();
        fs::write(
            safe.join("item.md"),
            "---\nstatus: open\n---\n\n## Notes\n\nold\n",
        )
        .unwrap();
        let conflict = tmp.path().join("issues/has-both");
        fs::create_dir_all(&conflict).unwrap();
        fs::write(
            conflict.join("item.md"),
            "---\nstatus: open\n---\n\n## Notes\n\nx\n\n## Comments\n\ny\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.notes_to_rename, vec!["safe-rename".to_string()]);
        assert_eq!(r.notes_conflicts, vec!["has-both".to_string()]);
    }

    #[test]
    fn migrate_notes_heading_skips_fenced_only_occurrence() {
        let body = "```\n## Notes\n```\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(!conflict);
        assert_eq!(out, body, "fenced ## Notes is content, not a heading");
    }

    #[test]
    fn doctor_fix_renames_notes_to_comments_in_flat_layout() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/legacy-notes-here");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T\n\n## Notes\n\nold note\n",
        )
        .unwrap();
        let mut r = scan(tmp.path()).unwrap();
        apply(tmp.path(), &mut r).unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after.contains("## Comments"));
        assert!(!after.contains("## Notes"));
        assert!(after.contains("old note"));
        assert_eq!(r.notes_renamed, vec!["legacy-notes-here".to_string()]);
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
