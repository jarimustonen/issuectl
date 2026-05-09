//! `issuectl doctor` — repository health-check and one-shot migration from
//! legacy `<NN>-<slug>/` directory layout to slug-only layout.
//!
//! Read-only by default; `--fix` applies migrations and fixes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::{Captures, Regex};

use crate::agents;
use crate::parser;
use crate::schema;
use crate::slug;
use crate::write;
use crate::{execute_migrate_layout_plan, plan_migrate_layout, MigrateConflict, MigrateMove};

/// Single-pass scanner snapshot (D7). Doctor previously re-walked
/// `issues/` once per check category (legacy detection, schema
/// validation, transition warnings, body sections, orphan epic refs,
/// status reconciliation, …). For repos with hundreds of issues the
/// duplicate I/O dominated the runtime; collapsing it into one read
/// and one parse pass is the point of this struct + `scan_issues`.
struct ScannedIssue {
    /// Directory basename (post-flat-layout slug, or legacy `<NN>-<slug>`).
    dir_name: String,
    /// Kanban-bucket axis — `"flat"`, `"open"`, or `"closed"`.
    folder: String,
    /// The issue directory itself (`issues/<...>/<dir_name>`).
    dir_path: PathBuf,
    item_path: PathBuf,
    item_present: bool,
    /// `Some(msg)` when `item_path.is_file()` but the read failed.
    read_error: Option<String>,
    /// Raw `item.md` text. `None` if absent or unreadable.
    text: Option<String>,
    /// Frontmatter as a free-form YAML mapping. `None` when absent or
    /// unparseable as a mapping.
    mapping: Option<serde_yaml::Mapping>,
    /// Text exists but no `---...---` frontmatter block.
    fm_missing: bool,
    /// Frontmatter extracted but YAML parse failed; carries the message
    /// in the same form `collect_schema_violations` used to emit.
    fm_yaml_error: Option<String>,
    /// `parser::parse_item_md_text_with_warnings` output. `None` only
    /// when the file could not be read or wasn't present.
    parsed: Option<parser::ParsedItem>,
    /// `Some(n)` when this directory is a legacy `<NN>-<slug>`
    /// migration candidate; see `legacy_number_from_mapping`.
    legacy_number: Option<u32>,
}

struct ScanResult {
    issues: Vec<ScannedIssue>,
    /// Symlinked entries directly under `issues/`, `issues/open/`, or
    /// `issues/closed/`.
    symlinked_dirs: Vec<String>,
    /// Orphan `.issuectl-tmp-*` files anywhere in the issues tree.
    tempfiles: Vec<PathBuf>,
}

/// Decide whether an issue directory is in the legacy numbered layout
/// and, if so, return its numeric id.
///
/// Two legacy variants exist in the wild:
///
/// 1. **Explicit:** frontmatter carries a numeric `number:` field.
/// 2. **Implicit:** the number lives only in the dirname (`<NN>-<slug>/`)
///    and frontmatter has neither `number:` nor `slug:`.
///
/// A user-supplied slug like `--slug 100-things-to-fix` matches the
/// dirname pattern but carries `slug:` in frontmatter — so the
/// presence of a string-typed `slug:` keeps us from migrating those.
fn legacy_number_from_mapping(
    mapping: Option<&serde_yaml::Mapping>,
    dir_name: &str,
) -> Option<u32> {
    if let Some(m) = mapping {
        if let Some(v) = m.get(serde_yaml::Value::String("number".into())) {
            if let Some(n) = v.as_u64().and_then(|u| u32::try_from(u).ok()) {
                return Some(n);
            }
        }
        if m.get(serde_yaml::Value::String("slug".into()))
            .and_then(|v| v.as_str())
            .is_some()
        {
            return None;
        }
    }
    parser::parse_legacy_dir(dir_name).map(|(n, _)| n)
}

/// One canonical walk over `issues/`: read every `item.md` once, parse
/// frontmatter once, run the typed parser once. Each downstream check
/// consumes this slice instead of re-reading from disk.
fn scan_issues(repo_root: &Path) -> Result<ScanResult> {
    let mut issues = Vec::new();
    let mut symlinked_dirs = Vec::new();
    let mut tempfiles = Vec::new();
    let issues_dir = repo_root.join("issues");
    if !issues_dir.is_dir() {
        return Ok(ScanResult {
            issues,
            symlinked_dirs,
            tempfiles,
        });
    }
    collect_issue_dir(
        &issues_dir,
        "flat",
        &mut issues,
        &mut symlinked_dirs,
        &mut tempfiles,
    )?;
    collect_issue_dir(
        &issues_dir.join("open"),
        "open",
        &mut issues,
        &mut symlinked_dirs,
        &mut tempfiles,
    )?;
    collect_issue_dir(
        &issues_dir.join("closed"),
        "closed",
        &mut issues,
        &mut symlinked_dirs,
        &mut tempfiles,
    )?;
    Ok(ScanResult {
        issues,
        symlinked_dirs,
        tempfiles,
    })
}

fn collect_issue_dir(
    dir: &Path,
    folder: &str,
    issues: &mut Vec<ScannedIssue>,
    symlinked_dirs: &mut Vec<String>,
    tempfiles: &mut Vec<PathBuf>,
) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)
        .with_context(|| format!("cannot read {}", dir.display()))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        if name.starts_with(".issuectl-tmp-") {
            tempfiles.push(path);
            continue;
        }
        let Ok(ftype) = entry.file_type() else { continue };
        if ftype.is_symlink() {
            symlinked_dirs.push(format!("{folder}/{name}"));
            continue;
        }
        if !ftype.is_dir() {
            continue;
        }
        if folder == "flat" && (name == "open" || name == "closed" || name == "archive") {
            continue;
        }
        // Tempfiles can also live next to item.md.
        if let Ok(rd) = fs::read_dir(&path) {
            for inner in rd.flatten() {
                let iname = inner.file_name().to_string_lossy().to_string();
                if iname.starts_with(".issuectl-tmp-") {
                    tempfiles.push(inner.path());
                }
            }
        }
        let item_path = path.join("item.md");
        let item_present = item_path.is_file();
        let mut text: Option<String> = None;
        let mut read_error: Option<String> = None;
        let mut mapping: Option<serde_yaml::Mapping> = None;
        let mut fm_missing = false;
        let mut fm_yaml_error: Option<String> = None;
        let mut parsed: Option<parser::ParsedItem> = None;
        if item_present {
            match fs::read_to_string(&item_path) {
                Ok(t) => {
                    let (fm, _body) = parser::split_frontmatter(&t);
                    match fm {
                        None => fm_missing = true,
                        Some(yaml) => match serde_yaml::from_str::<serde_yaml::Mapping>(yaml) {
                            Ok(m) => mapping = Some(m),
                            Err(e) => {
                                fm_yaml_error =
                                    Some(format!("invalid frontmatter YAML: {e}"));
                            }
                        },
                    }
                    parsed = Some(parser::parse_item_md_text_with_warnings(
                        &t, &name, folder, &item_path,
                    ));
                    text = Some(t);
                }
                Err(e) => {
                    read_error = Some(format!("cannot read {}: {}", item_path.display(), e));
                }
            }
        }
        let legacy_number = if item_present {
            legacy_number_from_mapping(mapping.as_ref(), &name)
        } else {
            None
        };
        issues.push(ScannedIssue {
            dir_name: name,
            folder: folder.to_string(),
            dir_path: path,
            item_path,
            item_present,
            read_error,
            text,
            mapping,
            fm_missing,
            fm_yaml_error,
            parsed,
            legacy_number,
        });
    }
    Ok(())
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
    /// Per-issue schema violations: (location, message). Populated by
    /// validating each issue's frontmatter against `issues/.schema.yaml`
    /// (or the built-in default if absent).
    schema_violations: Vec<(String, String)>,
    /// True if the schema file was missing at scan time. `--fix` writes
    /// the default schema; without `--fix` this is reported as a hint.
    schema_missing: bool,
    /// True if the schema file is present but failed to parse. Causes
    /// `--fix` to skip per-issue schema validation rather than treating
    /// every issue as broken against an unparseable rule set.
    schema_parse_error: Option<String>,
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
    /// Broken cross-references: `(slug, kind, target)` where `kind` is
    /// "epic" / "related" / "blocked_by" and `target` is the unresolved
    /// slug-form ref. (`orphan_epic_refs` is kept separately for
    /// backwards-compat with existing JSON consumers; this list covers
    /// the broader set.)
    broken_refs: Vec<(String, String, String)>,
    /// Dependency cycles via `blocked_by:`. Each inner Vec is a cycle
    /// path (canonicalised so the lowest slug appears first).
    blocked_by_cycles: Vec<Vec<String>>,
    /// Status/closed-date consistency violations. `(slug, message)`.
    status_consistency: Vec<(String, String)>,
    /// Timestamp sanity violations (created > updated, future dates).
    timestamp_issues: Vec<(String, String)>,
    /// Frontmatter keys not declared by the schema (after merging the
    /// user's `.schema.yaml` over the built-in defaults). Surfaced
    /// separately from schema violations because unknown keys are
    /// preserved verbatim by the round-trip; flagging them is purely
    /// a hint that a typo or stray key may be lurking.
    unknown_keys: Vec<(String, String)>,
    /// `item.md` containing git merge-conflict markers. Logged only;
    /// `--fix` never auto-resolves these.
    conflict_markers: Vec<String>,
    /// Orphan `.issuectl-tmp-*` files inside `issues/**` (atomic-write
    /// tempfiles that survived a SIGKILL). `--fix` deletes them.
    orphan_tempfiles: Vec<PathBuf>,
    /// Tempfiles deleted during a fix pass. Subset of `orphan_tempfiles`.
    orphan_tempfiles_removed: Vec<PathBuf>,
    /// Symlinked issue directories — refused by `repo::resolve_layout`,
    /// reported here for the user to either restore or remove.
    symlinked_dirs: Vec<String>,
    /// Slug present at both `issues/open/<slug>/` AND `issues/closed/<slug>/`.
    /// Human-attention only; never auto-fixed.
    both_open_and_closed: Vec<String>,
    /// `issues/closed/<slug>` carrying an active status — legacy folder
    /// repos only. With `--fix`: rewrite status to `done`, set `closed:`
    /// to today if absent.
    closed_with_active_status: Vec<(String, String, PathBuf)>,
    /// `issues/open/<slug>` carrying a closing status — legacy folder
    /// repos only. With `--fix`: rewrite status to `open`, drop `closed:`.
    open_with_closing_status: Vec<(String, String, PathBuf)>,
    /// Status reconciliation rewrites that actually ran during `--fix`.
    status_reconciled: Vec<String>,
    /// Issues whose current status would fail the declarative
    /// transition rules in `.issuectl/transitions.yaml` (e.g. `done`
    /// without an assignee). Surfaced as warnings — these may be
    /// legacy data, so doctor never blocks the exit code on them.
    transition_warnings: Vec<(String, String)>,
    /// Issues missing required H2 body sections per
    /// `.issuectl/transitions.yaml` body_sections rules. `(slug, missing_section)`.
    missing_body_sections: Vec<(String, String)>,
    /// True when `.issuectl/AGENTS.md` exists but its schema-derived
    /// managed block is out of sync with the live schema +
    /// transition rules. Non-critical: `--fix` regenerates the block
    /// in place, preserving prose outside the sentinels. Absent file
    /// is NOT drift — `agents init` is opt-in.
    agents_md_drift: bool,
    /// `.issuectl/AGENTS.md` is structurally malformed (multiple
    /// managed-block pairs, dangling sentinel, end-before-start).
    /// Critical: `--fix` refuses to touch malformed files because
    /// auto-collapsing them would destroy ambiguous user content.
    /// Holds the diagnostic reason for rendering.
    agents_md_malformed: Option<String>,
    /// `.issuectl/AGENTS.md` drift check skipped because the schema
    /// or transition rules failed to parse. Critical: a regenerated
    /// block from defaults would silently overwrite real policy when
    /// the user fixes the unrelated YAML typo. Holds the loader
    /// error message for rendering.
    agents_md_check_skipped: Option<String>,
    /// Set to true after `--fix` regenerates the AGENTS.md managed
    /// block.
    agents_md_regenerated: bool,
}

