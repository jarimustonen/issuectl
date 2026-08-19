//! `issuectl doctor` — repository health-check and one-shot migration from
//! legacy `<NN>-<slug>/` directory layout to slug-only layout.
//!
//! Read-only by default; `--fix` applies migrations and fixes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::{Captures, Regex};

/// Severity tag for entries in `DoctorFindings::parse_errors`. Replaces
/// the previous substring-matching classifier in `is_hard_parse_error`:
/// classification is set at push-time using the typed parser state
/// (`ParsedItem::has_hard_frontmatter_error`), so re-wording a parser
/// message no longer reclassifies hard-fail as soft-warn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseSeverity {
    /// File unreadable or frontmatter completely unparseable. `--fix`
    /// must refuse — doctor cannot safely rewrite frontmatter whose
    /// shape it could not understand.
    Hard,
    /// Recoverable warning that the migration pass is designed to
    /// heal (legacy numeric refs, etc.). Does not block `--fix`.
    Soft,
}

#[derive(Debug, Clone)]
struct ParseError {
    location: String,
    message: String,
    severity: ParseSeverity,
}

use crate::agents;
use crate::migrate_layout::{
    execute_migrate_layout_plan, plan_migrate_layout, MigrateConflict, MigrateLayoutPlan,
    MigrateMove, PlannedMove,
};
use crate::parser;
use crate::schema;
use crate::slug;
use crate::write;

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
    /// `parser::parse_item_md_text_with_warnings` output. `None` only
    /// when the file could not be read or wasn't present. The parser
    /// exposes the raw mapping plus `fm_missing` / `fm_yaml_error`
    /// flags so this struct doesn't carry duplicate copies.
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
        // `slug:` short-circuits BEFORE `number:` so a modern issue
        // that happens to carry a stray `number:` field (left over
        // from a botched manual edit, a forward-compat field, ...)
        // is not classified legacy and silently queued for NN-rename.
        // `issuectl new` always writes `slug:` for new issues, so
        // the presence of `slug:` is the typed signal that this is
        // a modern issue regardless of any other frontmatter.
        if m.get(serde_yaml::Value::String("slug".into()))
            .and_then(|v| v.as_str())
            .is_some()
        {
            return None;
        }
        if let Some(v) = m.get(serde_yaml::Value::String("number".into())) {
            if let Some(n) = v.as_u64().and_then(|u| u32::try_from(u).ok()) {
                return Some(n);
            }
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
        let Ok(ftype) = entry.file_type() else {
            continue;
        };
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
        let mut parsed: Option<parser::ParsedItem> = None;
        if item_present {
            match fs::read_to_string(&item_path) {
                Ok(t) => {
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
            legacy_number_from_mapping(parsed.as_ref().and_then(|p| p.mapping.as_ref()), &name)
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

/// Stage 1: read-only output of `scan()`. No mutations recorded here.
/// Compare to `ApplyOutcome` (stage 3) which records the result of
/// writes and to `DoctorActions` (stage 2, derived inside `run`) which
/// captures the planned writes. Splitting the three stages makes
/// `fix_applied` reliable (it is computed from `ApplyOutcome` alone)
/// and removes the brittle field-by-field splice list `run` used to
/// maintain when a new applied-action variant was added.
#[derive(Debug, Default)]
struct DoctorFindings {
    legacy_dirs: Vec<LegacyMigration>,
    /// Slug-shaped issues still living under `issues/{open,closed}/<slug>/`
    /// (post-flat-layout legacy). Planned moves to `issues/<slug>/`.
    /// Pending migration plan from `plan_migrate_layout`. Held opaque
    /// so `apply` can hand it back to `execute_migrate_layout_plan`
    /// (which consumes it under `&WriteLock`).
    flat_layout_plan: Option<MigrateLayoutPlan>,
    flat_layout_conflicts: Vec<MigrateConflict>,
    invalid_slugs: Vec<String>,
    duplicate_slugs: Vec<String>,
    missing_item_md: Vec<String>,
    orphan_epic_refs: Vec<(String, String)>,
    /// Per-issue parse warnings (malformed YAML, unreadable file, ...).
    /// Each entry carries an explicit `severity` — Hard entries block
    /// `--fix` (frontmatter unparseable), Soft entries do not (legacy
    /// numeric refs the migration pass is designed to heal).
    parse_errors: Vec<ParseError>,
    /// Per-issue schema violations: (location, message). Populated by
    /// validating each issue's frontmatter against `issues/.schema.yaml`
    /// (or the built-in default if absent).
    schema_violations: Vec<(String, String)>,
    /// Legacy `status` / `type` values that `doctor --fix` will coerce
    /// to a canonical value via the schema's `status_aliases` /
    /// `type_aliases`. `(slug, field, from, to, item_path)`. An issue
    /// flagged here is intentionally NOT also listed under
    /// `schema_violations` for the same field — the coercion is the fix.
    alias_coercions: Vec<(String, String, String, String, PathBuf)>,
    /// True if the schema file was missing at scan time. `--fix` writes
    /// the default schema; without `--fix` this is reported as a hint.
    schema_missing: bool,
    /// True if the schema file is present but failed to parse. Causes
    /// `--fix` to skip per-issue schema validation rather than treating
    /// every issue as broken against an unparseable rule set.
    schema_parse_error: Option<String>,
    /// Slugs the read-only scan classified as safe to migrate from
    /// `## Notes` → `## Comments`. Populated in `scan()`; consumed
    /// (and emptied) by `rename_notes_to_comments()` during `--fix`.
    notes_to_rename: Vec<String>,
    /// Slugs with an ambiguous legacy-section shape — multiple
    /// `## Notes`, or a `## Notes` alongside multiple `## Comments` —
    /// where the merge target is unclear, so doctor flags them for
    /// manual merge and skips. (The unambiguous both-exist case — one
    /// of each — is auto-merged and lives in `notes_to_rename`.)
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
    /// Slugs that list themselves in their own `blocked_by` array.
    /// Reported separately from `blocked_by_cycles` because the fix
    /// is local (drop the self-reference) and the error message can
    /// be sharper than the generic cycle list.
    blocked_by_self: Vec<String>,
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
    /// Issues whose `reviewer:` value is not in the repo's known-user
    /// universe (no other issue lists this name as `reporter`/
    /// `assignee`/`owner`). `(slug, reviewer)`. Warning-only — there
    /// is no separate user catalog, so a fresh hire whose first
    /// touchpoint is a review will trip this until they own/report
    /// something. `--fix` never edits it.
    unknown_reviewers: Vec<(String, String)>,
    /// Orphan `.issuectl-tmp-*` files inside `issues/**` (atomic-write
    /// tempfiles that survived a SIGKILL). `--fix` deletes them.
    orphan_tempfiles: Vec<PathBuf>,
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
    /// True when `.issuectl/AGENTS.md` does not exist. Init is opt-in
    /// so this is informational only — `--fix` does not create the
    /// file (the user runs `issuectl agents init` to opt in). Surfaces
    /// the missing-policy condition that drift detection alone would
    /// otherwise hide.
    agents_md_missing: bool,
    /// Canonical issuectl-tracked files that `git check-ignore` says
    /// would be ignored by `.gitignore`. Asymmetric footgun: the file
    /// exists locally so doctor and the agents skill find it, but
    /// teammates and CI never see it. Informational warning;
    /// `--fix` does not edit `.gitignore`.
    /// (#simply-workable-umbrella)
    gitignored_paths: Vec<String>,
    /// True when `issues/AGENTS.md` exists with pre-v0.5.0 scaffold
    /// content (numbered layout, `open/`/`closed/` subdirs, sequential
    /// numbering section). `--fix` rewrites it with the current
    /// minimal pointer template. Only flagged when legacy markers are
    /// present, so user-customized content is left alone.
    legacy_issues_agents_md: bool,
    /// Large binary files under `issues/<slug>/` (including its
    /// `attachments/` / `fixtures/` subdirs) exceeding
    /// `LARGE_BINARY_BYTES`. Warning-only — committing big binaries
    /// bloats git history; suggest external storage or `.gitignore`.
    /// `(slug, repo_relative_path, bytes)`.
    large_binaries: Vec<(String, String, u64)>,
    /// Raster images under an issue dir that are not AVIF. The issue
    /// convention prefers AVIF; warning-only nudge to convert.
    /// `(slug, repo_relative_path)`.
    non_avif_images: Vec<(String, String)>,
    /// Relative-path references in an issue body that do not resolve to
    /// a file inside that issue's directory (moved/renamed attachment or
    /// a typo). Warning-only. `(slug, reference)`.
    broken_attachment_refs: Vec<(String, String)>,
    /// Issues still carrying the retired `deferred` lifecycle label.
    /// Warning-only and auto-fixable. The intake `deferred` status remains
    /// valid; this check is deliberately scoped to the label.
    /// `(slug, item_path)`.
    deferred_labels: Vec<(String, PathBuf)>,
    /// Deferred labels whose issue still needs the conflict-aware legacy
    /// intake migration. Doctor reports these but leaves them untouched so
    /// it cannot erase the state that `intake migrate` must interpret.
    /// `(slug, reason)`.
    deferred_labels_require_intake_migrate: Vec<(String, String)>,
}

/// Stage 2: planned writes derived from `DoctorFindings`. Built by
/// `DoctorActions::from_findings` inside `run` when `--fix` is set.
/// `apply` consumes this and never reads back from `DoctorFindings`,
/// so a new applied-action variant is added by extending this struct
/// + `ApplyOutcome` — no need to update a manual splice list in `run`.
#[derive(Debug, Default)]
struct DoctorActions {
    legacy_dirs: Vec<LegacyMigration>,
    flat_layout_plan: Option<MigrateLayoutPlan>,
    notes_to_rename: Vec<String>,
    /// Slugs with an ambiguous legacy-section shape (multiple
    /// `## Notes`, or a `## Notes` alongside multiple `## Comments`)
    /// that cannot be auto-merged. The apply pipeline records them in
    /// `outcome.notes_conflicts_at_apply` so the human/JSON output
    /// surfaces the manual-merge need. Carried as actions (not implicit
    /// from findings) so the post-flat-layout rescan can repopulate
    /// this alongside `notes_to_rename`. See issue: @doctor-fix-noop.
    notes_conflicts: Vec<String>,
    orphan_tempfiles: Vec<PathBuf>,
    closed_with_active_status: Vec<(String, String, PathBuf)>,
    open_with_closing_status: Vec<(String, String, PathBuf)>,
    /// Legacy status/type values to coerce via the schema alias tables.
    /// `(slug, field, from, to, item_path)`.
    alias_coercions: Vec<(String, String, String, String, PathBuf)>,
    /// True when scan flagged AGENTS.md drift AND the file is present
    /// AND drift is regeneratable (not malformed / not check-skipped).
    regenerate_agents_md: bool,
    /// True when scan flagged a legacy `issues/AGENTS.md` (pre-v0.5.0
    /// scaffold). `--fix` overwrites with the current template.
    rewrite_issues_agents_md: bool,
    /// Retired `deferred` labels to remove. `(slug, item_path)`.
    deferred_labels: Vec<(String, PathBuf)>,
    /// Critical findings that block `--fix`. Computed via
    /// `apply_blockers` — the layout-fatal subset of
    /// `critical_blockers`. Schema-shape findings (schema violations,
    /// broken refs, dependency cycles, status/timestamp consistency)
    /// drive exit-1 but are intentionally NOT in this list: layout
    /// migration is a directory rename and is independent of
    /// frontmatter content. See `BlockerScope` for the rationale.
    preflight_blockers: Vec<String>,
}

impl DoctorActions {
    /// Move action triggers out of findings into a derived plan. After
    /// this call, `findings` no longer carries the to-do data — it is
    /// fresh-scanned again post-apply for the user-facing render, so
    /// nothing depends on the cleared fields.
    fn from_findings(findings: &mut DoctorFindings) -> Self {
        let preflight_blockers = apply_blockers(findings);
        DoctorActions {
            legacy_dirs: std::mem::take(&mut findings.legacy_dirs),
            flat_layout_plan: findings.flat_layout_plan.take(),
            notes_to_rename: std::mem::take(&mut findings.notes_to_rename),
            // Taken (not cloned): `run()` unconditionally re-scans
            // after `apply()` returns, so the rendered findings come
            // from a fresh scan and don't depend on this field
            // surviving the move. Issue: @doctor-fix-noop.
            notes_conflicts: std::mem::take(&mut findings.notes_conflicts),
            orphan_tempfiles: std::mem::take(&mut findings.orphan_tempfiles),
            closed_with_active_status: std::mem::take(&mut findings.closed_with_active_status),
            open_with_closing_status: std::mem::take(&mut findings.open_with_closing_status),
            alias_coercions: std::mem::take(&mut findings.alias_coercions),
            regenerate_agents_md: findings.agents_md_drift
                && findings.agents_md_malformed.is_none()
                && findings.agents_md_check_skipped.is_none(),
            rewrite_issues_agents_md: findings.legacy_issues_agents_md,
            deferred_labels: findings
                .deferred_labels
                .iter()
                .filter(|(slug, _)| {
                    !findings
                        .deferred_labels_require_intake_migrate
                        .iter()
                        .any(|(pending, _)| pending == slug)
                })
                .cloned()
                .collect(),
            preflight_blockers,
        }
    }
}

/// Discriminator on `ApplyOutcome` that disambiguates the two
/// blocker-bail paths. `Ok` means apply ran to completion; `Preflight`
/// means we refused to mutate (no writes); `PostApply` means the
/// post-flat-layout safety re-check fired AFTER phase-5 writes
/// already landed (partial progress, forward-only). `--json`
/// consumers should branch on this rather than inferring from the
/// presence of `blockers` + `fix_applied`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum StopPhase {
    #[default]
    Ok,
    Preflight,
    PostApply,
}

impl StopPhase {
    fn as_str(self) -> &'static str {
        match self {
            StopPhase::Ok => "ok",
            StopPhase::Preflight => "preflight",
            StopPhase::PostApply => "post_apply",
        }
    }
}

/// Stage 3: result of running `apply`. The single source of truth for
/// "what was actually written". `fix_applied` is computed from the
/// outcome alone — no `report.fix_applied = true` early-return path
/// can lie about it.
#[derive(Debug, Default)]
struct ApplyOutcome {
    /// Critical blockers found during the apply pass. Populated in
    /// two places (use `stop_phase` to tell them apart):
    ///   1. **Preflight** (`stop_phase == Preflight`) — set from
    ///      `actions.preflight_blockers` before any write; coexists
    ///      with all-zero/empty fields and `fix_applied() == false`.
    ///   2. **Post-flat-layout safety re-check**
    ///      (`stop_phase == PostApply`) — set after phase 5 when the
    ///      post-migration scan surfaces a condition the pre-migration
    ///      scan could not see (e.g. `## Notes` / `## Comments`
    ///      ambiguity in a freshly-lifted dir). In this case
    ///      `flat_layout_migrated` (and possibly other already-
    ///      completed phase fields) is non-empty and
    ///      `fix_applied() == true` — `--fix` is forward-progress only
    ///      and does not roll back. JSON consumers should branch on
    ///      `stop_phase` rather than inferring from `blockers` +
    ///      `fix_applied`.
    blockers: Vec<String>,
    /// See `StopPhase`. Default `Ok`.
    stop_phase: StopPhase,
    legacy_dirs_migrated: Vec<LegacyMigration>,
    flat_layout_migrated: Vec<MigrateMove>,
    notes_renamed: Vec<String>,
    orphan_tempfiles_removed: Vec<PathBuf>,
    status_reconciled: Vec<String>,
    /// Legacy status/type values rewritten via the schema alias tables.
    /// `(slug, field, from, to)`.
    alias_coercions_applied: Vec<(String, String, String, String)>,
    files_rewritten: usize,
    agents_md_regenerated: bool,
    issues_agents_md_rewritten: bool,
    /// Slugs from which the retired `deferred` label was removed.
    deferred_labels_removed: Vec<String>,
    schema_bootstrapped: bool,
    /// Slugs whose `## Notes` rename was planned by scan but skipped
    /// at apply time because a concurrent edit introduced a
    /// `## Comments` heading between scan and apply (TOCTOU; rare
    /// since the WriteLock is held). Recorded explicitly because the
    /// post-apply rescan only re-classifies if the conflict is still
    /// visible — and because users running `--json --fix` need a
    /// signal that some planned work didn't run.
    notes_conflicts_at_apply: Vec<String>,
    /// Failure cause from a mid-pipeline `--fix` step that aborted
    /// before completion. Currently populated only by phase 5
    /// (flat-layout migration) when `execute_migrate_layout_plan`
    /// returns mid-loop with a partial `flat_layout_migrated`
    /// already on disk. Set instead of returning `Err` from `apply`
    /// so `--json --fix` callers receive a structured envelope
    /// (with the partial `flat_layout_migrated` intact) rather than
    /// an anyhow-formatted stderr blob that hides what landed.
    apply_error: Option<String>,
}

impl ApplyOutcome {
    /// True iff at least one filesystem write actually happened.
    /// Adding a new applied-action variant means adding one OR-clause
    /// here — the field list is in one place, not spliced across `run`
    /// and three other functions.
    fn fix_applied(&self) -> bool {
        !self.legacy_dirs_migrated.is_empty()
            || !self.flat_layout_migrated.is_empty()
            || !self.notes_renamed.is_empty()
            || !self.orphan_tempfiles_removed.is_empty()
            || !self.status_reconciled.is_empty()
            || !self.alias_coercions_applied.is_empty()
            || self.files_rewritten > 0
            || self.agents_md_regenerated
            || self.issues_agents_md_rewritten
            || !self.deferred_labels_removed.is_empty()
            || self.schema_bootstrapped
    }

    /// Set `blockers` and `stop_phase` together. Forces callers to
    /// supply the phase at the assignment site, preventing a future
    /// `outcome.blockers = ...; return Ok(outcome);` site from silently
    /// emitting `stop_phase: "ok"` because the field defaults to it.
    /// Debug-asserts the documented invariants:
    ///   - phase ∈ {Preflight, PostApply}
    ///   - blockers != []
    ///   - Preflight ⇒ no prior writes BEYOND schema bootstrap. The
    ///     bootstrap is intentionally hoisted above preflight (issue:
    ///     `@unreasonably-attractive-star`) and is the only write the
    ///     pipeline allows before the preflight gate. Any other field
    ///     being non-default at this point indicates a phase ran out
    ///     of order.
    ///   - PostApply  ⇒ at least one phase BEYOND schema bootstrap
    ///     ran (the post-flat-layout re-check fires only after phase
    ///     5 writes land).
    fn stop_with_blockers(&mut self, phase: StopPhase, blockers: Vec<String>) {
        debug_assert!(
            matches!(phase, StopPhase::Preflight | StopPhase::PostApply),
            "stop_with_blockers requires Preflight or PostApply, got {phase:?}"
        );
        debug_assert!(
            !blockers.is_empty(),
            "stop_with_blockers requires at least one blocker"
        );
        match phase {
            StopPhase::Preflight => debug_assert!(
                !self.fix_applied_beyond_schema_bootstrap(),
                "Preflight stop must precede any write beyond schema bootstrap"
            ),
            StopPhase::PostApply => debug_assert!(
                self.fix_applied_beyond_schema_bootstrap(),
                "PostApply stop must follow at least one phase beyond schema bootstrap"
            ),
            StopPhase::Ok => unreachable!(),
        }
        self.blockers = blockers;
        self.stop_phase = phase;
    }

    /// True iff a phase beyond the unconditional schema bootstrap
    /// ran. Used by `stop_with_blockers` to encode the new pipeline
    /// invariant: schema bootstrap is the only write allowed before
    /// preflight refusal; everything else must wait for a clean
    /// preflight pass.
    fn fix_applied_beyond_schema_bootstrap(&self) -> bool {
        !self.legacy_dirs_migrated.is_empty()
            || !self.flat_layout_migrated.is_empty()
            || !self.notes_renamed.is_empty()
            || !self.orphan_tempfiles_removed.is_empty()
            || !self.status_reconciled.is_empty()
            || !self.alias_coercions_applied.is_empty()
            || self.files_rewritten > 0
            || self.agents_md_regenerated
            || self.issues_agents_md_rewritten
            || !self.deferred_labels_removed.is_empty()
    }
}

/// Project an absolute path under the repo root to a repo-relative
/// `String` (UTF-8 lossy fallback). JSON consumers (and the text
/// renderer) shouldn't see absolute filesystem paths leaking into
/// CI logs.
mod apply;
mod checks;
mod core;
#[cfg(test)]
mod doctor_tests;
mod render;

use apply::*;
use checks::*;
use core::*;
pub use core::{run, run_via};
use render::*;