/// Project an absolute path under the repo root to a repo-relative
/// `String` (UTF-8 lossy fallback). JSON consumers (and the text
/// renderer) shouldn't see absolute filesystem paths leaking into
/// CI logs.
fn rel(repo_root: &Path, p: &Path) -> String {
    p.strip_prefix(repo_root)
        .unwrap_or(p)
        .to_string_lossy()
        .to_string()
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
        // After mutation, the in-memory report mixes "what we planned"
        // with "what's still true". Stale findings (e.g. broken refs we
        // just fixed via legacy-migration rewrites) would mis-report
        // the post-fix state and mis-compute the exit code. Take one
        // clean snapshot and merge the just-applied actions back in
        // for the user-facing summary.
        let mut fresh = scan(repo_root)?;
        fresh.legacy_dirs = std::mem::take(&mut report.legacy_dirs);
        fresh.flat_layout_migrated = std::mem::take(&mut report.flat_layout_migrated);
        fresh.notes_renamed = std::mem::take(&mut report.notes_renamed);
        fresh.orphan_tempfiles_removed =
            std::mem::take(&mut report.orphan_tempfiles_removed);
        fresh.status_reconciled = std::mem::take(&mut report.status_reconciled);
        fresh.files_rewritten = report.files_rewritten;
        fresh.agents_md_regenerated = report.agents_md_regenerated;
        // `fix_applied` means "a mutation actually happened", not "fix
        // mode ran". A clean repo + `--fix` is a no-op; consumers
        // (JSON, text rendering) should be able to distinguish.
        fresh.fix_applied = !fresh.legacy_dirs.is_empty()
            || !fresh.flat_layout_migrated.is_empty()
            || !fresh.notes_renamed.is_empty()
            || !fresh.orphan_tempfiles_removed.is_empty()
            || !fresh.status_reconciled.is_empty()
            || fresh.files_rewritten > 0
            || fresh.agents_md_regenerated;
        report = fresh;
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&render_json(&report, fix, repo_root))?
        );
    } else {
        render_text(&report, fix);
    }
    if has_critical_findings(&report) {
        std::process::exit(1);
    }
    Ok(())
}

/// "Critical" = the repo is in a state the user must intervene on.
/// Routine migrations (legacy dirs, flat-layout moves, notes renames)
/// are not critical: doctor handles them in `--fix`. Parse errors,
/// schema violations, ambiguous slugs, dependency cycles, conflict
/// markers, both-folders presence, broken refs, and status/closed
/// inconsistencies are all critical.
fn has_critical_findings(report: &DoctorReport) -> bool {
    !report.parse_errors.is_empty()
        || !report.schema_violations.is_empty()
        || report.schema_parse_error.is_some()
        || !report.duplicate_slugs.is_empty()
        || !report.invalid_slugs.is_empty()
        || !report.missing_item_md.is_empty()
        || !report.broken_refs.is_empty()
        || !report.blocked_by_cycles.is_empty()
        || !report.status_consistency.is_empty()
        || !report.timestamp_issues.is_empty()
        || !report.conflict_markers.is_empty()
        || !report.symlinked_dirs.is_empty()
        || !report.both_open_and_closed.is_empty()
        || !report.flat_layout_conflicts.is_empty()
        || !report.notes_conflicts.is_empty()
        || report.agents_md_malformed.is_some()
        || report.agents_md_check_skipped.is_some()
}

fn scan(repo_root: &Path) -> Result<DoctorReport> {
    let mut report = DoctorReport::default();
    let scan = scan_issues(repo_root)?;

    populate_slug_and_legacy(&scan, repo_root, &mut report);
    populate_orphan_epic_refs(&scan, &mut report);

    report.schema_missing = !schema::schema_path(repo_root).is_file();
    let schema_value = match schema::load(repo_root) {
        Ok(s) => Some(s),
        Err(e) => {
            report.schema_parse_error = Some(e.to_string());
            None
        }
    };
    if let Some(s) = schema_value.as_ref() {
        populate_schema_violations(&scan, repo_root, s, &mut report);
    }

    // Transition rules + body-section linting. Both are warning-only
    // (legacy data may pre-date the rules).
    let rules = match crate::transitions::load(repo_root) {
        Ok(r) => {
            // N2: cross-validate status references against the schema
            // enum so a typo'd status surfaces here too.
            if let Some(s) = schema_value.as_ref() {
                let universe = schema::status_universe(s);
                if let Err(e) = crate::transitions::validate_status_refs(&r, &universe) {
                    report.parse_errors.push((
                        crate::transitions::RULES_RELATIVE_PATH.to_string(),
                        format!("{e:#}"),
                    ));
                    None
                } else {
                    Some(r)
                }
            } else {
                Some(r)
            }
        }
        Err(e) => {
            report.parse_errors.push((
                crate::transitions::RULES_RELATIVE_PATH.to_string(),
                format!("{e:#}"),
            ));
            None
        }
    };
    if rules.is_some() || schema_value.is_some() {
        populate_transition_warnings(
            &scan,
            rules.as_ref(),
            schema_value.as_ref(),
            &mut report,
        );
        // M2: stable, deterministic ordering for CLI text + JSON +
        // tests. `read_dir` traversal order is platform-dependent.
        report.transition_warnings.sort();
        report.missing_body_sections.sort();
    }

    // AGENTS.md drift. Only flag when the file already exists — the
    // file itself is opt-in (`issuectl agents init`). Both loaders
    // already return defaults on missing file, so a non-Err return
    // means we can trust the values; an Err signals parse/version
    // trouble and we MUST NOT regenerate from defaults (would
    // overwrite real policy with empty rules).
    let agents_path = agents::agents_path(repo_root);
    if agents_path.is_file() {
        if let Ok(text) = fs::read_to_string(&agents_path) {
            match (schema::load(repo_root), crate::transitions::load(repo_root)) {
                (Ok(s), Ok(r)) => match agents::locate_managed_block(&text) {
                    agents::BlockLocation::Malformed { reason } => {
                        report.agents_md_malformed = Some(reason);
                    }
                    _ => {
                        if !agents::managed_in_sync(&text, &s, &r) {
                            report.agents_md_drift = true;
                        }
                    }
                },
                (Err(e), _) | (_, Err(e)) => {
                    report.agents_md_check_skipped = Some(format!("{e:#}"));
                }
            }
        }
    }

    let plan = plan_migrate_layout(repo_root)?;
    report.flat_layout_moves = plan.moves;
    report.flat_layout_conflicts = plan.conflicts;

    // Round-2 finding O6: read-only `doctor` must surface pending
    // Notes migrations and conflicts so users see the work even
    // before running `--fix`.
    populate_notes_migration(&scan, &mut report);

    populate_extended_validation(&scan, repo_root, &mut report);

    Ok(report)
}

/// Slug uniqueness, legacy-migration plan, missing-item-md, parse
/// warnings, invalid slug detection. Mirrors the original main scan
/// loop but consumes `ScanResult` instead of re-reading from disk.
fn populate_slug_and_legacy(scan: &ScanResult, repo_root: &Path, report: &mut DoctorReport) {
    let issues_dir = repo_root.join("issues");
    let mut all_slugs: BTreeMap<String, usize> = BTreeMap::new();

    for s in &scan.issues {
        let location = format!("{}/{}", s.folder, s.dir_name);
        if let Some(number) = s.legacy_number {
            let new_slug = slug::generate_unique(repo_root);
            // Always migrate to the canonical flat path — even if the
            // legacy `<NN>-<slug>` dir lives under
            // `issues/{open,closed}/`, doctor `--fix` should bring it
            // forward to the post-flat-layout home in one pass.
            let new_path = issues_dir.join(&new_slug);
            report.legacy_dirs.push(LegacyMigration {
                folder: s.folder.clone(),
                old_dir_name: s.dir_name.clone(),
                old_path: s.dir_path.clone(),
                new_slug: new_slug.clone(),
                new_path,
                old_number: number,
            });
            *all_slugs.entry(new_slug).or_insert(0) += 1;
        } else {
            // Report invalid slug + duplicate even when item.md is
            // missing — the directory is still a problem worth flagging.
            if !slug::is_valid(&s.dir_name) {
                report.invalid_slugs.push(location.clone());
            }
            *all_slugs.entry(s.dir_name.clone()).or_insert(0) += 1;
        }

        if !s.item_present {
            report.missing_item_md.push(location);
            continue;
        }

        // Surface parse warnings without printing them to stderr.
        // Skip for legacy dirs: the migration pass rewrites their
        // frontmatter anyway, and a missing slug/number combo would
        // otherwise be flagged for every legacy issue.
        if s.legacy_number.is_none() {
            if let Some(parsed) = &s.parsed {
                for w in &parsed.warnings {
                    report.parse_errors.push((location.clone(), w.clone()));
                }
            }
        }
    }

    for (slug_name, n) in &all_slugs {
        if *n > 1 {
            report.duplicate_slugs.push(slug_name.clone());
        }
    }
}

/// Orphan epic-reference detection. Uses the cached parser output for
/// each issue rather than re-reading every `item.md`.
fn populate_orphan_epic_refs(scan: &ScanResult, report: &mut DoctorReport) {
    let mut existing_slugs: BTreeSet<String> = BTreeSet::new();
    for s in &scan.issues {
        existing_slugs.insert(s.dir_name.clone());
        if let Some((_, rest)) = parser::parse_legacy_dir(&s.dir_name) {
            existing_slugs.insert(rest);
        }
    }
    for s in &scan.issues {
        if !s.item_present {
            continue;
        }
        let Some(parsed) = &s.parsed else { continue };
        if let Some(epic) = parsed.issue.epic.as_deref() {
            let stripped = epic.strip_prefix('@').unwrap_or(epic);
            let exists = existing_slugs.contains(stripped) || stripped.parse::<u32>().is_ok();
            if !exists {
                report
                    .orphan_epic_refs
                    .push((s.dir_name.clone(), epic.to_string()));
            }
        }
    }
}

/// v0.5.0 validation suite (reference integrity, status/closed
/// consistency, timestamp sanity, unknown-key flagging, conflict
/// markers, orphan tempfiles, symlinked dirs, status-folder
/// mismatches). Reads no files — operates entirely on the cached
/// `ScanResult`.
fn populate_extended_validation(
    scan: &ScanResult,
    repo_root: &Path,
    report: &mut DoctorReport,
) {
    use chrono::NaiveDate;

    report.symlinked_dirs = scan.symlinked_dirs.clone();
    report.orphan_tempfiles = scan.tempfiles.clone();

    // Group present-issue records by slug across flat + legacy folders.
    let mut by_slug: BTreeMap<String, Vec<&ScannedIssue>> = BTreeMap::new();
    for s in &scan.issues {
        if !s.item_present {
            continue;
        }
        by_slug.entry(s.dir_name.clone()).or_default().push(s);
    }

    // Both open/<slug> AND closed/<slug>: ambiguous; never auto-fix.
    for (slug, hits) in &by_slug {
        let has_open = hits.iter().any(|h| h.folder == "open");
        let has_closed = hits.iter().any(|h| h.folder == "closed");
        if has_open && has_closed {
            report.both_open_and_closed.push(slug.clone());
        }
    }

    // Schema-known field names for unknown-key flagging.
    let schema_fields: BTreeSet<String> = match schema::load(repo_root) {
        Ok(s) => s.fields.keys().cloned().collect(),
        Err(_) => schema::default_schema().fields.keys().cloned().collect(),
    };
    let mut known: BTreeSet<String> = schema_fields;
    // Frontmatter keys the parser/canonical layer recognises but the
    // built-in schema may not declare (e.g. `commits`, `blocked_by`,
    // `number`).
    for k in [
        "created",
        "updated",
        "type",
        "reporter",
        "assignee",
        "owner",
        "status",
        "priority",
        "epic",
        "related",
        "labels",
        "closed",
        "commits",
        "slug",
        "number",
        "blocked_by",
    ] {
        known.insert(k.to_string());
    }

    let today = chrono::Local::now().date_naive();
    let existing_slugs: BTreeSet<String> = by_slug.keys().cloned().collect();
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (slug, hits) in &by_slug {
        // For status reconciliation we want every legacy path
        // occurrence; for the rest, the canonical (flat) hit if any,
        // else the first legacy hit.
        let primary: &ScannedIssue = hits
            .iter()
            .find(|h| h.folder == "flat")
            .copied()
            .unwrap_or(hits[0]);

        let Some(text) = primary.text.as_deref() else {
            continue;
        };
        if has_conflict_markers(text) {
            report.conflict_markers.push(slug.clone());
        }

        let Some(fm) = primary.mapping.as_ref() else {
            continue;
        };

        // Unknown-key flagging.
        for (k, _) in fm.iter() {
            if let serde_yaml::Value::String(name) = k {
                if !known.contains(name) {
                    report.unknown_keys.push((slug.clone(), name.clone()));
                }
            }
        }

        let status = fm
            .get(serde_yaml::Value::String("status".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let closed = fm
            .get(serde_yaml::Value::String("closed".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        let created = fm
            .get(serde_yaml::Value::String("created".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let updated = fm
            .get(serde_yaml::Value::String("updated".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Status/closed consistency.
        if let Some(s) = &status {
            let closing = crate::is_closing_status(s);
            let active = crate::ACTIVE_STATUSES.contains(&s.as_str());
            if closing && closed.is_none() {
                report.status_consistency.push((
                    slug.clone(),
                    format!("closing status {s:?} requires `closed:` date"),
                ));
            }
            if active && closed.is_some() {
                report.status_consistency.push((
                    slug.clone(),
                    format!("active status {s:?} must not carry `closed:`"),
                ));
            }
        }

        // Timestamp sanity.
        let parse = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok();
        let cd = created.as_deref().and_then(parse);
        let ud = updated.as_deref().and_then(parse);
        let xd = closed.as_deref().and_then(parse);
        if let (Some(c), Some(u)) = (cd, ud) {
            if c > u {
                report.timestamp_issues.push((
                    slug.clone(),
                    format!("created ({c}) is after updated ({u})"),
                ));
            }
        }
        for (label, d) in [("created", cd), ("updated", ud), ("closed", xd)] {
            if let Some(d) = d {
                if d > today {
                    report
                        .timestamp_issues
                        .push((slug.clone(), format!("{label} date {d} is in the future")));
                }
            }
        }
        if let (Some(u), Some(x)) = (ud, xd) {
            if x > u {
                report.timestamp_issues.push((
                    slug.clone(),
                    format!("closed ({x}) is after updated ({u})"),
                ));
            }
        }

        // Reference integrity.
        let check_ref = |raw: &str| -> Option<String> {
            let trimmed = raw.trim();
            let bare = trimmed
                .strip_prefix('@')
                .or_else(|| trimmed.strip_prefix('#'))
                .unwrap_or(trimmed);
            if bare.is_empty() {
                return None;
            }
            if bare.chars().all(|c| c.is_ascii_digit()) {
                return Some(format!("{bare} (legacy numeric ref)"));
            }
            if !crate::slug::is_valid(bare) {
                return Some(bare.to_string());
            }
            if !existing_slugs.contains(bare) {
                return Some(bare.to_string());
            }
            None
        };

        if let Some(epic_v) = fm.get(serde_yaml::Value::String("epic".into())) {
            let epic_str = match epic_v {
                serde_yaml::Value::String(s) => Some(s.clone()),
                serde_yaml::Value::Number(n) => Some(n.to_string()),
                _ => None,
            };
            if let Some(epic) = epic_str {
                if let Some(missing) = check_ref(&epic) {
                    report
                        .broken_refs
                        .push((slug.clone(), "epic".into(), missing));
                }
            }
        }
        for key in ["related", "blocked_by"] {
            if let Some(serde_yaml::Value::Sequence(seq)) =
                fm.get(serde_yaml::Value::String(key.into()))
            {
                let mut deps = Vec::new();
                for item in seq {
                    if let Some(s) = item.as_str() {
                        if let Some(missing) = check_ref(s) {
                            report
                                .broken_refs
                                .push((slug.clone(), key.to_string(), missing));
                        } else if key == "blocked_by" {
                            let bare =
                                s.trim().strip_prefix('@').unwrap_or(s.trim()).to_string();
                            if existing_slugs.contains(&bare) {
                                deps.push(bare);
                            }
                        }
                    }
                }
                if key == "blocked_by" && !deps.is_empty() {
                    graph.insert(slug.clone(), deps);
                }
            }
        }

        // Status/folder reconciliation (legacy folders only). Use each
        // hit's own cached mapping — a slug present in flat AND legacy
        // can have divergent status fields.
        let in_both_legacy_folders = hits.iter().any(|h| h.folder == "open")
            && hits.iter().any(|h| h.folder == "closed");
        if in_both_legacy_folders {
            continue;
        }
        for hit in hits {
            if hit.folder != "open" && hit.folder != "closed" {
                continue;
            }
            let Some(fm) = hit.mapping.as_ref() else {
                continue;
            };
            let Some(hit_status) = fm
                .get(serde_yaml::Value::String("status".into()))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            match hit.folder.as_str() {
                "closed" if crate::ACTIVE_STATUSES.contains(&hit_status) => {
                    report.closed_with_active_status.push((
                        slug.clone(),
                        hit_status.to_string(),
                        hit.item_path.clone(),
                    ));
                }
                "open" if crate::is_closing_status(hit_status) => {
                    report.open_with_closing_status.push((
                        slug.clone(),
                        hit_status.to_string(),
                        hit.item_path.clone(),
                    ));
                }
                _ => {}
            }
        }
    }

    report.blocked_by_cycles = detect_cycles(&graph);
}

fn has_conflict_markers(text: &str) -> bool {
    use crate::body_sections::{closes_fence, opening_fence, Fence};
    let mut fence: Option<Fence> = None;
    for line in text.lines() {
        match fence {
            Some(open) if closes_fence(line, open) => {
                fence = None;
                continue;
            }
            Some(_) => continue,
            None => {
                if let Some(o) = opening_fence(line) {
                    fence = Some(o);
                    continue;
                }
            }
        }
        let trimmed = line.trim_end();
        if trimmed.starts_with("<<<<<<< ")
            || trimmed.starts_with(">>>>>>> ")
            || trimmed.starts_with("||||||| ")
            || trimmed == "======="
        {
            return true;
        }
    }
    false
}

/// 3-color DFS: each cycle reported once, rotated so the
/// lexicographically-smallest slug appears first. Adding a `visited`
/// set (the "black" color) caps the work at O(V + E) for cycle
/// detection — without it, a dense DAG re-explores subtrees from
/// every starting node and degrades exponentially.
///
/// This is *not* full Johnson's enumeration: we report at least one
/// cycle per strongly-connected component, not every elementary
/// cycle inside it. Doctor only needs to flag that cycles exist;
/// listing every elementary cycle adds nothing actionable.
fn detect_cycles(graph: &BTreeMap<String, Vec<String>>) -> Vec<Vec<String>> {
    let mut found: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut stack: Vec<String> = Vec::new();
    let mut on_stack: BTreeSet<String> = BTreeSet::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();

    fn dfs(
        node: &str,
        graph: &BTreeMap<String, Vec<String>>,
        stack: &mut Vec<String>,
        on_stack: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        found: &mut BTreeSet<Vec<String>>,
    ) {
        stack.push(node.to_string());
        on_stack.insert(node.to_string());
        if let Some(neigh) = graph.get(node) {
            for n in neigh {
                if on_stack.contains(n) {
                    let start = stack.iter().position(|s| s == n).unwrap();
                    let cycle: Vec<String> = stack[start..].to_vec();
                    let min_idx = cycle
                        .iter()
                        .enumerate()
                        .min_by(|a, b| a.1.cmp(b.1))
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    let mut rotated: Vec<String> = cycle[min_idx..].to_vec();
                    rotated.extend_from_slice(&cycle[..min_idx]);
                    found.insert(rotated);
                } else if graph.contains_key(n) && !visited.contains(n) {
                    dfs(n, graph, stack, on_stack, visited, found);
                }
            }
        }
        stack.pop();
        on_stack.remove(node);
        visited.insert(node.to_string());
    }

    for node in graph.keys() {
        if !visited.contains(node) {
            dfs(node, graph, &mut stack, &mut on_stack, &mut visited, &mut found);
        }
    }
    found.into_iter().collect()
}

fn populate_notes_migration(scan: &ScanResult, report: &mut DoctorReport) {
    for s in &scan.issues {
        if s.folder != "flat" || !s.item_present {
            continue;
        }
        let Some(text) = s.text.as_deref() else {
            continue;
        };
        match classify_notes(text) {
            NotesScan::NoOp => {}
            NotesScan::SafeRename => report.notes_to_rename.push(s.dir_name.clone()),
            NotesScan::Conflict => report.notes_conflicts.push(s.dir_name.clone()),
        }
    }
}

fn populate_schema_violations(
    scan: &ScanResult,
    repo_root: &Path,
    schema: &schema::Schema,
    report: &mut DoctorReport,
) {
    for s in &scan.issues {
        if !s.item_present {
            continue;
        }
        // Skip legacy <NN>-<slug> dirs under the legacy `open/`/`closed/`
        // folders — `--fix` rewrites their frontmatter, so flagging them
        // is just noise. Don't apply the skip to the flat root: a flat
        // issue named `7-alpha` is not legacy by location even though
        // its name matches the legacy shape.
        let in_legacy_folder = s.folder == "open" || s.folder == "closed";
        if in_legacy_folder && parser::parse_legacy_dir(&s.dir_name).is_some() {
            continue;
        }
        let location = format!(
            "{}",
            s.item_path
                .strip_prefix(repo_root)
                .unwrap_or(&s.item_path)
                .display()
        );
        if let Some(err) = &s.read_error {
            report.parse_errors.push((location, err.clone()));
            continue;
        }
        if s.fm_missing {
            report
                .parse_errors
                .push((location, "missing or unterminated frontmatter".into()));
            continue;
        }
        if let Some(err) = &s.fm_yaml_error {
            report.parse_errors.push((location, err.clone()));
            continue;
        }
        let Some(fm) = s.mapping.as_ref() else {
            continue;
        };
        for v in schema::validate(schema, fm) {
            report
                .schema_violations
                .push((location.clone(), v.message()));
        }
    }
}

fn populate_transition_warnings(
    scan: &ScanResult,
    rules: Option<&crate::transitions::TransitionRules>,
    schema: Option<&schema::Schema>,
    report: &mut DoctorReport,
) {
    let rules_active = rules.map(|r| !r.status_rules.is_empty()).unwrap_or(false);
    let sections_active = schema
        .map(|s| !s.body_sections.is_empty())
        .unwrap_or(false);
    if !rules_active && !sections_active {
        return;
    }
    for s in &scan.issues {
        if !s.item_present {
            continue;
        }
        let in_legacy_folder = s.folder == "open" || s.folder == "closed";
        if in_legacy_folder && parser::parse_legacy_dir(&s.dir_name).is_some() {
            continue;
        }
        let Some(parsed) = s.parsed.as_ref() else {
            continue;
        };
        // S5: only skip when essential frontmatter is absent. A legacy
        // numeric-epic ref produces a warning but leaves `status` /
        // `type` intact, so the lint can still run usefully.
        if essential_frontmatter_absent_from_mapping(s.mapping.as_ref()) {
            continue;
        }
        let issue = &parsed.issue;
        if let Some(rules) = rules {
            for msg in crate::transitions::evaluate_existing(rules, issue) {
                report.transition_warnings.push((issue.slug.clone(), msg));
            }
        }
        if let Some(sch) = schema {
            for missing in schema::missing_body_sections(sch, &issue.issue_type, &issue.body) {
                report
                    .missing_body_sections
                    .push((issue.slug.clone(), missing));
            }
        }
    }
}

fn essential_frontmatter_absent_from_mapping(
    mapping: Option<&serde_yaml::Mapping>,
) -> bool {
    let Some(m) = mapping else { return true };
    let has = |k: &str| {
        m.get(serde_yaml::Value::String(k.into()))
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    };
    !has("status") || !has("type")
}

/// Bail before any filesystem mutation if the repo is in a state that
/// `--fix` cannot safely heal. A mid-fix bail leaves the repo in a
/// half-migrated state with no rollback path; surface *every* blocker
/// up-front so the user can resolve them in one pass instead of
/// iterating one preflight failure at a time.
fn preflight_apply(report: &DoctorReport) -> Result<()> {
    let mut blockers: Vec<String> = Vec::new();

    if !report.flat_layout_conflicts.is_empty() {
        let detail = report
            .flat_layout_conflicts
            .iter()
            .map(|c| format!("    {}: {}", c.slug, c.detail))
            .collect::<Vec<_>>()
            .join("\n");
        blockers.push(format!("flat-layout migration conflicts:\n{detail}"));
    }
    if !report.duplicate_slugs.is_empty() {
        blockers.push(format!("duplicate slugs: {:?}", report.duplicate_slugs));
    }
    if !report.both_open_and_closed.is_empty() {
        blockers.push(format!(
            "slugs present in BOTH issues/open/ and issues/closed/: {:?}",
            report.both_open_and_closed
        ));
    }
    if !report.conflict_markers.is_empty() {
        blockers.push(format!(
            "git merge-conflict markers in: {:?}",
            report.conflict_markers
        ));
    }
    // Only HARD parse errors block — recoverable warnings (e.g.
    // "legacy numeric epic ref" hints) would otherwise refuse `--fix`
    // on the very repos the legacy migration was designed to heal.
    let hard_parse_errors: Vec<&(String, String)> = report
        .parse_errors
        .iter()
        .filter(|(_, msg)| is_hard_parse_error(msg))
        .collect();
    if !hard_parse_errors.is_empty() {
        let detail = hard_parse_errors
            .iter()
            .map(|(loc, msg)| format!("    {loc}: {msg}"))
            .collect::<Vec<_>>()
            .join("\n");
        blockers.push(format!(
            "unparseable issue file(s) ({}):\n{detail}",
            hard_parse_errors.len()
        ));
    }

    if !blockers.is_empty() {
        bail!(
            "doctor: cannot safely apply --fix until these issues are resolved:\n\n  - {}",
            blockers.join("\n\n  - ")
        );
    }
    Ok(())
}

/// Distinguish hard parse failures (file unreadable, frontmatter
/// completely unparseable) from soft warnings the legacy migration
/// path is supposed to heal. Only hard failures block `--fix`.
fn is_hard_parse_error(msg: &str) -> bool {
    msg.contains("invalid frontmatter YAML")
        || msg.contains("missing or unterminated frontmatter")
        || msg.contains("invalid YAML frontmatter")
        || msg.starts_with("cannot read")
}

fn apply(repo_root: &Path, report: &mut DoctorReport) -> Result<()> {
    // Preflight: refuse to mutate before checking every condition that
    // would force us to bail mid-way. A mid-fix bail leaves the repo in
    // a half-migrated state with no rollback path, so all "manual
    // attention required" findings must block at the top.
    preflight_apply(report)?;

    // Orphan tempfile cleanup runs FIRST so paths recorded by scan()
    // are still valid: directory migration would invalidate them.
    apply_orphan_tempfiles(report)?;

    // Status/folder reconciliation runs BEFORE the flat-layout
    // migration so the rewrites land at the legacy path that scan()
    // recorded; the subsequent migration moves the corrected file.
    apply_status_reconciliation(repo_root, report)?;

    // Notes → Comments migration is independent of layout migration:
    // it touches body markdown of flat-layout dirs only, never moves
    // files. Run it FIRST so layout-conflict bail-outs don't block
    // unrelated body fixes (round-2 finding O18).
    rename_notes_to_comments(repo_root, report)?;

    regenerate_agents_md(repo_root, report)?;

    // Auto-bootstrap the schema file on --fix. Cheap; idempotent. The
    // bootstrap call also ensures the issues/ directory exists so a
    // brand-new repo with `issuectl doctor --fix` ends in a usable
    // state.
    let issues_dir = repo_root.join("issues");
    fs::create_dir_all(&issues_dir)
        .with_context(|| format!("cannot create {}", issues_dir.display()))?;
    let wrote_default = schema::ensure_default_written(repo_root)?;
    report.schema_missing = false;
    if wrote_default {
        // We just laid down a known-good default; any pre-existing
        // parse error from the report is now stale.
        report.schema_parse_error = None;
    }

    // Flat-layout migration: any issue still under
    // `issues/{open,closed}/<slug>/` moves up to `issues/<slug>/`. The
    // pre-acquired write lock in `run` covers this — `execute_migrate_layout_plan`
    // is the lock-free body and must not re-acquire.
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
/// Regenerate the schema-derived block in `.issuectl/AGENTS.md` when
/// scan flagged drift. No-op if the file is absent (init is opt-in),
/// the block is already in sync, the file is malformed (refuse —
/// auto-collapse would destroy user content), or the schema/rules
/// failed to parse (would regenerate from defaults, overwriting real
/// policy). Doctor's run() already holds `mutate::WriteLock` for the
/// whole apply pass; this function does not re-acquire.
fn regenerate_agents_md(repo_root: &Path, report: &mut DoctorReport) -> Result<()> {
    if !report.agents_md_drift {
        return Ok(());
    }
    if report.agents_md_malformed.is_some() || report.agents_md_check_skipped.is_some() {
        return Ok(());
    }
    let path = agents::agents_path(repo_root);
    if !path.is_file() {
        return Ok(());
    }
    let original = fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let schema = schema::load(repo_root)?;
    let rules = crate::transitions::load(repo_root)?;
    let new_text = agents::regenerate_managed(&original, &schema, &rules)?;
    if new_text != original {
        agents::atomic_write(&path, new_text.as_bytes())?;
        report.agents_md_regenerated = true;
    }
    report.agents_md_drift = false;
    Ok(())
}

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

fn apply_orphan_tempfiles(report: &mut DoctorReport) -> Result<()> {
    let planned = std::mem::take(&mut report.orphan_tempfiles);
    let mut removed = Vec::new();
    for path in planned {
        match fs::remove_file(&path) {
            Ok(_) => removed.push(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(anyhow::Error::from(e))
                    .with_context(|| format!("cannot remove {}", path.display()));
            }
        }
    }
    report.orphan_tempfiles_removed = removed;
    Ok(())
}

fn apply_status_reconciliation(_repo_root: &Path, report: &mut DoctorReport) -> Result<()> {
    let active_to_closed = std::mem::take(&mut report.closed_with_active_status);
    let closing_to_open = std::mem::take(&mut report.open_with_closing_status);
    for (slug, _old_status, item_path) in active_to_closed {
        let mut item = write::read_item(&item_path)?;
        write::set_string(&mut item.frontmatter, "status", "done");
        let has_closed = item
            .frontmatter
            .get(serde_yaml::Value::String("closed".into()))
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if !has_closed {
            write::set_string(&mut item.frontmatter, "closed", &write::today());
        }
        write::write_item(&item_path, &item)?;
        report.status_reconciled.push(slug);
    }
    for (slug, _old_status, item_path) in closing_to_open {
        let mut item = write::read_item(&item_path)?;
        write::set_string(&mut item.frontmatter, "status", "open");
        write::remove_key(&mut item.frontmatter, "closed");
        write::write_item(&item_path, &item)?;
        report.status_reconciled.push(slug);
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
    let has_problems = !report.legacy_dirs.is_empty()
        || !report.flat_layout_moves.is_empty()
        || !report.flat_layout_migrated.is_empty()
        || !report.flat_layout_conflicts.is_empty()
        || !report.invalid_slugs.is_empty()
        || !report.duplicate_slugs.is_empty()
        || !report.missing_item_md.is_empty()
        || !report.orphan_epic_refs.is_empty()
        || !report.parse_errors.is_empty()
        || !report.notes_renamed.is_empty()
        || !report.notes_to_rename.is_empty()
        || !report.notes_conflicts.is_empty()
        || !report.schema_violations.is_empty()
        || report.schema_parse_error.is_some()
        || !report.broken_refs.is_empty()
        || !report.blocked_by_cycles.is_empty()
        || !report.status_consistency.is_empty()
        || !report.timestamp_issues.is_empty()
        || !report.unknown_keys.is_empty()
        || !report.conflict_markers.is_empty()
        || !report.orphan_tempfiles.is_empty()
        || !report.orphan_tempfiles_removed.is_empty()
        || !report.symlinked_dirs.is_empty()
        || !report.both_open_and_closed.is_empty()
        || !report.closed_with_active_status.is_empty()
        || !report.open_with_closing_status.is_empty()
        || !report.status_reconciled.is_empty()
        || !report.transition_warnings.is_empty()
        || !report.missing_body_sections.is_empty()
        || report.agents_md_drift
        || report.agents_md_malformed.is_some()
        || report.agents_md_check_skipped.is_some()
        || report.agents_md_regenerated;
    if !has_problems {
        if report.schema_missing {
            println!(
                "Repository OK — no migrations or fixes needed.\nNote: {} not present yet (will be auto-created on first write or `--fix`).",
                schema::SCHEMA_RELATIVE_PATH
            );
        } else {
            println!("Repository OK — no migrations or fixes needed.");
        }
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
    if report.schema_missing {
        println!(
            "Schema file missing at {} (will be auto-created on first `--fix` or write).",
            schema::SCHEMA_RELATIVE_PATH
        );
        println!();
    }
    if let Some(err) = &report.schema_parse_error {
        println!("Schema file parse error: {err}");
        println!();
    }
    if !report.schema_violations.is_empty() {
        println!("Schema violations:");
        for (location, msg) in &report.schema_violations {
            println!("  {location}: {msg}");
        }
        println!();
    }
    if !report.broken_refs.is_empty() {
        println!("Broken cross-references:");
        for (slug, kind, target) in &report.broken_refs {
            println!("  {slug}: {kind} → {target}");
        }
        println!();
    }
    if !report.blocked_by_cycles.is_empty() {
        println!("Dependency cycles via `blocked_by`:");
        for cycle in &report.blocked_by_cycles {
            println!("  {} → {}", cycle.join(" → "), cycle[0]);
        }
        println!();
    }
    if !report.status_consistency.is_empty() {
        println!("Status / closed-date inconsistencies:");
        for (slug, msg) in &report.status_consistency {
            println!("  {slug}: {msg}");
        }
        println!();
    }
    if !report.timestamp_issues.is_empty() {
        println!("Timestamp sanity issues:");
        for (slug, msg) in &report.timestamp_issues {
            println!("  {slug}: {msg}");
        }
        println!();
    }
    if !report.unknown_keys.is_empty() {
        println!("Unknown frontmatter keys (not declared by schema):");
        for (slug, key) in &report.unknown_keys {
            println!("  {slug}: {key}");
        }
        println!();
    }
    if !report.conflict_markers.is_empty() {
        println!("Files with git merge-conflict markers (manual fix required):");
        for s in &report.conflict_markers {
            println!("  {s}");
        }
        println!();
    }
    if !report.orphan_tempfiles_removed.is_empty() {
        println!("Removed orphan tempfiles:");
        for p in &report.orphan_tempfiles_removed {
            println!("  {}", p.display());
        }
        println!();
    } else if !report.orphan_tempfiles.is_empty() {
        println!("Orphan `.issuectl-tmp-*` files:");
        for p in &report.orphan_tempfiles {
            println!("  {}", p.display());
        }
        println!();
    }
    if !report.symlinked_dirs.is_empty() {
        println!("Symlinked issue directories (refused):");
        for s in &report.symlinked_dirs {
            println!("  {s}");
        }
        println!();
    }
    if !report.both_open_and_closed.is_empty() {
        println!("Slugs present in BOTH `issues/open/` and `issues/closed/` (manual fix required):");
        for s in &report.both_open_and_closed {
            println!("  {s}");
        }
        println!();
    }
    if !report.closed_with_active_status.is_empty() {
        println!("`closed/<slug>` with active status:");
        for (slug, st, _) in &report.closed_with_active_status {
            println!("  {slug} (status: {st})");
        }
        println!();
    }
    if !report.open_with_closing_status.is_empty() {
        println!("`open/<slug>` with closing status:");
        for (slug, st, _) in &report.open_with_closing_status {
            println!("  {slug} (status: {st})");
        }
        println!();
    }
    if !report.status_reconciled.is_empty() {
        println!("Reconciled status/folder mismatches:");
        for s in &report.status_reconciled {
            println!("  {s}");
        }
        println!();
    }
    if !report.transition_warnings.is_empty() {
        println!("Transition-rule warnings (warning-only — legacy data may pre-date the rules):");
        for (slug, msg) in &report.transition_warnings {
            println!("  {slug}: {msg}");
        }
        println!();
    }
    if !report.missing_body_sections.is_empty() {
        println!("Missing required body sections:");
        for (slug, section) in &report.missing_body_sections {
            println!("  {slug}: ## {section}");
        }
        println!();
    }
    if let Some(reason) = &report.agents_md_malformed {
        println!(
            "{} is malformed: {} — fix manually before re-running --fix.",
            agents::AGENTS_RELATIVE_PATH,
            reason
        );
        println!();
    }
    if let Some(err) = &report.agents_md_check_skipped {
        println!(
            "{} drift check skipped: {} (fix the schema/rules file first).",
            agents::AGENTS_RELATIVE_PATH,
            err
        );
        println!();
    }
    if report.agents_md_regenerated {
        println!(
            "Regenerated schema-derived block in {}.",
            agents::AGENTS_RELATIVE_PATH
        );
        println!();
    } else if report.agents_md_drift {
        println!(
            "{} schema-derived block is out of date (re-run with --fix to regenerate).",
            agents::AGENTS_RELATIVE_PATH
        );
        println!();
    }
    if fix {
        println!(
            "Applied. {} dir(s) migrated, {} markdown file(s) rewritten, {} `## Notes` rename(s), {} AGENTS.md block(s) regenerated.",
            report.legacy_dirs.len(),
            report.files_rewritten,
            report.notes_renamed.len(),
            if report.agents_md_regenerated { 1 } else { 0 }
        );
    } else {
        println!("Read-only — re-run with --fix to apply.");
    }
}

fn render_json(report: &DoctorReport, fix: bool, repo_root: &Path) -> serde_json::Value {
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

    let schema_violations: Vec<serde_json::Value> = report
        .schema_violations
        .iter()
        .map(|(loc, msg)| serde_json::json!({"location": loc, "message": msg}))
        .collect();

    let broken_refs: Vec<serde_json::Value> = report
        .broken_refs
        .iter()
        .map(|(slug, kind, target)| {
            serde_json::json!({"slug": slug, "kind": kind, "target": target})
        })
        .collect();
    let status_consistency: Vec<serde_json::Value> = report
        .status_consistency
        .iter()
        .map(|(s, m)| serde_json::json!({"slug": s, "message": m}))
        .collect();
    let timestamp_issues: Vec<serde_json::Value> = report
        .timestamp_issues
        .iter()
        .map(|(s, m)| serde_json::json!({"slug": s, "message": m}))
        .collect();
    let unknown_keys: Vec<serde_json::Value> = report
        .unknown_keys
        .iter()
        .map(|(s, k)| serde_json::json!({"slug": s, "key": k}))
        .collect();
    let orphan_tempfiles: Vec<String> = report
        .orphan_tempfiles
        .iter()
        .map(|p| rel(repo_root, p))
        .collect();
    let orphan_tempfiles_removed: Vec<String> = report
        .orphan_tempfiles_removed
        .iter()
        .map(|p| rel(repo_root, p))
        .collect();
    let closed_with_active: Vec<serde_json::Value> = report
        .closed_with_active_status
        .iter()
        .map(|(s, st, _)| serde_json::json!({"slug": s, "status": st}))
        .collect();
    let open_with_closing: Vec<serde_json::Value> = report
        .open_with_closing_status
        .iter()
        .map(|(s, st, _)| serde_json::json!({"slug": s, "status": st}))
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
        "schema_missing": report.schema_missing,
        "schema_parse_error": report.schema_parse_error,
        "schema_violations": schema_violations,
        "files_rewritten": report.files_rewritten,
        "notes_to_rename": report.notes_to_rename,
        "notes_renamed": report.notes_renamed,
        "notes_conflicts": report.notes_conflicts,
        "broken_refs": broken_refs,
        "blocked_by_cycles": report.blocked_by_cycles,
        "status_consistency": status_consistency,
        "timestamp_issues": timestamp_issues,
        "unknown_keys": unknown_keys,
        "conflict_markers": report.conflict_markers,
        "orphan_tempfiles": orphan_tempfiles,
        "orphan_tempfiles_removed": orphan_tempfiles_removed,
        "symlinked_dirs": report.symlinked_dirs,
        "both_open_and_closed": report.both_open_and_closed,
        "closed_with_active_status": closed_with_active,
        "open_with_closing_status": open_with_closing,
        "status_reconciled": report.status_reconciled,
        "transition_warnings": report
            .transition_warnings
            .iter()
            .map(|(s, m)| serde_json::json!({"slug": s, "message": m}))
            .collect::<Vec<_>>(),
        "missing_body_sections": report
            .missing_body_sections
            .iter()
            .map(|(s, sec)| serde_json::json!({"slug": s, "section": sec}))
            .collect::<Vec<_>>(),
        "agents_md_drift": report.agents_md_drift,
        "agents_md_malformed": report.agents_md_malformed,
        "agents_md_check_skipped": report.agents_md_check_skipped,
        "agents_md_regenerated": report.agents_md_regenerated,
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

    #[test]
    fn scan_flags_schema_violation_for_missing_required_field() {
        let tmp = fresh_repo();
        // Issue missing `priority` (required by default schema).
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.schema_violations
                .iter()
                .any(|(_, msg)| msg.contains("priority")),
            "expected `priority` violation, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn scan_flags_schema_violation_for_invalid_enum() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: nonsense\ncreated: 2026-01-01\nstatus: open\npriority: normal\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.schema_violations
                .iter()
                .any(|(_, msg)| msg.contains("type") && msg.contains("nonsense")),
            "expected enum violation, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn scan_reports_schema_missing_when_file_absent() {
        let tmp = fresh_repo();
        let r = scan(tmp.path()).unwrap();
        assert!(r.schema_missing);
        assert!(r.schema_parse_error.is_none());
    }

    #[test]
    fn fix_writes_default_schema_when_missing() {
        let tmp = fresh_repo();
        let mut r = scan(tmp.path()).unwrap();
        assert!(r.schema_missing);
        apply(tmp.path(), &mut r).unwrap();
        let path = tmp.path().join("issues/.schema.yaml");
        assert!(path.is_file(), "schema file should be auto-written");
        // Should contain the canonical built-in fields.
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("type:"));
        assert!(content.contains("status:"));
    }

    #[test]
    fn scan_skips_legacy_dirs_for_schema_violations() {
        // A legacy <NN>-<slug> dir is rewritten by --fix; flagging it
        // as schema-violating would just be noise.
        let tmp = fresh_repo();
        put_legacy(
            &tmp,
            "open",
            7,
            "alpha",
            "---\nnumber: 7\nstatus: open\n---\n# A\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.schema_violations.is_empty(),
            "legacy dirs should not generate schema violations, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn schema_walk_reports_malformed_yaml_as_parse_error() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        // Frontmatter that the lenient `Mapping` parser also rejects.
        fs::write(dir.join("item.md"), "---\nfoo: : :\n---\n# T\n").unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.parse_errors
                .iter()
                .any(|(_, msg)| msg.contains("YAML") || msg.contains("yaml") || msg.contains("invalid")),
            "expected parse error report, got {:?}",
            r.parse_errors
        );
    }

    #[test]
    fn schema_walk_uses_repo_relative_paths_not_flat_prefix() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        // Missing `priority`. Location must be a real path, not "flat/...".
        let (loc, _) = r
            .schema_violations
            .iter()
            .find(|(_, msg)| msg.contains("priority"))
            .expect("expected priority violation");
        assert!(loc.contains("issues/quiet-brave-otter"), "got {loc:?}");
        assert!(!loc.starts_with("flat/"), "got {loc:?}");
    }

    #[test]
    fn schema_walk_does_not_skip_flat_issue_with_legacy_shape_name() {
        // A user who passes `--slug 12-things-to-do` ends up with a
        // flat-layout issue whose name matches the legacy `<NN>-<slug>`
        // shape. It must still be checked for schema violations.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/12-things-to-do");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.schema_violations
                .iter()
                .any(|(_, msg)| msg.contains("priority")),
            "expected violation on flat NN-shaped slug, got {:?}",
            r.schema_violations
        );
    }

    fn put_flat(tmp: &TempDir, slug: &str, body: &str) {
        let dir = tmp.path().join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), body).unwrap();
    }

    #[test]
    fn flags_broken_epic_ref() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nepic: nonexistent-ghost-fox\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.broken_refs
                .iter()
                .any(|(s, k, t)| s == "quiet-brave-otter" && k == "epic" && t == "nonexistent-ghost-fox"),
            "broken_refs={:?}",
            r.broken_refs
        );
    }

    #[test]
    fn flags_numeric_legacy_ref_in_flat_repo() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nepic: 5\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.broken_refs
                .iter()
                .any(|(_, k, t)| k == "epic" && t.contains("legacy")),
            "expected legacy-numeric flag, got {:?}",
            r.broken_refs
        );
    }

    #[test]
    fn conflict_marker_check_skips_fenced_blocks() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n```\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> branch\n```\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.conflict_markers.is_empty(), "got {:?}", r.conflict_markers);
    }

    #[test]
    fn detect_cycles_visits_each_node_once() {
        // Acyclic diamond: A→B, A→C, B→D, C→D. Without the visited
        // set, D was traversed from both B and C; with it, the
        // traversal stops after the first complete walk.
        let mut g: BTreeMap<String, Vec<String>> = BTreeMap::new();
        g.insert("a".into(), vec!["b".into(), "c".into()]);
        g.insert("b".into(), vec!["d".into()]);
        g.insert("c".into(), vec!["d".into()]);
        g.insert("d".into(), vec![]);
        let cycles = detect_cycles(&g);
        assert!(cycles.is_empty(), "no cycles in DAG, got {cycles:?}");
    }

    #[test]
    fn does_not_flag_existing_epic_ref() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "real-epic-here",
            "---\ntype: epic\nstatus: open\npriority: normal\n---\n# E\n",
        );
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nepic: real-epic-here\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.broken_refs.is_empty(), "got {:?}", r.broken_refs);
    }

    #[test]
    fn flags_broken_blocked_by_ref() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nblocked_by: ['@nope-not-here']\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .broken_refs
            .iter()
            .any(|(_, k, t)| k == "blocked_by" && t == "nope-not-here"));
    }

    #[test]
    fn detects_blocked_by_cycle() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "alpha-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\nblocked_by: ['@beta-bright-cat']\n---\n# A\n",
        );
        put_flat(
            &tmp,
            "beta-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\nblocked_by: ['@alpha-bright-cat']\n---\n# B\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.blocked_by_cycles.len(), 1);
        let cycle = &r.blocked_by_cycles[0];
        assert_eq!(cycle[0], "alpha-bright-cat");
        assert!(cycle.contains(&"beta-bright-cat".to_string()));
    }

    #[test]
    fn no_cycle_for_acyclic_chain() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "alpha-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\nblocked_by: ['@beta-bright-cat']\n---\n# A\n",
        );
        put_flat(
            &tmp,
            "beta-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# B\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.blocked_by_cycles.is_empty());
    }

    #[test]
    fn flags_closing_status_without_closed_date() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: done\npriority: normal\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .status_consistency
            .iter()
            .any(|(_, m)| m.contains("closing") && m.contains("closed")));
    }

    #[test]
    fn flags_active_status_with_closed_date() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nclosed: 2026-01-01\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .status_consistency
            .iter()
            .any(|(_, m)| m.contains("active") && m.contains("closed")));
    }

    #[test]
    fn does_not_flag_consistent_status() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: done\npriority: normal\nclosed: 2026-01-01\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.status_consistency.is_empty(), "{:?}", r.status_consistency);
    }

    #[test]
    fn flags_created_after_updated() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-05-01\nupdated: 2026-04-01\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .timestamp_issues
            .iter()
            .any(|(_, m)| m.contains("created") && m.contains("after")));
    }

    #[test]
    fn flags_future_dates() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2999-01-01\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .timestamp_issues
            .iter()
            .any(|(_, m)| m.contains("future")));
    }

    #[test]
    fn does_not_flag_sane_dates() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-01-01\nupdated: 2026-02-01\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.timestamp_issues.is_empty(), "{:?}", r.timestamp_issues);
    }

    #[test]
    fn flags_unknown_frontmatter_key() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nwhimsy: 1\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.unknown_keys.iter().any(|(_, k)| k == "whimsy"));
    }

    #[test]
    fn does_not_flag_schema_known_custom_key() {
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: false\n",
        )
        .unwrap();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nteam: payments\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            !r.unknown_keys.iter().any(|(_, k)| k == "team"),
            "team is schema-known: {:?}",
            r.unknown_keys
        );
    }

    #[test]
    fn preflight_aggregates_blockers_in_one_message() {
        // Repo with TWO independent blockers: a slug present in both
        // legacy folders + a file with conflict markers. The user
        // should see both in a single bail, not have to iterate.
        let tmp = fresh_repo();
        for f in ["open", "closed"] {
            let dir = tmp.path().join("issues").join(f).join("quiet-brave-otter");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
            )
            .unwrap();
        }
        let conflicted = tmp.path().join("issues/alpha-bright-cat");
        fs::create_dir_all(&conflicted).unwrap();
        fs::write(
            conflicted.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> b\n",
        )
        .unwrap();
        let mut r = scan(tmp.path()).unwrap();
        let err = apply(tmp.path(), &mut r).unwrap_err().to_string();
        assert!(err.contains("BOTH"), "missing both-folders blocker: {err}");
        assert!(
            err.contains("merge-conflict markers"),
            "missing conflict-marker blocker: {err}"
        );
    }

    #[test]
    fn preflight_does_not_block_on_soft_parse_warnings() {
        // A legacy-numeric epic ref produces a parser warning (now
        // categorised as "soft") but should NOT prevent --fix from
        // running its migration pass.
        let tmp = fresh_repo();
        put_legacy(
            &tmp,
            "open",
            7,
            "alpha",
            "---\nnumber: 7\nstatus: open\nepic: 12\n---\n# A\n",
        );
        let mut r = scan(tmp.path()).unwrap();
        // Pre-fix: there are likely parser warnings in `parse_errors`.
        // None of them should trip the hard-error preflight check.
        apply(tmp.path(), &mut r).expect("--fix should not refuse on soft parse warnings");
    }

    #[test]
    fn flags_conflict_markers_and_apply_refuses() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> branch\n",
        );
        let mut r = scan(tmp.path()).unwrap();
        assert!(r.conflict_markers.iter().any(|s| s == "quiet-brave-otter"));
        let before = fs::read_to_string(
            tmp.path().join("issues/quiet-brave-otter/item.md"),
        )
        .unwrap();
        // Preflight bails before any mutation runs.
        let err = apply(tmp.path(), &mut r).unwrap_err();
        assert!(
            err.to_string().contains("merge-conflict markers"),
            "got: {err}"
        );
        let after = fs::read_to_string(
            tmp.path().join("issues/quiet-brave-otter/item.md"),
        )
        .unwrap();
        assert_eq!(before, after, "conflict markers must not be auto-fixed");
    }

    #[test]
    fn does_not_flag_clean_file() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.conflict_markers.is_empty());
    }

    #[test]
    fn detects_and_removes_orphan_tempfiles_with_fix() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
        );
        let orphan = tmp
            .path()
            .join("issues/quiet-brave-otter/.issuectl-tmp-XYZ");
        fs::write(&orphan, "leftover").unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r.orphan_tempfiles.iter().any(|p| p == &orphan));
        let mut r = scan(tmp.path()).unwrap();
        apply(tmp.path(), &mut r).unwrap();
        assert!(!orphan.exists(), "tempfile should be removed by --fix");
        assert!(r.orphan_tempfiles_removed.iter().any(|p| p == &orphan));
    }

    #[test]
    fn flags_both_open_and_closed_present() {
        let tmp = fresh_repo();
        let s = "quiet-brave-otter";
        for f in ["open", "closed"] {
            let dir = tmp.path().join("issues").join(f).join(s);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
            )
            .unwrap();
        }
        let r = scan(tmp.path()).unwrap();
        assert!(r.both_open_and_closed.iter().any(|x| x == s));
    }

    #[test]
    fn reconciles_closed_with_active_status() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/closed/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
        )
        .unwrap();
        let mut r = scan(tmp.path()).unwrap();
        assert!(r
            .closed_with_active_status
            .iter()
            .any(|(s, _, _)| s == "quiet-brave-otter"));
        apply(tmp.path(), &mut r).unwrap();
        // Flat-layout migration runs in the same apply pass and moves
        // the file from `issues/closed/<slug>/` to `issues/<slug>/`.
        let migrated = tmp.path().join("issues/quiet-brave-otter/item.md");
        let after = fs::read_to_string(&migrated).unwrap();
        assert!(after.contains("status: done"), "got: {after}");
        assert!(after.contains("closed:"));
    }

    #[test]
    fn reconciles_open_with_closing_status() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: done\npriority: normal\nclosed: 2026-01-01\n---\n# T\n",
        )
        .unwrap();
        let mut r = scan(tmp.path()).unwrap();
        apply(tmp.path(), &mut r).unwrap();
        let migrated = tmp.path().join("issues/quiet-brave-otter/item.md");
        let after = fs::read_to_string(&migrated).unwrap();
        assert!(after.contains("status: open"), "got: {after}");
        assert!(!after.contains("closed:"), "closed should be dropped: {after}");
    }

    #[cfg(unix)]
    #[test]
    fn detects_symlinked_issue_dir() {
        // Symlink target need not exist meaningfully; we just check
        // that doctor surfaces the symlink.
        let tmp = fresh_repo();
        let target = tmp.path().join("elsewhere");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("item.md"), "---\n---\n# x\n").unwrap();
        let link = tmp.path().join("issues/quiet-brave-otter");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .symlinked_dirs
            .iter()
            .any(|s| s.contains("quiet-brave-otter")));
    }

    #[cfg(unix)]
    #[test]
    fn detects_broken_symlinked_issue_dir() {
        let tmp = fresh_repo();
        let link = tmp.path().join("issues/quiet-brave-otter");
        std::os::unix::fs::symlink("/nonexistent/target/path", &link).unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .symlinked_dirs
            .iter()
            .any(|s| s.contains("quiet-brave-otter")));
    }

    #[test]
    fn schema_validation_honours_user_edited_required_field() {
        let tmp = fresh_repo();
        // Custom schema requires a `team` field.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: open\npriority: normal\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.schema_violations
                .iter()
                .any(|(_, msg)| msg.contains("team")),
            "expected `team` violation, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn scan_surfaces_transition_warnings_and_missing_sections() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(
            tmp.path().join(".issuectl/transitions.yaml"),
            "version: 1\nstatus_rules:\n  done:\n    requires_assignee: true\n",
        )
        .unwrap();
        // Body-section requirements moved to schema (C6).
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields: {}\nbody_sections:\n  bug: [Steps to Reproduce, Expected, Actual]\n",
        )
        .unwrap();
        // Issue is `done` without an assignee → transition warning.
        // Bug is missing the required body sections → body section warning.
        let dir = tmp.path().join("issues/legacy-bug");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: done\npriority: normal\n---\n\n# T\n\n## Description\n\nx\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.transition_warnings
                .iter()
                .any(|(s, m)| s == "legacy-bug" && m.contains("assignee")),
            "expected assignee warning, got {:?}",
            r.transition_warnings
        );
        let missing: Vec<_> = r
            .missing_body_sections
            .iter()
            .filter(|(s, _)| s == "legacy-bug")
            .map(|(_, sec)| sec.clone())
            .collect();
        assert!(missing.contains(&"Steps to Reproduce".to_string()));
        assert!(missing.contains(&"Expected".to_string()));
        assert!(missing.contains(&"Actual".to_string()));
    }

    #[test]
    fn agents_md_drift_not_flagged_when_file_absent() {
        let tmp = fresh_repo();
        let r = scan(tmp.path()).unwrap();
        assert!(!r.agents_md_drift);
    }

    #[test]
    fn agents_md_drift_detected_after_schema_change() {
        let tmp = fresh_repo();
        // Write a fresh AGENTS.md against the default schema.
        agents::run_init(tmp.path(), false, false).unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(!r.agents_md_drift, "freshly-written file is in sync");

        // Mutate the schema so the rendered block no longer matches.
        let schema_path = tmp.path().join("issues/.schema.yaml");
        fs::write(
            &schema_path,
            "version: 1\nbody_sections:\n  bug: [Reproduction]\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r.agents_md_drift, "drift after schema edit");
    }

    #[test]
    fn agents_md_fix_regenerates_block_preserving_prose() {
        let tmp = fresh_repo();
        let path = tmp.path().join(agents::AGENTS_RELATIVE_PATH);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Hand-written file with a stale managed block + custom prose.
        let custom = format!(
            "# My custom heading\n\nMy hand-written notes.\n\n{}\n\nstale body\n{}\n\nClosing prose.\n",
            agents::MANAGED_START,
            agents::MANAGED_END
        );
        fs::write(&path, &custom).unwrap();

        let mut report = scan(tmp.path()).unwrap();
        assert!(report.agents_md_drift);
        apply(tmp.path(), &mut report).unwrap();
        assert!(report.agents_md_regenerated);

        let after = fs::read_to_string(&path).unwrap();
        assert!(after.starts_with("# My custom heading\n\nMy hand-written notes.\n\n"));
        assert!(after.contains("Closing prose.\n"));
        assert!(!after.contains("stale body"));
        assert!(after.contains(agents::MANAGED_START));
        assert!(after.contains(agents::MANAGED_END));
    }

    /// Single-pass `scan_issues` powers every check. This fixture wires
    /// up many independent findings in one repo and asserts the merged
    /// `DoctorReport` looks the same as the multi-walk produced — a
    /// regression guard for the D7 refactor.
    #[test]
    fn single_pass_scan_surfaces_all_categories() {
        let tmp = fresh_repo();
        // Legacy <NN>-<slug> dir under issues/open/.
        put_legacy(
            &tmp,
            "open",
            7,
            "old-style",
            "---\nnumber: 7\nstatus: open\n---\n# E7. Old\n",
        );
        // Flat-layout issue with: broken epic ref + future timestamp +
        // unknown frontmatter key + ## Notes that needs renaming.
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n\
             epic: nonexistent-ghost-fox\ncreated: 2999-01-01\n\
             whimsy: 1\n---\n# T\n\n## Notes\n\nold note\n",
        )
        .unwrap();
        // Symlink + orphan tempfile.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                tmp.path().join("issues/quiet-brave-otter"),
                tmp.path().join("issues/symlinked-thing"),
            )
            .unwrap();
        }
        fs::write(
            tmp.path().join("issues/quiet-brave-otter/.issuectl-tmp-XYZ"),
            "leftover",
        )
        .unwrap();

        let r = scan(tmp.path()).unwrap();

        assert_eq!(r.legacy_dirs.len(), 1, "legacy dir detected");
        assert!(
            r.broken_refs
                .iter()
                .any(|(_, k, t)| k == "epic" && t == "nonexistent-ghost-fox"),
            "broken_refs={:?}",
            r.broken_refs
        );
        assert!(
            r.timestamp_issues.iter().any(|(_, m)| m.contains("future")),
            "timestamp_issues={:?}",
            r.timestamp_issues
        );
        assert!(
            r.unknown_keys.iter().any(|(_, k)| k == "whimsy"),
            "unknown_keys={:?}",
            r.unknown_keys
        );
        assert!(
            r.notes_to_rename.iter().any(|s| s == "quiet-brave-otter"),
            "notes_to_rename={:?}",
            r.notes_to_rename
        );
        assert!(
            r.orphan_tempfiles
                .iter()
                .any(|p| p.to_string_lossy().contains(".issuectl-tmp-XYZ")),
            "orphan_tempfiles={:?}",
            r.orphan_tempfiles
        );
        #[cfg(unix)]
        assert!(
            r.symlinked_dirs.iter().any(|s| s.contains("symlinked-thing")),
            "symlinked_dirs={:?}",
            r.symlinked_dirs
        );
        // Schema violations should ignore the legacy dir.
        assert!(
            !r.schema_violations
                .iter()
                .any(|(loc, _)| loc.contains("7-old-style")),
            "schema_violations should skip legacy dirs: {:?}",
            r.schema_violations
        );
    }
}
