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
    }
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

pub fn run(repo_root: &Path, fix: bool, json: bool, verbose: bool) -> Result<()> {
    let mut findings = scan(repo_root)?;
    let outcome: Option<ApplyOutcome> = if fix {
        // D2: hold the repo write lock through the apply pass so doctor
        // doesn't race CLI/server mutations. Re-scan under the lock to
        // ensure the plan reflects the locked-state filesystem.
        let lock = crate::mutate::WriteLock::acquire(repo_root)?;
        findings = scan(repo_root)?;
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(repo_root, actions, &lock)?;
        // ALWAYS re-scan after apply, regardless of fix_applied. The
        // call to `DoctorActions::from_findings` drained findings via
        // `mem::take` (legacy_dirs / flat_layout_plan / notes_to_rename
        // / orphan_tempfiles / status reconciliation lists) — without
        // this rescan, render_text and render_json would receive a
        // gutted `findings` on preflight-blocked or no-write runs, and
        // the user would see "doctor: cannot apply --fix" with NONE of
        // the actual to-do lists below. Caches are hot post-apply, so
        // the I/O is negligible.
        findings = scan(repo_root)?;
        Some(outcome)
    } else {
        None
    };

    let exit_decision = classify_exit(&findings, outcome.as_ref(), fix);
    if json {
        // The envelope-on-stderr contract is scoped to `--fix --json`
        // per the success criteria: read-only `--json doctor` keeps
        // the historical behaviour of emitting the full result on
        // stdout regardless of exit code, so existing scripts doing
        // `issuectl --json doctor | jq …` on an unhealthy repo still
        // work. Issue: @doctor-fix-noop.
        if fix && exit_decision.code != 0 {
            let details = render_json(&findings, outcome.as_ref(), fix, repo_root);
            let envelope = serde_json::json!({
                "error": {
                    "code": exit_decision.error_code,
                    "message": exit_decision.message,
                    "details": details,
                }
            });
            eprintln!("{}", serde_json::to_string_pretty(&envelope)?);
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&render_json(
                    &findings,
                    outcome.as_ref(),
                    fix,
                    repo_root
                ))?
            );
        }
    } else {
        render_text(&findings, outcome.as_ref(), fix, verbose);
    }
    if exit_decision.code != 0 {
        std::process::exit(exit_decision.code);
    }
    Ok(())
}

/// Decision returned by `classify_exit`. `code == 0` means a clean
/// run. Non-zero `code` carries a stable `error_code` + human
/// `message` for the `--json` error envelope (issue: @doctor-fix-noop).
#[derive(Debug, Clone)]
struct ExitDecision {
    code: i32,
    error_code: &'static str,
    message: String,
}

/// Compute the exit code + envelope code/message from a post-apply
/// findings scan and optional outcome. Pure: extracted from `run` so
/// the mapping is unit-testable (issue: @doctor-fix-noop, success
/// criterion D).
///
/// Mapping:
///   - `outcome.apply_error.is_some()` → `doctor-apply-error` (exit 1)
///   - `stop_phase == Preflight`       → `doctor-blocked`     (exit 1)
///   - `stop_phase == PostApply`       → `doctor-partial`     (exit 1)
///   - critical findings remain        → `doctor-partial`     (exit 1)
///   - `notes_conflicts_at_apply` non-empty after a clean `Ok` apply
///     → `doctor-partial` (exit 1; the apply ran but some manual-only
///     work is left for the user)
///   - else → exit 0
fn classify_exit(
    findings: &DoctorFindings,
    outcome: Option<&ApplyOutcome>,
    fix: bool,
) -> ExitDecision {
    let ok = ExitDecision {
        code: 0,
        error_code: "",
        message: String::new(),
    };
    let crit = critical_blockers(findings);
    if let Some(oc) = outcome {
        if let Some(err) = &oc.apply_error {
            return ExitDecision {
                code: 1,
                error_code: "doctor-apply-error",
                message: format!("doctor --fix aborted mid-pipeline: {err}"),
            };
        }
        match oc.stop_phase {
            StopPhase::Preflight => {
                return ExitDecision {
                    code: 1,
                    error_code: "doctor-blocked",
                    message: format!(
                        "doctor --fix refused: {} preflight blocker(s)",
                        oc.blockers.len()
                    ),
                };
            }
            StopPhase::PostApply => {
                return ExitDecision {
                    code: 1,
                    error_code: "doctor-partial",
                    message: format!(
                        "doctor --fix partial: {} post-apply blocker(s) remain after partial writes",
                        oc.blockers.len()
                    ),
                };
            }
            StopPhase::Ok => {
                // Manual-merge notes/comments findings produce a
                // specific message — checked BEFORE the generic
                // `crit` branch because notes_conflicts persists in
                // `findings.notes_conflicts` (and therefore in
                // `crit` via `critical_blockers`) after the apply
                // pass that recorded them; the generic branch would
                // otherwise mask the specific guidance the user
                // needs.
                if !oc.notes_conflicts_at_apply.is_empty() {
                    return ExitDecision {
                        code: 1,
                        error_code: "doctor-partial",
                        message: format!(
                            "doctor --fix partial: {} issue(s) need manual `## Notes`/`## Comments` merge",
                            oc.notes_conflicts_at_apply.len()
                        ),
                    };
                }
                if !crit.is_empty() {
                    return ExitDecision {
                        code: 1,
                        error_code: "doctor-partial",
                        message: format!(
                            "doctor --fix partial: {} unfixable finding(s) remain",
                            crit.len()
                        ),
                    };
                }
                return ok;
            }
        }
    }
    // Read-only path: any critical finding drives exit 1 too. No
    // envelope code distinction for `--fix=false` (no `apply_outcome`
    // to attach), but if `--json doctor` is run on an unhealthy repo
    // we still emit the structured envelope so scripts parse one
    // shape.
    if !crit.is_empty() {
        return ExitDecision {
            code: 1,
            error_code: if fix {
                "doctor-partial"
            } else {
                "doctor-unhealthy"
            },
            message: format!("doctor: {} unfixable finding(s)", crit.len()),
        };
    }
    ok
}

/// Scope for `blockers_for` — disambiguates "is the repo unhealthy
/// enough to exit 1?" (the broad set) from "is the repo unsafe to
/// run the apply pipeline against?" (the narrow, layout-fatal
/// subset). Schema-shape findings (schema violations, non-legacy
/// broken refs, dependency cycles, status/timestamp inconsistencies)
/// drive the exit code but are NOT layout-fatal: the safest, most
/// mechanical phase (`--fix`'s flat-layout migration) just renames
/// directories and is independent of frontmatter contents. Treating
/// schema findings as preflight blockers forced users with hundreds
/// of pre-existing schema violations to hand-fix every one of them
/// before doctor would lift a finger — the largest single adoption
/// blocker reported in 3DBear 0.5.1 feedback (@intensely-ill-garden,
/// @staggeringly-important-zoo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockerScope {
    /// Drives the run-time exit code. The full set of "user must
    /// intervene" findings.
    ExitCode,
    /// Drives the `--fix` preflight refusal AND the post-flat-layout
    /// safety re-check. Narrower than `ExitCode`: schema-shape
    /// findings are filtered out so `--fix` can still migrate the
    /// layout while schema violations are pending. The user fixes
    /// the schema violations afterward against the post-migration
    /// state — the same scan output ranks them under exit-1 so they
    /// remain visible.
    ApplyPreflight,
}

/// Single-source-of-truth predicate for "the repo is in a state the
/// user must intervene on". Drives both the exit code (`run` checks
/// `!critical_blockers(&findings).is_empty()`) AND the `--fix`
/// preflight check via the narrower `apply_blockers` view.
/// Previously these two callers held drifting copies of the rule
/// set, which produced two failure modes the spin-off flagged: (a)
/// `--fix` would mutate a repo `has_critical_findings` rated
/// critical (partial mutations on a critically-unhealthy repo), and
/// (b) parse-error classification used a fragile substring matcher
/// whose Hard/Soft split was easy to flip by re-wording the parser's
/// message.
fn critical_blockers(findings: &DoctorFindings) -> Vec<String> {
    blockers_for(findings, BlockerScope::ExitCode)
}

/// Layout-fatal subset of `critical_blockers`. Used by the `--fix`
/// preflight check and the post-flat-layout safety re-check.
fn apply_blockers(findings: &DoctorFindings) -> Vec<String> {
    blockers_for(findings, BlockerScope::ApplyPreflight)
}

fn blockers_for(findings: &DoctorFindings, scope: BlockerScope) -> Vec<String> {
    let layout_only = matches!(scope, BlockerScope::ApplyPreflight);
    let mut blockers: Vec<String> = Vec::new();

    if !findings.flat_layout_conflicts.is_empty() {
        let detail = findings
            .flat_layout_conflicts
            .iter()
            .map(|c| format!("    {}: {}", c.slug, c.detail))
            .collect::<Vec<_>>()
            .join("\n");
        blockers.push(format!("flat-layout migration conflicts:\n{detail}"));
    }
    if !findings.duplicate_slugs.is_empty() {
        blockers.push(format!("duplicate slugs: {:?}", findings.duplicate_slugs));
    }
    if !findings.both_open_and_closed.is_empty() {
        blockers.push(format!(
            "slugs present in BOTH issues/open/ and issues/closed/: {:?}",
            findings.both_open_and_closed
        ));
    }
    if !findings.conflict_markers.is_empty() {
        blockers.push(format!(
            "git merge-conflict markers in: {:?}",
            findings.conflict_markers
        ));
    }
    let hard_parse: Vec<&ParseError> = findings
        .parse_errors
        .iter()
        .filter(|e| e.severity == ParseSeverity::Hard)
        .collect();
    if !hard_parse.is_empty() {
        let detail = hard_parse
            .iter()
            .map(|e| format!("    {}: {}", e.location, e.message))
            .collect::<Vec<_>>()
            .join("\n");
        blockers.push(format!(
            "unparseable issue file(s) ({}):\n{detail}",
            hard_parse.len()
        ));
    }
    if let Some(err) = &findings.schema_parse_error {
        blockers.push(format!("schema file parse error: {err}"));
    }
    if !layout_only && !findings.schema_violations.is_empty() {
        blockers.push(format!(
            "schema violations: {} issue(s) fail validation",
            findings.schema_violations.len()
        ));
    }
    if !findings.invalid_slugs.is_empty() {
        blockers.push(format!("invalid slugs: {:?}", findings.invalid_slugs));
    }
    if !findings.missing_item_md.is_empty() {
        blockers.push(format!(
            "directories missing item.md: {:?}",
            findings.missing_item_md
        ));
    }
    // Exclude legacy numeric refs from the critical set: they are
    // exactly what `--fix`'s legacy migration translates (number →
    // slug via `rewrite_item_frontmatter`). Treating them as critical
    // would refuse the very migration designed to heal them, so a
    // partially-flat-layout repo with a few stale `epic: 7` refs
    // could not progress. The `(legacy numeric ref)` suffix is set
    // by `populate_extended_validation::check_ref` and is the typed
    // signal — not a substring matcher on user content.
    let non_legacy_broken: Vec<&(String, String, String)> = findings
        .broken_refs
        .iter()
        .filter(|(_, _, target)| !target.ends_with("(legacy numeric ref)"))
        .collect();
    if !layout_only && !non_legacy_broken.is_empty() {
        blockers.push(format!(
            "broken cross-references: {} entry/entries",
            non_legacy_broken.len()
        ));
    }
    if !layout_only && !findings.blocked_by_cycles.is_empty() {
        blockers.push(format!(
            "dependency cycles via blocked_by: {} cycle(s)",
            findings.blocked_by_cycles.len()
        ));
    }
    if !layout_only && !findings.blocked_by_self.is_empty() {
        blockers.push(format!(
            "self-dependencies in blocked_by: {:?}",
            findings.blocked_by_self
        ));
    }
    if !layout_only && !findings.status_consistency.is_empty() {
        blockers.push(format!(
            "status/closed-date inconsistencies: {} entry/entries",
            findings.status_consistency.len()
        ));
    }
    if !layout_only && !findings.timestamp_issues.is_empty() {
        blockers.push(format!(
            "timestamp sanity issues: {} entry/entries",
            findings.timestamp_issues.len()
        ));
    }
    if !findings.symlinked_dirs.is_empty() {
        // Symlinks under `issues/` could redirect a rewrite outside
        // the repo (a `--fix` body rewrite could land on arbitrary
        // disk). Kept as a preflight blocker for safety.
        blockers.push(format!(
            "symlinked issue directories: {:?}",
            findings.symlinked_dirs
        ));
    }
    // `notes_conflicts`, `agents_md_malformed`, `agents_md_check_skipped`
    // are localised, per-file manual-merge findings. They drive exit-1
    // (so the user keeps seeing them) but MUST NOT be preflight
    // blockers — they used to silently swallow orthogonal auto-fixable
    // work (alias coercion, AGENTS.md schema-block regen, NN-rename)
    // by aborting the whole apply pass. `rename_notes_to_comments`
    // already records skipped slugs via `outcome.notes_conflicts_at_apply`,
    // and `regenerate_agents_md` is already gated on these AGENTS.md
    // flags in `DoctorActions::from_findings`. See issue: @doctor-fix-noop.
    if !layout_only {
        if !findings.notes_conflicts.is_empty() {
            blockers.push(format!(
                "## Notes / ## Comments conflicts (manual merge): {:?}",
                findings.notes_conflicts
            ));
        }
        if let Some(reason) = &findings.agents_md_malformed {
            blockers.push(format!("AGENTS.md is malformed: {reason}"));
        }
        if let Some(err) = &findings.agents_md_check_skipped {
            blockers.push(format!("AGENTS.md drift check skipped: {err}"));
        }
    }
    blockers
}

fn scan(repo_root: &Path) -> Result<DoctorFindings> {
    let mut report = DoctorFindings::default();
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
        // Coercion detection runs first so `populate_schema_violations`
        // can suppress the enum violation for any value the coercion
        // will rewrite (otherwise the user sees the same value flagged
        // both as a violation and as a pending fix).
        populate_alias_coercions(&scan, s, &mut report);
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
                    report.parse_errors.push(ParseError {
                        location: crate::transitions::RULES_RELATIVE_PATH.to_string(),
                        message: format!("{e:#}"),
                        severity: ParseSeverity::Hard,
                    });
                    None
                } else {
                    Some(r)
                }
            } else {
                Some(r)
            }
        }
        Err(e) => {
            report.parse_errors.push(ParseError {
                location: crate::transitions::RULES_RELATIVE_PATH.to_string(),
                message: format!("{e:#}"),
                severity: ParseSeverity::Hard,
            });
            None
        }
    };
    if rules.is_some() || schema_value.is_some() {
        populate_transition_warnings(
            &scan,
            rules.as_deref(),
            schema_value.as_deref(),
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
    if !agents_path.exists() {
        report.agents_md_missing = true;
    }
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

    // Legacy `issues/AGENTS.md` scaffold (pre-v0.5.0): the old template
    // documented numbered `<NN>-<slug>/` layout, `open/` / `closed/`
    // subdirs, and sequential numbering — none of which apply now. Flag
    // only when known legacy markers appear so customized files survive.
    let issues_agents_path = repo_root.join("issues").join("AGENTS.md");
    if issues_agents_path.is_file() {
        if let Ok(text) = fs::read_to_string(&issues_agents_path) {
            if text != crate::skill::ISSUES_AGENTS_TEMPLATE && is_legacy_issues_agents(&text) {
                report.legacy_issues_agents_md = true;
            }
        }
    }

    report.gitignored_paths = detect_gitignored_canonical_paths(repo_root);

    let plan = plan_migrate_layout(repo_root)?;
    report.flat_layout_conflicts = plan.conflicts().to_vec();
    report.flat_layout_plan = Some(plan);

    // Round-2 finding O6: read-only `doctor` must surface pending
    // Notes migrations and conflicts so users see the work even
    // before running `--fix`.
    populate_notes_migration(&scan, &mut report);

    populate_extended_validation(&scan, schema_value.as_deref(), &mut report);

    populate_attachment_health(&scan, repo_root, &mut report);

    Ok(report)
}

/// Slug uniqueness, legacy-migration plan, missing-item-md, parse
/// warnings, invalid slug detection. Mirrors the original main scan
/// loop but consumes `ScanResult` instead of re-reading from disk.
fn populate_slug_and_legacy(scan: &ScanResult, repo_root: &Path, report: &mut DoctorFindings) {
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
        // For LEGACY directories, only HARD errors are surfaced —
        // SOFT warnings (legacy numeric refs etc.) are noise on
        // dirs the migration pass rewrites wholesale. HARD errors
        // (frontmatter unparseable, file unreadable) MUST surface
        // even for legacy issues: `--fix`'s `rewrite_item_frontmatter`
        // calls `write::read_item`, which would panic mid-apply on
        // an unparseable file. Letting them flow through into
        // `critical_blockers` makes preflight refuse cleanly.
        if let Some(parsed) = &s.parsed {
            let severity = if parsed.has_hard_frontmatter_error() {
                ParseSeverity::Hard
            } else {
                ParseSeverity::Soft
            };
            if s.legacy_number.is_some() && severity == ParseSeverity::Soft {
                continue;
            }
            for w in &parsed.warnings {
                report.parse_errors.push(ParseError {
                    location: location.clone(),
                    message: w.clone(),
                    severity,
                });
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
fn populate_orphan_epic_refs(scan: &ScanResult, report: &mut DoctorFindings) {
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

/// Attachment / fixture health: large binaries, non-AVIF images, and
/// relative body references that no longer resolve. All warning-only —
/// these never enter `blockers_for`, so they cannot block `--fix` or
/// flip the exit code. Walks the whole issue directory tree (item.md and
/// atomic-write tempfiles excluded, symlinks not followed) — that
/// naturally covers `attachments/` and `fixtures/` as well as any other
/// files an issue carries.
fn populate_attachment_health(scan: &ScanResult, repo_root: &Path, report: &mut DoctorFindings) {
    for s in &scan.issues {
        let mut files = Vec::new();
        collect_issue_files(&s.dir_path, &mut files);
        for path in &files {
            // The issue's own item.md is text we already lint elsewhere.
            if path == &s.item_path {
                continue;
            }
            if let Ok(meta) = fs::metadata(path) {
                if meta.len() > LARGE_BINARY_BYTES {
                    report.large_binaries.push((
                        s.dir_name.clone(),
                        rel(repo_root, path),
                        meta.len(),
                    ));
                }
            }
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if NON_AVIF_IMAGE_EXTS
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(ext))
                {
                    report
                        .non_avif_images
                        .push((s.dir_name.clone(), rel(repo_root, path)));
                }
            }
        }

        // Relative body references pointing inside the issue dir that no
        // longer resolve. Scan only the body — a YAML frontmatter value
        // can legitimately contain `[text](paren)` syntax, which would
        // otherwise register as a phantom broken reference. A target
        // carrying a GitHub-style `#L<n>` line anchor that also exists
        // at the repo root is a cross-file code permalink, not an
        // attachment — skip those. Crucially the skip is gated on the
        // anchor shape so a bare `![logo](README.md)` referencing a
        // missing sibling is NOT silently masked by an unrelated
        // `README.md` at the repo root.
        if let Some(text) = &s.text {
            let body = crate::item_text::split(text).body;
            for r in crate::refs::extract_relative_body_refs(body) {
                if s.dir_path.join(&r.path).exists() {
                    continue;
                }
                if r.has_line_anchor && repo_root.join(&r.path).exists() {
                    continue;
                }
                report
                    .broken_attachment_refs
                    .push((s.dir_name.clone(), r.path));
            }
        }
    }
    report.large_binaries.sort();
    report.non_avif_images.sort();
    report.broken_attachment_refs.sort();
}

/// Recursively collect regular files under `dir`, skipping symlinks and
/// atomic-write tempfiles. Used by `populate_attachment_health`.
fn collect_issue_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(ftype) = entry.file_type() else {
            continue;
        };
        if ftype.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ftype.is_dir() {
            collect_issue_files(&path, out);
        } else if ftype.is_file() {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(".issuectl-tmp-"))
            {
                continue;
            }
            out.push(path);
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
    schema: Option<&schema::Schema>,
    report: &mut DoctorFindings,
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

    // Schema-known field names for unknown-key flagging. Use the
    // pre-loaded schema if available, otherwise fall back to the
    // built-in defaults so the universe of known keys is never empty.
    let owned_default;
    let known_schema = match schema {
        Some(s) => s,
        None => {
            owned_default = schema::default_schema();
            &owned_default
        }
    };
    let mut known: BTreeSet<String> = known_schema.fields.keys().cloned().collect();
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
        // `lane_seq` is a typed field lifted by the parser but, like
        // `commits`, intentionally NOT declared in the schema (the v1
        // string validator would reject the YAML integer). Recognise it
        // here so doctor doesn't flag it as an unknown key.
        "lane_seq",
        "slug",
        "number",
        "blocked_by",
        "reviewer",
        "review_status",
    ] {
        known.insert(k.to_string());
    }

    // Universe of "known users" for the reviewer-validation check. We
    // accept any name that appears as `reporter:`, `assignee:`, or
    // `owner:` on at least one issue in the repo — there is no
    // separate user catalog, so reusing the values already in the
    // graph is the lightest-weight signal. Empty strings are stripped
    // so a stray `reviewer: ""` (which the typed parser already
    // forbids via the trim check, but custom-field writes could
    // sneak through) doesn't validate against another empty entry.
    let mut known_users: BTreeSet<String> = BTreeSet::new();
    for hits in by_slug.values() {
        let primary = hits
            .iter()
            .find(|h| h.folder == "flat")
            .copied()
            .unwrap_or(hits[0]);
        let Some(fm) = primary.parsed.as_ref().and_then(|p| p.mapping.as_ref()) else {
            continue;
        };
        for key in ["reporter", "assignee", "owner"] {
            if let Some(v) = fm
                .get(serde_yaml::Value::String(key.into()))
                .and_then(|v| v.as_str())
            {
                let v = v.trim();
                if !v.is_empty() {
                    known_users.insert(v.to_string());
                }
            }
        }
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

        let Some(fm) = primary.parsed.as_ref().and_then(|p| p.mapping.as_ref()) else {
            continue;
        };

        // Skip lints that flag findings critical_blockers treats as
        // hard refusals when the issue is queued for the NN-rename
        // pipeline. `--fix` migrates these wholesale (frontmatter
        // rewritten, refs translated, file renamed), so emitting
        // hard findings on them would refuse the very fix designed
        // to heal them. The typed signal is `legacy_number.is_some()`
        // — applies whether the dir lives at `issues/{open,closed}/`
        // (pre-migration) or at the flat root (post flat-layout
        // migration but before NN-rename). Mirrors the skip in
        // `populate_schema_violations`.
        let primary_is_legacy = primary.legacy_number.is_some();
        if primary_is_legacy {
            // Still run the per-hit status/folder reconciliation pass
            // below — that is exactly the legacy state `--fix` heals,
            // and emitting `closed_with_active_status` /
            // `open_with_closing_status` here is what triggers the
            // reconciliation.
            for hit in hits {
                if hit.folder != "open" && hit.folder != "closed" {
                    continue;
                }
                let Some(fm) = hit.parsed.as_ref().and_then(|p| p.mapping.as_ref()) else {
                    continue;
                };
                let Some(hit_status) = fm
                    .get(serde_yaml::Value::String("status".into()))
                    .and_then(|v| v.as_str())
                else {
                    continue;
                };
                match hit.folder.as_str() {
                    "closed" if crate::issue_fields::ACTIVE_STATUSES.contains(&hit_status) => {
                        report.closed_with_active_status.push((
                            slug.clone(),
                            hit_status.to_string(),
                            hit.item_path.clone(),
                        ));
                    }
                    "open" if crate::issue_fields::is_closing_status(hit_status) => {
                        report.open_with_closing_status.push((
                            slug.clone(),
                            hit_status.to_string(),
                            hit.item_path.clone(),
                        ));
                    }
                    _ => {}
                }
            }
            continue;
        }

        // Unknown-key flagging.
        for (k, _) in fm.iter() {
            if let serde_yaml::Value::String(name) = k {
                if !known.contains(name) {
                    report.unknown_keys.push((slug.clone(), name.clone()));
                }
            }
        }

        // Reviewer must be a known user. The check fires only when
        // `reviewer:` is present and the value is a non-empty string;
        // shape errors are surfaced by schema validation and the typed
        // parser, not here.
        if let Some(reviewer) = fm
            .get(serde_yaml::Value::String("reviewer".into()))
            .and_then(|v| v.as_str())
        {
            let reviewer = reviewer.trim();
            if !reviewer.is_empty() && !known_users.contains(reviewer) {
                report
                    .unknown_reviewers
                    .push((slug.clone(), reviewer.to_string()));
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
        let closed_by = fm
            .get(serde_yaml::Value::String("closed_by".into()))
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

        // Status/closed consistency. Schema-aware: a project that
        // declares `archived` (or similar) as a closing status via
        // `status_classes:` in `.schema.yaml` flags `archived without
        // closed:` here just like a built-in `done`. Both branches go
        // through the same `status_class` lookup so the layering
        // (schema → built-in → default-active) is applied
        // consistently. An unknown status defaults to `Active` in
        // `status_class`; we suppress the active-side check for
        // unknowns so doctor doesn't pile a confusing "active status
        // must not carry closed:" on top of the schema-validation
        // failure that already flagged the typo.
        if let Some(s) = &status {
            let class = schema::status_class(known_schema, s);
            let recognised = known_schema.status_classes.contains_key(s.as_str())
                || crate::issue_fields::ACTIVE_STATUSES.contains(&s.as_str())
                || crate::issue_fields::is_closing_status(s);
            match class {
                // Gate on the schema's `required_when` declaration so
                // the closing-side rule is the SAME one `schema::validate`
                // enforces (and so relaxing/removing `closed.required_when`
                // in `.schema.yaml` relaxes this finding too). The
                // built-in default declares it, so behaviour is unchanged
                // for stock repos.
                schema::StatusClass::Closing
                    if closed.is_none()
                        && schema::field_required_for_status(known_schema, "closed", s) =>
                {
                    report.status_consistency.push((
                        slug.clone(),
                        format!("closing status {s:?} requires `closed:` date"),
                    ));
                }
                schema::StatusClass::Active if recognised && closed.is_some() => {
                    report.status_consistency.push((
                        slug.clone(),
                        format!("active status {s:?} must not carry `closed:`"),
                    ));
                }
                _ => {}
            }

            // `closed_by:` tracks `closed:` — the close path scrubs it on
            // the active edge, so an active issue carrying a closer is
            // self-inconsistent (legacy or hand-edited state). Flag it on
            // any recognised active status, independently of the `closed:`
            // check above so a stranded `closed_by` still surfaces even
            // when `closed:` was already cleared.
            if matches!(class, schema::StatusClass::Active) && recognised && closed_by.is_some() {
                report.status_consistency.push((
                    slug.clone(),
                    format!("active status {s:?} must not carry `closed_by:`"),
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
                report
                    .timestamp_issues
                    .push((slug.clone(), format!("closed ({x}) is after updated ({u})")));
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
                            let bare = s.trim().strip_prefix('@').unwrap_or(s.trim()).to_string();
                            if bare == *slug {
                                // Self-dep: surface explicitly so the user
                                // gets a focused remediation, and skip it
                                // for the cycle graph so we don't double-
                                // report it as a (trivial) 1-node cycle.
                                if !report.blocked_by_self.contains(slug) {
                                    report.blocked_by_self.push(slug.clone());
                                }
                            } else if existing_slugs.contains(&bare) {
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
        let in_both_legacy_folders =
            hits.iter().any(|h| h.folder == "open") && hits.iter().any(|h| h.folder == "closed");
        if in_both_legacy_folders {
            continue;
        }
        for hit in hits {
            if hit.folder != "open" && hit.folder != "closed" {
                continue;
            }
            let Some(fm) = hit.parsed.as_ref().and_then(|p| p.mapping.as_ref()) else {
                continue;
            };
            let Some(hit_status) = fm
                .get(serde_yaml::Value::String("status".into()))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            // A status value that `--fix` will coerce is owned by the
            // alias pass — skip reconciliation for it so the two passes
            // don't both rewrite the same field (which classified the
            // pre-coercion value with the lenient `Active` default and
            // would otherwise clobber the coerced result).
            if schema::would_coerce(known_schema, "status", hit_status).is_some() {
                continue;
            }
            match hit.folder.as_str() {
                "closed"
                    if (known_schema.status_classes.contains_key(hit_status)
                        || crate::issue_fields::ACTIVE_STATUSES.contains(&hit_status)
                        || crate::issue_fields::is_closing_status(hit_status))
                        && schema::status_class(known_schema, hit_status)
                            == schema::StatusClass::Active =>
                {
                    report.closed_with_active_status.push((
                        slug.clone(),
                        hit_status.to_string(),
                        hit.item_path.clone(),
                    ));
                }
                "open" if schema::is_closing(known_schema, hit_status) => {
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

/// Canonical issuectl-tracked files that should never be ignored by
/// `.gitignore`. If any of these exist locally but `git check-ignore`
/// says they're masked, teammates and CI won't see them — the local
/// developer will believe `agents init` / schema setup worked.
const GITIGNORE_CANONICAL_PATHS: &[&str] = &[".issuectl/AGENTS.md", "issues/.schema.yaml"];

/// Files larger than this under an issue directory are flagged as large
/// binaries (warning-only). 1 MiB: a git-tracked issue tracker shouldn't
/// carry artifacts this size inline without a deliberate choice, because
/// every revision is kept forever in history. The suggested remedies are
/// external storage or a `.gitignore` entry.
const LARGE_BINARY_BYTES: u64 = 1 << 20;

/// Raster image extensions the AVIF convention asks contributors to
/// convert. Flagged as warning-only nudges, independent of size.
const NON_AVIF_IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tiff", "tif"];

/// Run `git check-ignore -- <path>...` against the canonical paths
/// that exist on disk and return those that git would actually ignore.
/// Silent no-op when this is not a git repo or `git` is unavailable.
///
/// Deliberately does NOT pass `--no-index`. Without that flag, git
/// returns exit 1 (not ignored) for any tracked file even when the
/// path matches a `.gitignore` pattern — which is the correct
/// semantics for the "teammates won't see this file" warning. With
/// `--no-index`, git reports tracked-but-pattern-matched files as
/// ignored, producing false positives in the common migration
/// scenario where someone committed `.issuectl/AGENTS.md` and later
/// added `.issuectl/` to `.gitignore`.
fn detect_gitignored_canonical_paths(repo_root: &Path) -> Vec<String> {
    let candidates: Vec<&str> = GITIGNORE_CANONICAL_PATHS
        .iter()
        .copied()
        .filter(|rel| repo_root.join(rel).exists())
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("check-ignore")
        .arg("--")
        .args(&candidates)
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    // git check-ignore exit codes:
    //   0 — at least one path is ignored
    //   1 — no paths ignored
    //   128 — fatal error (e.g. not a git repo)
    if !matches!(output.status.code(), Some(0)) {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut hits: Vec<String> = stdout
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();
    hits.sort();
    hits.dedup();
    hits
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
            dfs(
                node,
                graph,
                &mut stack,
                &mut on_stack,
                &mut visited,
                &mut found,
            );
        }
    }
    found.into_iter().collect()
}

fn populate_notes_migration(scan: &ScanResult, report: &mut DoctorFindings) {
    for s in &scan.issues {
        if s.folder != "flat" || !s.item_present {
            continue;
        }
        let Some(text) = s.text.as_deref() else {
            continue;
        };
        match classify_notes(text) {
            NotesScan::NoOp => {}
            // Both SafeRename and Merge are forward-fixable by
            // `migrate_notes_heading`; the apply pass re-classifies and
            // routes each to a rename or a merge.
            NotesScan::SafeRename | NotesScan::Merge => {
                report.notes_to_rename.push(s.dir_name.clone())
            }
            NotesScan::Conflict => report.notes_conflicts.push(s.dir_name.clone()),
        }
    }
}

fn populate_schema_violations(
    scan: &ScanResult,
    repo_root: &Path,
    schema: &schema::Schema,
    report: &mut DoctorFindings,
) {
    for s in &scan.issues {
        if !s.item_present {
            continue;
        }
        // Skip dirs queued for the NN-rename pipeline — `--fix`
        // rewrites their frontmatter wholesale, so flagging schema
        // violations on them is noise that would refuse the very
        // fix designed to heal them. The typed signal is
        // `legacy_number.is_some()`, set by `legacy_number_from_mapping`
        // when frontmatter has neither `number:` nor `slug:` and the
        // dirname parses as `<NN>-<rest>`. This applies regardless of
        // folder: pre-migration a numbered-legacy lives under
        // `issues/{open,closed}/`, but after the flat-layout migration
        // moves it up, the same dir lives at `issues/<NN>-<rest>/`
        // pending NN-rename. A user-named flat slug like
        // `12-things-to-do` carries `slug:` in frontmatter (written
        // by `issuectl new`) and so does NOT trip this skip.
        if s.legacy_number.is_some() {
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
            report.parse_errors.push(ParseError {
                location,
                message: err.clone(),
                severity: ParseSeverity::Hard,
            });
            continue;
        }
        let Some(parsed) = s.parsed.as_ref() else {
            continue;
        };
        if parsed.fm_missing {
            report.parse_errors.push(ParseError {
                location,
                message: "missing or unterminated frontmatter".into(),
                severity: ParseSeverity::Hard,
            });
            continue;
        }
        if let Some(err) = &parsed.fm_yaml_error {
            report.parse_errors.push(ParseError {
                location,
                message: err.clone(),
                severity: ParseSeverity::Hard,
            });
            continue;
        }
        let Some(fm) = parsed.mapping.as_ref() else {
            continue;
        };
        for v in schema::validate(schema, fm) {
            // The built-in `closed` required-when-closing rule is
            // surfaced by the lifecycle-aware status/closed consistency
            // check in `populate_extended_validation`; suppress only
            // THAT specific violation here so the same condition isn't
            // reported twice. Any OTHER `required_when` (a user-declared
            // conditional field) has no other reporting channel, so it
            // must flow through to `schema_violations`.
            if let schema::ViolationKind::RequiredWhen { field, .. } = &v {
                if field == "closed" {
                    continue;
                }
            }
            // An enum violation on a value `doctor --fix` would coerce
            // is reported as a pending coercion instead of a violation.
            if let schema::ViolationKind::InvalidEnum { field, value, .. } = &v {
                if schema::would_coerce(schema, field, value).is_some() {
                    continue;
                }
            }
            report
                .schema_violations
                .push((location.clone(), v.message()));
        }
    }
}

/// Detect legacy `status` / `type` values that map to a canonical value
/// via the schema's alias tables. Records them as pending coercions so
/// the read-only report shows the planned rewrite and `--fix` applies
/// it. Skips dirs queued for the NN-rename pipeline (same typed
/// `legacy_number.is_some()` signal as `populate_schema_violations`).
fn populate_alias_coercions(
    scan: &ScanResult,
    schema: &schema::Schema,
    report: &mut DoctorFindings,
) {
    for s in &scan.issues {
        if !s.item_present || s.legacy_number.is_some() {
            continue;
        }
        let Some(fm) = s.parsed.as_ref().and_then(|p| p.mapping.as_ref()) else {
            continue;
        };
        for field in ["status", "type"] {
            let Some(value) = fm
                .get(serde_yaml::Value::String(field.into()))
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
            else {
                continue;
            };
            if let Some(to) = schema::would_coerce(schema, field, value) {
                report.alias_coercions.push((
                    s.dir_name.clone(),
                    field.to_string(),
                    value.to_string(),
                    to.to_string(),
                    s.item_path.clone(),
                ));
            }
        }
    }
    report.alias_coercions.sort();
}

fn populate_transition_warnings(
    scan: &ScanResult,
    rules: Option<&crate::transitions::TransitionRules>,
    schema: Option<&schema::Schema>,
    report: &mut DoctorFindings,
) {
    let rules_active = rules.map(|r| !r.status_rules.is_empty()).unwrap_or(false);
    let sections_active = schema.map(|s| !s.body_sections.is_empty()).unwrap_or(false);
    if !rules_active && !sections_active {
        return;
    }
    for s in &scan.issues {
        if !s.item_present {
            continue;
        }
        // Mirror the typed `legacy_number.is_some()` skip used by
        // `populate_schema_violations` and `populate_extended_validation`
        // — a numbered-legacy lifted to flat by phase 5 is the same
        // issue pending NN-rename; transition warnings on it would
        // refuse the very fix designed to heal it.
        if s.legacy_number.is_some() {
            continue;
        }
        let Some(parsed) = s.parsed.as_ref() else {
            continue;
        };
        // S5: only skip when essential frontmatter is absent. A legacy
        // numeric-epic ref produces a warning but leaves `status` /
        // `type` intact, so the lint can still run usefully.
        if essential_frontmatter_absent_from_mapping(parsed.mapping.as_ref()) {
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

fn essential_frontmatter_absent_from_mapping(mapping: Option<&serde_yaml::Mapping>) -> bool {
    let Some(m) = mapping else { return true };
    let has = |k: &str| {
        m.get(serde_yaml::Value::String(k.into()))
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    };
    !has("status") || !has("type")
}

fn apply(
    repo_root: &Path,
    mut actions: DoctorActions,
    lock: &crate::mutate::WriteLock,
) -> Result<ApplyOutcome> {
    let mut outcome = ApplyOutcome::default();

    // Schema bootstrap runs UNCONDITIONALLY, before the preflight
    // refusal. The read-only `doctor` output advertises that
    // `.schema.yaml` will be auto-created on first `--fix`; gating
    // bootstrap on an empty preflight blocker list breaks that
    // promise for repos with any other violation present. The
    // operation is idempotent (`ensure_default_written` returns
    // `false` when the file exists) and writes a known-good template
    // — there is no failure mode where running it makes the repo
    // worse than before. (issue: @unreasonably-attractive-star)
    let issues_dir = repo_root.join("issues");
    fs::create_dir_all(&issues_dir)
        .with_context(|| format!("cannot create {}", issues_dir.display()))?;
    outcome.schema_bootstrapped = schema::ensure_default_written(repo_root)?;

    // Preflight: refuse to mutate when a layout-fatal blocker is
    // present (see `BlockerScope::ApplyPreflight` for the narrowed
    // list). Schema-shape findings deliberately do NOT block the
    // apply pipeline — the user can fix layout first and address
    // schema violations against the post-migration state. We DO NOT
    // `bail!` — the blockers go into the outcome so `--json --fix`
    // callers receive structured output instead of an anyhow-
    // formatted stderr blob (the AGENTS.md "always `--json` when
    // scripting" promise).
    if !actions.preflight_blockers.is_empty() {
        // Schema bootstrap above may have written `.schema.yaml`
        // before this preflight refusal — that's intentional and the
        // documented behaviour (issue: @unreasonably-attractive-star).
        // The Preflight invariant in `stop_with_blockers` accepts
        // `schema_bootstrapped` as the one allowed pre-preflight
        // write; no state masking is needed.
        outcome.stop_with_blockers(
            StopPhase::Preflight,
            std::mem::take(&mut actions.preflight_blockers),
        );
        return Ok(outcome);
    }

    // Orphan tempfile cleanup runs FIRST so paths recorded by scan()
    // are still valid: directory migration would invalidate them.
    apply_orphan_tempfiles(&mut actions, &mut outcome)?;

    // Alias coercion (legacy status/type values → canonical) runs
    // BEFORE status/folder reconciliation: reconciliation classifies a
    // status via the lifecycle layering, and a legacy value like
    // `resolved` resolves to the lenient `Active` default until it is
    // coerced to its canonical (`fixed`, closing) form. Coercing first
    // means reconciliation reasons about canonical statuses, not the
    // pre-migration ones. Both run before the flat-layout migration so
    // the rewrites land at the legacy path scan() recorded. Schema is
    // loaded post-bootstrap so a repo with no prior `.schema.yaml`
    // still gets the built-in alias table.
    let apply_schema = schema::load(repo_root)?;
    apply_alias_coercions(&mut actions, &mut outcome, &apply_schema)?;

    // Status/folder reconciliation runs BEFORE the flat-layout
    // migration so the rewrites land at the legacy path that scan()
    // recorded; the subsequent migration moves the corrected file.
    apply_status_reconciliation(&mut actions, &mut outcome)?;

    // Notes → Comments migration is independent of layout migration:
    // it touches body markdown of flat-layout dirs only, never moves
    // files. Run it FIRST so layout-conflict bail-outs don't block
    // unrelated body fixes (round-2 finding O18).
    rename_notes_to_comments(repo_root, &mut actions, &mut outcome)?;

    regenerate_agents_md(repo_root, &actions, &mut outcome)?;
    rewrite_legacy_issues_agents_md(repo_root, &actions, &mut outcome)?;

    // Flat-layout migration: any issue still under
    // `issues/{open,closed}/<slug>/` moves up to `issues/<slug>/`. The
    // pre-acquired write lock in `run` covers this — `execute_migrate_layout_plan`
    // is the lock-free body and must not re-acquire.
    let mut legacy_dirs = std::mem::take(&mut actions.legacy_dirs);
    if let Some(plan) = actions.flat_layout_plan.take() {
        if !plan.moves().is_empty() {
            // `ExecuteOutcome` carries partial progress on mid-loop
            // failure so the user-facing summary can still render
            // "moved A, B before failing on C".
            let exec_outcome = execute_migrate_layout_plan(plan, lock);
            outcome.flat_layout_migrated = exec_outcome.migrated;
            // Prune empty `issues/{open,closed}` parent dirs as soon
            // as the moves land — every code path below this point
            // can early-return (post-migration blocker bail, empty
            // `legacy_dirs`, or successful NN-rename), and the prune
            // is best-effort idempotent so calling it once here is
            // simpler than gating it at every exit.
            crate::migrate_layout::prune_empty_legacy_parents(&repo_root.join("issues"));
            if let Some(err) = exec_outcome.error {
                // Forward-progress only: surface the failure cause on
                // the structured outcome and bail. Returning `Err` here
                // would propagate past `render_text` / `render_json` and
                // strand the partial `flat_layout_migrated` (already on
                // disk) inside an anyhow text blob on stderr — invisible
                // to `--json` consumers.
                outcome.apply_error = Some(format!("{err:#}"));
                return Ok(outcome);
            }
            // Re-scan so the NN-rename pass operates on fresh
            // `old_path`s and picks up frontmatter-only legacy issues
            // that just moved into the flat layout.
            let fresh = scan(repo_root)?;
            // Re-check `apply_blockers` (the layout-fatal subset)
            // against the fresh scan before the NN-rename phase.
            // Phase 5 can surface a layout-fatal condition that was
            // hidden by the pre-migration layout —
            // `populate_notes_migration` walks only flat-folder dirs,
            // so a `## Notes` / `## Comments` ambiguity in a body
            // that was still under `issues/{open,closed}/` is
            // invisible to the initial scan, and the planner's own
            // `flat_layout_conflicts` could surface only on the
            // post-move state in unusual layouts. NN-rename builds
            // `number_to_slug` against `legacy_dirs` and rewrites
            // refs + renames dirs based on it; running that pass
            // over a layout-unhealthy repo can rewrite refs to the
            // wrong target or have `fs::rename` overwrite a sibling.
            // We use `apply_blockers` (not the broader
            // `critical_blockers`) so newly-surfaced schema
            // violations don't strand the partial layout migration —
            // schema fixes are forward work the user does after the
            // layout is in place. Forward-progress only: rolling
            // back N partial renames is itself a multi-step
            // operation that can fail mid-rollback.
            let post_blockers = apply_blockers(&fresh);
            if !post_blockers.is_empty() {
                outcome.stop_with_blockers(StopPhase::PostApply, post_blockers);
                return Ok(outcome);
            }
            // Re-run the Notes → Comments rename against the
            // post-migration state. `populate_notes_migration` walks
            // only `folder == "flat"` dirs, so any issue still under
            // `issues/{open,closed}/<slug>/` whose body has `## Notes`
            // is invisible to the pre-migration scan. After phase 5
            // lifts it to `issues/<slug>/`, the rename is applicable —
            // running it here closes the one-shot `--fix` contract so
            // users don't have to invoke `doctor --fix` twice. Safe to
            // call twice in the same apply: `rename_notes_to_comments`
            // appends to `outcome.notes_renamed`, and the first call
            // already drained `actions.notes_to_rename`.
            actions.notes_to_rename = fresh.notes_to_rename;
            // Post-flat-layout dirs may now expose `## Notes`/`## Comments`
            // ambiguity that was invisible while still under `issues/{open,closed}/`.
            // Surface them via the same outcome field so they don't
            // silently disappear (issue: @doctor-fix-noop).
            actions.notes_conflicts = fresh.notes_conflicts;
            rename_notes_to_comments(repo_root, &mut actions, &mut outcome)?;
            legacy_dirs = fresh.legacy_dirs;
        }
    }

    if legacy_dirs.is_empty() {
        return Ok(outcome);
    }

    // Build maps for reference rewriting.
    let mut number_to_slug: BTreeMap<u32, String> = BTreeMap::new();
    let mut dir_to_slug: BTreeMap<String, String> = BTreeMap::new();
    for m in &legacy_dirs {
        let _prev = number_to_slug.insert(m.old_number, m.new_slug.clone());
        // Duplicate legacy numbers are flagged via build_ambiguous below;
        // rewrites for those numbers will be skipped.
        dir_to_slug.insert(m.old_dir_name.clone(), m.new_slug.clone());
    }

    let ambiguous_numbers = build_ambiguous(&legacy_dirs);

    // Single-phase atomic rename: old dirname (`<NN>-<slug>`) and new
    // slug (`<intensifier-adj-noun>`) cannot collide, so the temp-suffix
    // shuffle that the previous version did is unnecessary — and worse,
    // an interruption mid-shuffle would leave `*.issuectl-doctor-<pid>`
    // dirs that no subsequent doctor run could recognize.
    for m in &legacy_dirs {
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

    for m in &legacy_dirs {
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
    outcome.files_rewritten = files_rewritten;
    outcome.legacy_dirs_migrated = legacy_dirs;

    // Prune empty `issues/{open,closed}` parent dirs again — covers
    // the numbered-legacy-only repo path where the flat-layout
    // planner had no moves and the earlier in-pipeline prune did not
    // run. Idempotent and best-effort.
    crate::migrate_layout::prune_empty_legacy_parents(&repo_root.join("issues"));

    Ok(outcome)
}

/// Apply the Notes → Comments rename to every slug in
/// `actions.notes_to_rename`. Best-effort, sequential (per round-2
/// decision: `O17` is intentionally not preflight-bail). Conflicts
/// are populated by the upstream scan; this function does not
/// re-classify. Callable multiple times in one `apply` pass —
/// `mem::take` drains the input on each call and outcomes append to
/// `outcome.notes_renamed` / `outcome.notes_conflicts_at_apply`.
/// Regenerate the schema-derived block in `.issuectl/AGENTS.md` when
/// scan flagged drift. No-op if the file is absent (init is opt-in),
/// the block is already in sync, the file is malformed (refuse —
/// auto-collapse would destroy user content), or the schema/rules
/// failed to parse (would regenerate from defaults, overwriting real
/// policy). Doctor's run() already holds `mutate::WriteLock` for the
/// whole apply pass; this function does not re-acquire.
fn regenerate_agents_md(
    repo_root: &Path,
    actions: &DoctorActions,
    outcome: &mut ApplyOutcome,
) -> Result<()> {
    if !actions.regenerate_agents_md {
        return Ok(());
    }
    let path = agents::agents_path(repo_root);
    if !path.is_file() {
        return Ok(());
    }
    let original =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let schema = schema::load(repo_root)?;
    let rules = crate::transitions::load(repo_root)?;
    let new_text = agents::regenerate_managed(&original, &schema, &rules)?;
    if new_text != original {
        agents::atomic_write(&path, new_text.as_bytes())?;
        outcome.agents_md_regenerated = true;
    }
    Ok(())
}

/// Heuristic for "this is the pre-v0.5.0 `issues/AGENTS.md` scaffold,
/// not user-authored content." Any one of the markers is enough — they
/// all point at concepts (numbered layout, `open/`/`closed/` subdirs,
/// sequential numbering) that no current template would produce.
fn is_legacy_issues_agents(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "## Issue Numbering",
        "├── open/",
        "└── open/",
        "NN-short-title",
        "moved from `open/` to `closed/`",
    ];
    MARKERS.iter().any(|m| text.contains(m))
}

fn rewrite_legacy_issues_agents_md(
    repo_root: &Path,
    actions: &DoctorActions,
    outcome: &mut ApplyOutcome,
) -> Result<()> {
    if !actions.rewrite_issues_agents_md {
        return Ok(());
    }
    let path = repo_root.join("issues").join("AGENTS.md");
    if !path.is_file() {
        return Ok(());
    }
    fs::write(&path, crate::skill::ISSUES_AGENTS_TEMPLATE)
        .with_context(|| format!("cannot write {}", path.display()))?;
    outcome.issues_agents_md_rewritten = true;
    Ok(())
}

fn rename_notes_to_comments(
    repo_root: &Path,
    actions: &mut DoctorActions,
    outcome: &mut ApplyOutcome,
) -> Result<()> {
    let issues = repo_root.join("issues");
    // Surface scan-time `## Notes`/`## Comments` conflicts via the same
    // outcome field as TOCTOU-race skips. Manual merge is required; the
    // apply pipeline used to bail the whole pass on these (issue:
    // @doctor-fix-noop). Drain so a second call (post-flat-layout
    // rescan) only adds newly-discovered conflicts.
    for slug in std::mem::take(&mut actions.notes_conflicts) {
        if !outcome.notes_conflicts_at_apply.contains(&slug) {
            outcome.notes_conflicts_at_apply.push(slug);
        }
    }
    let planned = std::mem::take(&mut actions.notes_to_rename);
    for slug in planned {
        let item_path = issues.join(&slug).join("item.md");
        if !item_path.is_file() {
            continue;
        }
        let original = fs::read_to_string(&item_path)
            .with_context(|| format!("cannot read {}", item_path.display()))?;
        let (rewritten, has_conflict) = migrate_notes_heading(&original);
        if has_conflict {
            // Conflict surfaced during apply (file changed between
            // scan and apply — manual edit). Record it explicitly:
            // the post-apply re-scan will pick up the conflict only
            // if both headings are still present, but we need a
            // reliable signal even on the no-write path so JSON
            // consumers see that planned work was skipped.
            outcome.notes_conflicts_at_apply.push(slug);
            continue;
        }
        if rewritten != original {
            fs::write(&item_path, rewritten)
                .with_context(|| format!("cannot write {}", item_path.display()))?;
            outcome.notes_renamed.push(slug);
        }
    }
    Ok(())
}

fn apply_orphan_tempfiles(actions: &mut DoctorActions, outcome: &mut ApplyOutcome) -> Result<()> {
    let planned = std::mem::take(&mut actions.orphan_tempfiles);
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
    outcome.orphan_tempfiles_removed = removed;
    Ok(())
}

/// Best-effort closed-date for an issue being coerced/reconciled into a
/// closing status without an explicit `closed:` date. Stamping
/// `write::today()` is lossy for an issue that was actually closed long
/// ago, so we prefer, in order: the author date of the last git commit
/// that touched `item.md`, the file's mtime, then today(). All steps are
/// best-effort — any failure (not a git repo, untracked file, unreadable
/// metadata) falls through to the next source.
fn derive_closed_date(item_path: &Path) -> String {
    git_last_commit_date(item_path)
        .or_else(|| file_mtime_date(item_path))
        .unwrap_or_else(write::today)
}

/// Author date (`%aI`, strict ISO 8601) of the last commit that touched
/// `item_path`, converted to the machine's local timezone and projected
/// to `YYYY-MM-DD`. Converting to local — rather than slicing the raw
/// committer-TZ date — keeps this consistent with `write::today()` and
/// `file_mtime_date`, which both use local time, so the three fallback
/// tiers never disagree by a day. `--follow` lets it find the history of
/// a file that was renamed (e.g. an earlier flat-layout move that was
/// already committed). `None` when git is unavailable, the path is not
/// in a git repo, or the file is untracked (empty output).
fn git_last_commit_date(item_path: &Path) -> Option<String> {
    let dir = item_path.parent()?;
    let name = item_path.file_name()?;
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["log", "-1", "--follow", "--format=%aI", "--"])
        .arg(name)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Parsing the full RFC 3339 line validates it end-to-end (a
    // malformed `%aI` never lands in frontmatter) and carries the
    // offset, so the local-time conversion below is correct.
    let dt = chrono::DateTime::parse_from_rfc3339(stdout.trim()).ok()?;
    Some(
        dt.with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string(),
    )
}

/// File mtime of `item_path` projected to a local-time `YYYY-MM-DD`.
fn file_mtime_date(item_path: &Path) -> Option<String> {
    let modified = fs::metadata(item_path).ok()?.modified().ok()?;
    let datetime: chrono::DateTime<chrono::Local> = modified.into();
    Some(datetime.format("%Y-%m-%d").to_string())
}

fn apply_status_reconciliation(
    actions: &mut DoctorActions,
    outcome: &mut ApplyOutcome,
) -> Result<()> {
    let active_to_closed = std::mem::take(&mut actions.closed_with_active_status);
    let closing_to_open = std::mem::take(&mut actions.open_with_closing_status);
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
            write::set_string(
                &mut item.frontmatter,
                "closed",
                &derive_closed_date(&item_path),
            );
        }
        write::write_item(&item_path, &item)?;
        outcome.status_reconciled.push(slug);
    }
    for (slug, _old_status, item_path) in closing_to_open {
        let mut item = write::read_item(&item_path)?;
        write::set_string(&mut item.frontmatter, "status", "open");
        write::remove_key(&mut item.frontmatter, "closed");
        write::write_item(&item_path, &item)?;
        outcome.status_reconciled.push(slug);
    }
    Ok(())
}

/// Rewrite legacy `status` / `type` values to their canonical form via
/// the schema alias tables. Re-reads the on-disk value and only
/// rewrites when it still equals the recorded `from`. This guard covers
/// both an external concurrent edit between scan and apply AND an
/// earlier in-process apply step that already changed the field, so a
/// stale coercion never clobbers a fresher value. When a coerced
/// status lands in a closing lifecycle class and no `closed:` date is
/// present, a `closed:` date is stamped — mirroring the status command
/// so the migrated issue doesn't immediately trip the `closed:`
/// required-when rule.
fn apply_alias_coercions(
    actions: &mut DoctorActions,
    outcome: &mut ApplyOutcome,
    schema: &schema::Schema,
) -> Result<()> {
    let planned = std::mem::take(&mut actions.alias_coercions);
    // Group consecutive coercions that share an `item_path` so an issue
    // carrying BOTH a status and a type coercion is read once and written
    // once instead of read+written per field. `planned` is sorted by scan
    // (slug first), so a given issue's entries are already adjacent; the
    // run-length accumulator below relies on that for single-read grouping
    // but stays correct (just less optimal) if the order ever changes, and
    // preserves planned order so `alias_coercions_applied` is deterministic.
    // `(slug, field, from, to)` — one applied/planned coercion sans path.
    type Coercion = (String, String, String, String);
    let mut groups: Vec<(PathBuf, Vec<Coercion>)> = Vec::new();
    for (slug, field, from, to, item_path) in planned {
        match groups.last_mut() {
            Some((p, v)) if *p == item_path => v.push((slug, field, from, to)),
            _ => groups.push((item_path, vec![(slug, field, from, to)])),
        }
    }

    for (item_path, coercions) in groups {
        if !item_path.is_file() {
            continue;
        }
        let mut item = write::read_item(&item_path)?;
        let mut applied: Vec<Coercion> = Vec::new();
        let mut coerced_to_closing = false;
        for (slug, field, from, to) in coercions {
            // Re-read the field from the in-memory mapping and only
            // rewrite when it still equals the recorded `from`. Guards
            // against a stale plan (external edit between scan and apply)
            // clobbering a fresher value.
            let current = item
                .frontmatter
                .get(serde_yaml::Value::String(field.clone()))
                .and_then(|v| v.as_str());
            if current != Some(from.as_str()) {
                continue;
            }
            write::set_string(&mut item.frontmatter, &field, &to);
            if field == "status" && schema::is_closing(schema, &to) {
                coerced_to_closing = true;
            }
            applied.push((slug, field, from, to));
        }
        if coerced_to_closing {
            let has_closed = item
                .frontmatter
                .get(serde_yaml::Value::String("closed".into()))
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if !has_closed {
                write::set_string(
                    &mut item.frontmatter,
                    "closed",
                    &derive_closed_date(&item_path),
                );
            }
        }
        if !applied.is_empty() {
            write::write_item(&item_path, &item)?;
            outcome.alias_coercions_applied.extend(applied);
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
    /// File has exactly one `## Notes` AND exactly one `## Comments`.
    /// The two are auto-merged: `## Notes`' entries fold into
    /// `## Comments` (document order preserved) and `## Notes` is
    /// dropped (issue @doctor-fix-merge-notes-comments).
    Merge,
    /// File has more than one `## Notes` (with or without
    /// `## Comments`), OR one `## Notes` alongside multiple
    /// `## Comments`. The merge target is ambiguous (round-2 finding
    /// G5/O5), so we skip and surface the slug for manual merge.
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
    } else if notes == 1 && comments == 1 {
        NotesScan::Merge
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

/// Pure function: migrate `## Notes` toward `## Comments`. When only
/// `## Notes` exists it's renamed; when both exist (exactly one of
/// each) `## Notes`' entries are folded into `## Comments` in document
/// order and `## Notes` is dropped. Fence-aware so a `## Notes` line
/// inside a code block is preserved verbatim. Returns
/// `(new_text, conflict)` — `conflict=true` only for the genuinely
/// ambiguous shapes (multiple `## Notes`, or a `## Notes` alongside
/// multiple `## Comments`) which the caller skips and surfaces.
fn migrate_notes_heading(text: &str) -> (String, bool) {
    use crate::body_sections::{closes_fence, opening_fence, Fence};
    match classify_notes(text) {
        NotesScan::NoOp => return (text.to_string(), false),
        NotesScan::Conflict => return (text.to_string(), true),
        NotesScan::Merge => {
            return (
                crate::body_sections::merge_h2_section(text, "Notes", "Comments"),
                false,
            )
        }
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

    // Migrate `related` / `blocked_by`: ["#NN", ...] → ["@<slug>", ...]
    // when unambiguous.
    for key in ["related", "blocked_by"] {
        let yaml_key = serde_yaml::Value::String(key.into());
        if let Some(serde_yaml::Value::Sequence(seq)) = item.frontmatter.get(&yaml_key).cloned() {
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
                .insert(yaml_key, serde_yaml::Value::Sequence(new_seq));
        }
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
    // Skip-region awareness (fenced code blocks, inline code spans,
    // and link URLs) is delegated to the shared
    // `body_sections::rewrite_outside_code_and_urls` walker that
    // `refs::rewrite_body_refs` also uses — keeping the two callers
    // from drifting on which markdown constructs are off-limits.
    crate::body_sections::rewrite_outside_code_and_urls(
        text,
        crate::body_sections::RewriteSkips::code_only(),
        |seg| {
            // heading_re is line-anchored, but a prose segment that
            // starts at a line beginning (the common case for legacy
            // `# E10. Title` headings — which never contain inline code
            // or link URLs in the heading number/dot) still matches the
            // pattern. If the segment doesn't begin a line, `^` simply
            // doesn't fire and the segment passes through.
            let seg = heading_re.replace(seg, "$1$2");
            let seg = ref_re.replace_all(&seg, |caps: &Captures| {
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
            });
            let mut s = seg.into_owned();
            for (re, new) in &dir_regexes {
                s = re
                    .replace_all(&s, |caps: &Captures| {
                        format!("{}{}{}", &caps[1], new, &caps[2])
                    })
                    .to_string();
            }
            s
        },
    )
}

// ── Output rendering ────────────────────────────────────────────────────────

fn planned_moves(report: &DoctorFindings) -> &[PlannedMove] {
    report
        .flat_layout_plan
        .as_ref()
        .map(|p| p.moves())
        .unwrap_or(&[])
}

/// Threshold above which long warning lists collapse to a one-line
/// count in default rendering. `--verbose` always prints the full
/// list. The number itself is a UX dial — small enough that an
/// "almost-clean" repo still shows individual entries, large enough
/// that a real-world legacy repo's 100+ entries collapse cleanly.
const RENDER_FULL_LIST_LIMIT: usize = 10;

/// Render a list section to `out`, collapsing to a one-liner when
/// not `verbose` and the list exceeds `RENDER_FULL_LIST_LIMIT`
/// entries. Caller passes the `verb_phrase` used in the collapsed
/// line (e.g. "need layout migration"). Empty lists render nothing.
/// Writing through `&mut dyn fmt::Write` keeps the helper testable
/// against an in-memory buffer (issue:
/// `@ridiculously-outrageous-fold`).
fn render_section<T>(
    out: &mut dyn fmt::Write,
    title: &str,
    items: &[T],
    verbose: bool,
    verb_phrase: &str,
    fmt_item: impl Fn(&T) -> String,
) {
    if items.is_empty() {
        return;
    }
    if !verbose && items.len() > RENDER_FULL_LIST_LIMIT {
        let _ = writeln!(
            out,
            "{} {} (re-run with --verbose to list).",
            items.len(),
            verb_phrase
        );
        let _ = writeln!(out);
        return;
    }
    let _ = writeln!(out, "{title}");
    for it in items {
        let _ = writeln!(out, "  {}", fmt_item(it));
    }
    let _ = writeln!(out);
}

/// `render_section` adapter that prints to stdout.
fn print_section<T>(
    title: &str,
    items: &[T],
    verbose: bool,
    verb_phrase: &str,
    fmt_item: impl Fn(&T) -> String,
) {
    let mut buf = String::new();
    render_section(&mut buf, title, items, verbose, verb_phrase, fmt_item);
    print!("{buf}");
}

fn render_text(report: &DoctorFindings, outcome: Option<&ApplyOutcome>, fix: bool, verbose: bool) {
    let outcome_default = ApplyOutcome::default();
    let oc = outcome.unwrap_or(&outcome_default);
    let has_problems = !report.legacy_dirs.is_empty()
        || !planned_moves(report).is_empty()
        || !oc.flat_layout_migrated.is_empty()
        || !report.flat_layout_conflicts.is_empty()
        || !report.invalid_slugs.is_empty()
        || !report.duplicate_slugs.is_empty()
        || !report.missing_item_md.is_empty()
        || !report.orphan_epic_refs.is_empty()
        || !report.parse_errors.is_empty()
        || !oc.notes_renamed.is_empty()
        || !report.notes_to_rename.is_empty()
        || !report.notes_conflicts.is_empty()
        || !report.schema_violations.is_empty()
        || !report.alias_coercions.is_empty()
        || !oc.alias_coercions_applied.is_empty()
        || report.schema_parse_error.is_some()
        || !report.broken_refs.is_empty()
        || !report.blocked_by_cycles.is_empty()
        || !report.blocked_by_self.is_empty()
        || !report.status_consistency.is_empty()
        || !report.timestamp_issues.is_empty()
        || !report.unknown_keys.is_empty()
        || !report.unknown_reviewers.is_empty()
        || !report.conflict_markers.is_empty()
        || !report.orphan_tempfiles.is_empty()
        || !oc.orphan_tempfiles_removed.is_empty()
        || !report.symlinked_dirs.is_empty()
        || !report.both_open_and_closed.is_empty()
        || !report.closed_with_active_status.is_empty()
        || !report.open_with_closing_status.is_empty()
        || !oc.status_reconciled.is_empty()
        || !report.transition_warnings.is_empty()
        || !report.missing_body_sections.is_empty()
        || report.agents_md_drift
        || report.agents_md_malformed.is_some()
        || report.agents_md_check_skipped.is_some()
        || oc.agents_md_regenerated
        || report.agents_md_missing
        || report.legacy_issues_agents_md
        || !report.gitignored_paths.is_empty()
        || oc.issues_agents_md_rewritten
        || !report.large_binaries.is_empty()
        || !report.non_avif_images.is_empty()
        || !report.broken_attachment_refs.is_empty()
        || !oc.blockers.is_empty()
        || oc.apply_error.is_some();
    if !oc.blockers.is_empty() {
        println!("doctor: cannot safely apply --fix until these issues are resolved:");
        for b in &oc.blockers {
            println!("  - {b}");
        }
        println!();
    }
    if let Some(err) = &oc.apply_error {
        println!("doctor: --fix aborted mid-pipeline; partial progress retained:");
        println!("  {err}");
        println!();
    }
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

    if !oc.flat_layout_migrated.is_empty() {
        print_section(
            "Migrated to flat layout:",
            &oc.flat_layout_migrated,
            verbose,
            "issue(s) migrated to flat layout",
            |m| format!("{}  ({} → {})", m.slug, m.from.display(), m.to.display()),
        );
    } else {
        let planned: Vec<_> = planned_moves(report).into_iter().collect();
        print_section(
            "Issues still in legacy `issues/{open,closed}/<slug>/` layout:",
            &planned,
            verbose,
            "issue(s) still in legacy `issues/{open,closed}/<slug>/` layout — re-run with --fix",
            |m| {
                format!(
                    "{}  ({} → {})",
                    m.slug(),
                    m.from().display(),
                    m.to().display()
                )
            },
        );
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
        let collapsed_phrase = if fix {
            "legacy numbered issue(s) migrated"
        } else {
            "legacy numbered issue(s) to migrate — re-run with --fix"
        };
        // Legacy <NN>-<slug> dirs are migrated to the canonical flat
        // path post-flat-layout; print the actual destination rather
        // than the (incorrect) "{folder}/{new}" pre-flat shape.
        print_section(title, &report.legacy_dirs, verbose, collapsed_phrase, |m| {
            format!(
                "{}/{}  →  {}",
                m.folder,
                m.old_dir_name,
                m.new_path.display()
            )
        });
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
        for e in &report.parse_errors {
            println!("  {}: {}", e.location, e.message);
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
    if !oc.notes_renamed.is_empty() {
        println!("Renamed `## Notes` → `## Comments`:");
        for s in &oc.notes_renamed {
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
    print_section(
        "Schema violations:",
        &report.schema_violations,
        verbose,
        "schema violation(s)",
        |(loc, msg)| format!("{loc}: {msg}"),
    );
    if !oc.alias_coercions_applied.is_empty() {
        println!("Coerced legacy values via schema aliases:");
        for (slug, field, from, to) in &oc.alias_coercions_applied {
            println!("  {slug}: {field} {from} → {to}");
        }
        println!();
    } else if !report.alias_coercions.is_empty() {
        println!("Legacy values to coerce via schema aliases (re-run with --fix):");
        for (slug, field, from, to, _) in &report.alias_coercions {
            println!("  {slug}: {field} {from} → {to}");
        }
        println!();
    }
    print_section(
        "Broken cross-references:",
        &report.broken_refs,
        verbose,
        "broken cross-reference(s)",
        |(slug, kind, target)| format!("{slug}: {kind} → {target}"),
    );
    if !report.blocked_by_cycles.is_empty() {
        println!("Dependency cycles via `blocked_by`:");
        for cycle in &report.blocked_by_cycles {
            println!("  {} → {}", cycle.join(" → "), cycle[0]);
        }
        println!();
    }
    if !report.blocked_by_self.is_empty() {
        println!("Self-dependencies in `blocked_by`:");
        for slug in &report.blocked_by_self {
            println!("  {slug}: lists itself as a blocker");
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
    if !report.unknown_reviewers.is_empty() {
        println!("Unknown reviewers (not present as reporter/assignee/owner anywhere):");
        for (slug, who) in &report.unknown_reviewers {
            println!("  {slug}: {who}");
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
    if !oc.orphan_tempfiles_removed.is_empty() {
        println!("Removed orphan tempfiles:");
        for p in &oc.orphan_tempfiles_removed {
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
        println!(
            "Slugs present in BOTH `issues/open/` and `issues/closed/` (manual fix required):"
        );
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
    if !oc.status_reconciled.is_empty() {
        println!("Reconciled status/folder mismatches:");
        for s in &oc.status_reconciled {
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
    if report.agents_md_missing {
        println!(
            "{} not present — run `issuectl agents init` to opt in to the agent policy file.",
            agents::AGENTS_RELATIVE_PATH
        );
        println!();
    }
    if !report.gitignored_paths.is_empty() {
        for p in &report.gitignored_paths {
            println!(
                "{p} is gitignored — agents on other machines won't see it. Adjust .gitignore or move the file."
            );
        }
        println!();
    }
    if oc.issues_agents_md_rewritten {
        println!(
            "Rewrote stale issues/AGENTS.md (pre-v0.5.0 scaffold) with current pointer template."
        );
        println!();
    } else if report.legacy_issues_agents_md {
        println!(
            "issues/AGENTS.md still carries the pre-v0.5.0 scaffold (re-run with --fix to replace)."
        );
        println!();
    }
    if oc.agents_md_regenerated {
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
    if !report.large_binaries.is_empty() {
        println!("Large binaries under issue dirs (consider external storage or .gitignore):");
        for (slug, path, bytes) in &report.large_binaries {
            println!("  {slug}: {path} ({} KiB)", bytes / 1024);
        }
        println!();
    }
    if !report.non_avif_images.is_empty() {
        println!("Non-AVIF images (convert to AVIF per the issue convention):");
        for (slug, path) in &report.non_avif_images {
            println!("  {slug}: {path}");
        }
        println!();
    }
    if !report.broken_attachment_refs.is_empty() {
        println!("Body references to missing files (relative paths that don't resolve):");
        for (slug, r) in &report.broken_attachment_refs {
            println!("  {slug}: {r}");
        }
        println!();
    }
    if fix {
        // Coherent end-of-run summary. Previously every `--fix` run
        // printed an `Applied. …` count line even when the pipeline
        // had refused to mutate at preflight, or when unfixable
        // findings remained (issue: @doctor-fix-noop). Prefix
        // follows `stop_phase` first, then falls back to whether the
        // post-apply scan still surfaces critical findings.
        let counts = format!(
            "{} legacy dir(s) migrated, {} flat-layout dir(s) migrated, {} markdown file(s) rewritten, {} `## Notes` rename(s), {} AGENTS.md block(s) regenerated.",
            oc.legacy_dirs_migrated.len(),
            oc.flat_layout_migrated.len(),
            oc.files_rewritten,
            oc.notes_renamed.len(),
            if oc.agents_md_regenerated { 1 } else { 0 }
        );
        match (oc.stop_phase, oc.apply_error.is_some()) {
            (StopPhase::Preflight, _) => {
                println!(
                    "Refused — {} preflight blocker(s); no writes applied.",
                    oc.blockers.len()
                );
            }
            (_, true) => {
                println!("Aborted mid-pipeline. {counts}");
            }
            (StopPhase::PostApply, _) => {
                println!(
                    "Partial — {} post-apply blocker(s); partial writes retained. {counts}",
                    oc.blockers.len()
                );
            }
            (StopPhase::Ok, _) if !oc.notes_conflicts_at_apply.is_empty() => {
                println!(
                    "Partial — auto-fixes ran where possible. {} issue(s) need manual attention (see above). {counts}",
                    oc.notes_conflicts_at_apply.len()
                );
            }
            (StopPhase::Ok, _) => {
                // Even on a clean apply pass, unfixable findings in
                // the post-apply scan (e.g. schema violations,
                // broken refs) drive exit-1; the summary must
                // acknowledge them rather than claim success.
                let crit = critical_blockers(report);
                if crit.is_empty() {
                    println!("Applied. {counts}");
                } else {
                    println!(
                        "Partial — {} unfixable finding(s) remain (see above). {counts}",
                        crit.len()
                    );
                }
            }
        }
    } else {
        println!("Read-only — re-run with --fix to apply.");
    }
}

fn render_json(
    report: &DoctorFindings,
    outcome: Option<&ApplyOutcome>,
    fix: bool,
    repo_root: &Path,
) -> serde_json::Value {
    let outcome_default = ApplyOutcome::default();
    let oc = outcome.unwrap_or(&outcome_default);
    let migrated_legacy: Vec<serde_json::Value> = oc
        .legacy_dirs_migrated
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
    let migrations: Vec<serde_json::Value> = if !oc.legacy_dirs_migrated.is_empty() {
        migrated_legacy.clone()
    } else {
        report
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
            .collect()
    };

    let orphans: Vec<serde_json::Value> = report
        .orphan_epic_refs
        .iter()
        .map(|(s, e)| serde_json::json!({"slug": s, "epic": e}))
        .collect();

    let parse_errors: Vec<serde_json::Value> = report
        .parse_errors
        .iter()
        .map(|e| serde_json::json!({"location": e.location, "message": e.message}))
        .collect();

    let flat_layout_planned: Vec<serde_json::Value> = planned_moves(report)
        .iter()
        .map(|m| {
            serde_json::json!({
                "slug": m.slug(),
                "from": rel(repo_root, m.from()),
                "to": rel(repo_root, m.to()),
            })
        })
        .collect();
    let flat_layout_migrated: Vec<serde_json::Value> = oc
        .flat_layout_migrated
        .iter()
        .map(|m| {
            serde_json::json!({
                "slug": m.slug,
                "from": rel(repo_root, &m.from),
                "to": rel(repo_root, &m.to),
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
    let alias_coercions: Vec<serde_json::Value> = report
        .alias_coercions
        .iter()
        .map(|(slug, field, from, to, _)| {
            serde_json::json!({"slug": slug, "field": field, "from": from, "to": to})
        })
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
    let unknown_reviewers: Vec<serde_json::Value> = report
        .unknown_reviewers
        .iter()
        .map(|(s, r)| serde_json::json!({"slug": s, "reviewer": r}))
        .collect();
    let orphan_tempfiles: Vec<String> = report
        .orphan_tempfiles
        .iter()
        .map(|p| rel(repo_root, p))
        .collect();
    let orphan_tempfiles_removed: Vec<String> = oc
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

    let large_binaries: Vec<serde_json::Value> = report
        .large_binaries
        .iter()
        .map(|(slug, path, bytes)| serde_json::json!({"slug": slug, "path": path, "bytes": bytes}))
        .collect();
    let non_avif_images: Vec<serde_json::Value> = report
        .non_avif_images
        .iter()
        .map(|(slug, path)| serde_json::json!({"slug": slug, "path": path}))
        .collect();
    let broken_attachment_refs: Vec<serde_json::Value> = report
        .broken_attachment_refs
        .iter()
        .map(|(slug, r)| serde_json::json!({"slug": slug, "ref": r}))
        .collect();

    let mut json_obj = serde_json::json!({
        "fix_applied": fix && oc.fix_applied(),
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
        "alias_coercions": alias_coercions,
        "files_rewritten": oc.files_rewritten,
        "notes_to_rename": report.notes_to_rename,
        "notes_renamed": oc.notes_renamed,
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
        "status_reconciled": oc.status_reconciled,
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
        "agents_md_regenerated": oc.agents_md_regenerated,
        "agents_md_missing": report.agents_md_missing,
        "legacy_issues_agents_md": report.legacy_issues_agents_md,
        "issues_agents_md_rewritten": oc.issues_agents_md_rewritten,
        "gitignored_paths": report.gitignored_paths,
    });
    // Inserted post-construction rather than inline: the read-only object
    // literal is already at the `serde_json::json!` macro recursion
    // limit, so three more inline keys overflow it. Map is a sorted
    // BTreeMap, so insertion order does not affect the rendered output.
    if let serde_json::Value::Object(map) = &mut json_obj {
        map.insert(
            "large_binaries".to_string(),
            serde_json::Value::Array(large_binaries),
        );
        map.insert(
            "non_avif_images".to_string(),
            serde_json::Value::Array(non_avif_images),
        );
        map.insert(
            "broken_attachment_refs".to_string(),
            serde_json::Value::Array(broken_attachment_refs),
        );
        map.insert(
            "unknown_reviewers".to_string(),
            serde_json::Value::Array(unknown_reviewers),
        );
    }
    // `apply_outcome` is the new structured envelope: emitted only on
    // `--fix` runs so the read-only JSON shape (golden snapshot) stays
    // byte-identical. Carries `fix_applied` (computed from the outcome
    // alone — no early-return path can lie about it), the preflight
    // `blockers` list (which makes `--json --fix` a structured bail
    // instead of an anyhow text on stderr), and a rollup of every
    // applied-action variant for scripts that prefer reading one
    // sub-object instead of N top-level keys.
    if fix {
        if let serde_json::Value::Object(map) = &mut json_obj {
            map.insert(
                "apply_outcome".to_string(),
                serde_json::json!({
                    "fix_applied": oc.fix_applied(),
                    "stop_phase": oc.stop_phase.as_str(),
                    "blockers": oc.blockers,
                    "schema_bootstrapped": oc.schema_bootstrapped,
                    "agents_md_regenerated": oc.agents_md_regenerated,
                    "issues_agents_md_rewritten": oc.issues_agents_md_rewritten,
                    "files_rewritten": oc.files_rewritten,
                    "legacy_dirs_migrated": migrated_legacy,
                    "flat_layout_migrated": oc
                        .flat_layout_migrated
                        .iter()
                        .map(|m| {
                            serde_json::json!({
                                "slug": m.slug,
                                "from": rel(repo_root, &m.from),
                                "to": rel(repo_root, &m.to),
                            })
                        })
                        .collect::<Vec<_>>(),
                    "notes_renamed": oc.notes_renamed,
                    "notes_conflicts_at_apply": oc.notes_conflicts_at_apply,
                    "orphan_tempfiles_removed": oc
                        .orphan_tempfiles_removed
                        .iter()
                        .map(|p| rel(repo_root, p))
                        .collect::<Vec<_>>(),
                    "status_reconciled": oc.status_reconciled,
                    "alias_coercions_applied": oc
                        .alias_coercions_applied
                        .iter()
                        .map(|(slug, field, from, to)| {
                            serde_json::json!({"slug": slug, "field": field, "from": from, "to": to})
                        })
                        .collect::<Vec<_>>(),
                    "apply_error": oc.apply_error,
                }),
            );
        }
    }
    json_obj
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
    fn detect_gitignored_canonical_paths_flags_ignored_agents_md() {
        // Regression for #simply-workable-umbrella: a tempdir repo
        // with `.gitignore` masking `.issuectl/` should surface
        // `.issuectl/AGENTS.md` in gitignored_paths after `agents init`.
        let tmp = fresh_repo();
        // Bootstrap a real git repo so `git check-ignore` works.
        std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .arg("init")
            .arg("--quiet")
            .output()
            .expect("git init");
        // Mask the canonical issuectl files via .gitignore.
        fs::write(
            tmp.path().join(".gitignore"),
            ".issuectl/\nissues/.schema.yaml\n",
        )
        .unwrap();
        // Place the canonical files on disk.
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(tmp.path().join(".issuectl/AGENTS.md"), "# placeholder\n").unwrap();
        fs::write(tmp.path().join("issues/.schema.yaml"), "fields: {}\n").unwrap();

        let hits = detect_gitignored_canonical_paths(tmp.path());
        let joined = hits.join("\n");
        assert!(
            joined.contains(".issuectl/AGENTS.md"),
            "expected hit for .issuectl/AGENTS.md, got {hits:?}"
        );
        assert!(
            joined.contains("issues/.schema.yaml"),
            "expected hit for issues/.schema.yaml, got {hits:?}"
        );

        // Full doctor scan surfaces the warning in the report.
        let report = scan(tmp.path()).unwrap();
        assert!(
            !report.gitignored_paths.is_empty(),
            "expected gitignored_paths populated; got empty"
        );
    }

    #[test]
    fn detect_gitignored_canonical_paths_does_not_flag_tracked_files() {
        // Followup-review regression: with `--no-index`, git reports
        // tracked-but-pattern-matched files as ignored, producing
        // false positives. Without `--no-index` (current behavior),
        // a tracked file is correctly considered visible to teammates
        // even when `.gitignore` would otherwise have masked it.
        let tmp = fresh_repo();
        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["init", "--quiet"])
            .output()
            .expect("git init");
        assert!(init.status.success());
        // First track the file, THEN add the ignore rule that would
        // have masked it. Tracked files are not ignored from git's
        // perspective.
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(tmp.path().join(".issuectl/AGENTS.md"), "# x\n").unwrap();
        let add = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["add", "-f", ".issuectl/AGENTS.md"])
            .output()
            .expect("git add");
        assert!(add.status.success(), "git add failed: {add:?}");
        // Configure user.email/name so commit can succeed even on a
        // CI host without global git config.
        for (k, v) in [
            ("user.email", "test@example.invalid"),
            ("user.name", "test"),
        ] {
            std::process::Command::new("git")
                .arg("-C")
                .arg(tmp.path())
                .args(["config", k, v])
                .output()
                .expect("git config");
        }
        let commit = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["commit", "--quiet", "-m", "track AGENTS.md"])
            .output()
            .expect("git commit");
        assert!(commit.status.success(), "git commit failed: {commit:?}");
        fs::write(tmp.path().join(".gitignore"), ".issuectl/\n").unwrap();

        let hits = detect_gitignored_canonical_paths(tmp.path());
        assert!(
            hits.is_empty(),
            "tracked file must NOT be flagged as gitignored; got {hits:?}"
        );
    }

    #[test]
    fn detect_gitignored_canonical_paths_silent_when_not_a_git_repo() {
        // Bare tempdir (no `git init`) — `git check-ignore` returns
        // exit 128. Doctor must not crash and must report no hits.
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(tmp.path().join(".issuectl/AGENTS.md"), "# x\n").unwrap();
        let hits = detect_gitignored_canonical_paths(tmp.path());
        assert!(hits.is_empty(), "got {hits:?}");
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
            "---\nnumber: 1\nstatus: open\nepic: 2\nrelated: [\"#3\"]\nblocked_by: [\"#3\"]\n---\n# E1. First\n",
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
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(outcome.fix_applied());
        assert!(
            outcome.blockers.is_empty(),
            "blockers={:?}",
            outcome.blockers
        );
        // Find the migrated 1-first directory.
        let mig1 = outcome
            .legacy_dirs_migrated
            .iter()
            .find(|m| m.old_number == 1)
            .unwrap();
        let item = mig1.new_path.join("item.md");
        let content = fs::read_to_string(&item).unwrap();
        assert!(content.contains(&format!("slug: {}", mig1.new_slug)));
        assert!(!content.contains("number:"));
        assert!(content.contains("# First"), "heading rewritten: {content}");
        // epic: 2 → epic: <slug-of-2>
        let mig2 = outcome
            .legacy_dirs_migrated
            .iter()
            .find(|m| m.old_number == 2)
            .unwrap();
        assert!(content.contains(&format!("epic: {}", mig2.new_slug)));
        // related: ['#3'] → ['@<slug-of-3>'], blocked_by: ['#3'] → ['@<slug-of-3>']
        let mig3 = outcome
            .legacy_dirs_migrated
            .iter()
            .find(|m| m.old_number == 3)
            .unwrap();
        let parsed = write::read_item(&item).unwrap();
        let expected = serde_yaml::Value::Sequence(vec![serde_yaml::Value::String(format!(
            "@{}",
            mig3.new_slug
        ))]);
        for key in ["related", "blocked_by"] {
            let got = parsed
                .frontmatter
                .get(serde_yaml::Value::String(key.into()))
                .unwrap_or_else(|| panic!("`{key}` missing after rewrite: {content}"));
            assert_eq!(got, &expected, "`{key}` not migrated: {content}");
        }
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
    fn rewrite_text_skips_inline_code_spans() {
        // Inline code is documentation, not a live reference: a
        // user explaining `the old #7 syntax` doesn't want it
        // silently rewritten to `the old @amber-loud-fox syntax`.
        let mut nm = BTreeMap::new();
        nm.insert(7, "amber-loud-fox".to_string());
        let dm = BTreeMap::new();
        let amb = BTreeSet::new();
        let text = "use `#7` literally, but rewrite #7 here.\n";
        let out = rewrite_text(text, &nm, &dm, &amb);
        assert_eq!(
            out,
            "use `#7` literally, but rewrite @amber-loud-fox here.\n"
        );
    }

    #[test]
    fn rewrite_text_still_rewrites_paths_inside_link_urls() {
        // Doctor intentionally rewrites intra-repo paths inside link
        // URLs — that's the whole point of the dir-rename step.
        // (Contrast `refs::rewrite_body_refs`, which DOES skip URLs.)
        let nm = BTreeMap::new();
        let mut dm = BTreeMap::new();
        dm.insert("7-something".to_string(), "amber-loud-fox".to_string());
        let amb = BTreeSet::new();
        let text = "see [link](../7-something/item.md).\n";
        let out = rewrite_text(text, &nm, &dm, &amb);
        assert!(
            out.contains("../amber-loud-fox/item.md"),
            "link-URL path must still be rewritten by doctor: {out:?}"
        );
    }

    #[test]
    fn migrate_notes_heading_renames_outside_fences() {
        let body =
            "---\nstatus: open\n---\n\n# T\n\n## Notes\n\nfirst\n\n```\n## not a heading\n```\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(!conflict);
        assert!(out.contains("## Comments"));
        assert!(!out.contains("## Notes\n"), "Notes heading must be renamed");
        // The fenced `## not a heading` is content and stays put.
        assert!(out.contains("```\n## not a heading\n```"));
    }

    #[test]
    fn migrate_notes_heading_merges_when_both_exist() {
        // Issue @doctor-fix-merge-notes-comments: one `## Notes` and one
        // `## Comments` auto-merge (no manual conflict). `## Notes`
        // preceded `## Comments`, so its entry lands first (document
        // order preserved) and `## Notes` is dropped.
        let body = "## Notes\n\nx\n\n## Comments\n\ny\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(!conflict, "both-exist is auto-merged, not a conflict");
        assert_eq!(out, "## Comments\n\nx\n\ny\n");
        assert!(!out.contains("## Notes"), "## Notes must be dropped");
    }

    #[test]
    fn migrate_notes_heading_merge_preserves_document_order_notes_after() {
        // When `## Comments` precedes `## Notes`, the Comments entries
        // stay first and the Notes entries are appended — document
        // order is preserved regardless of which section came first.
        let body = "## Comments\n\ny\n\n## Notes\n\nx\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(!conflict);
        assert_eq!(out, "## Comments\n\ny\n\nx\n");
    }

    #[test]
    fn migrate_notes_heading_merge_preserves_intervening_section() {
        // A section between `## Notes` and `## Comments` is preserved in
        // place; only `## Notes` is folded away.
        let body = "## Notes\n\nx\n\n## Decisions\n\nd\n\n## Comments\n\ny\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(!conflict);
        assert_eq!(out, "## Decisions\n\nd\n\n## Comments\n\nx\n\ny\n");
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
        // One `## Notes` + one `## Comments` is now an auto-merge, so it
        // joins `notes_to_rename`, not `notes_conflicts`.
        let merge = tmp.path().join("issues/has-both");
        fs::create_dir_all(&merge).unwrap();
        fs::write(
            merge.join("item.md"),
            "---\nstatus: open\n---\n\n## Notes\n\nx\n\n## Comments\n\ny\n",
        )
        .unwrap();
        // Multiple `## Notes` stays an ambiguous conflict.
        let conflict = tmp.path().join("issues/two-notes");
        fs::create_dir_all(&conflict).unwrap();
        fs::write(
            conflict.join("item.md"),
            "---\nstatus: open\n---\n\n## Notes\n\na\n\n## Decisions\n\nd\n\n## Notes\n\nb\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        let mut to_rename = r.notes_to_rename.clone();
        to_rename.sort();
        assert_eq!(
            to_rename,
            vec!["has-both".to_string(), "safe-rename".to_string()]
        );
        assert_eq!(r.notes_conflicts, vec!["two-notes".to_string()]);
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
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after.contains("## Comments"));
        assert!(!after.contains("## Notes"));
        assert!(after.contains("old note"));
        assert_eq!(outcome.notes_renamed, vec!["legacy-notes-here".to_string()]);
    }

    #[test]
    fn doctor_fix_merges_notes_into_comments_when_both_exist() {
        // Issue @doctor-fix-merge-notes-comments: a body with BOTH
        // `## Notes` and `## Comments` is auto-merged by `--fix`
        // (document order preserved, `## Notes` dropped) — it no longer
        // surfaces as a manual-merge conflict, so nothing lands in
        // `notes_conflicts_at_apply` and the apply completes cleanly.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/has-both");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T\n\n## Notes\n\nfirst\n\n## Comments\n\nsecond\n",
        )
        .unwrap();
        let mut r = scan(tmp.path()).unwrap();
        assert!(
            r.notes_conflicts.is_empty(),
            "both-exist must not be a scan conflict, got {:?}",
            r.notes_conflicts
        );
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(
            !after.contains("## Notes"),
            "## Notes must be dropped: {after}"
        );
        assert_eq!(after.matches("## Comments").count(), 1, "single Comments");
        // Document order preserved: the Notes entry precedes the
        // existing Comments entry.
        let first = after.find("first").expect("Notes entry retained");
        let second = after.find("second").expect("Comments entry retained");
        assert!(first < second, "Notes entry must come first: {after}");
        assert_eq!(outcome.notes_renamed, vec!["has-both".to_string()]);
        assert!(
            outcome.notes_conflicts_at_apply.is_empty(),
            "no manual-merge leftovers: {:?}",
            outcome.notes_conflicts_at_apply
        );
        // A second doctor run is a clean no-op (idempotent merge) and
        // leaves the file byte-for-byte unchanged.
        let mut r2 = scan(tmp.path()).unwrap();
        assert!(r2.notes_to_rename.is_empty() && r2.notes_conflicts.is_empty());
        let actions2 = DoctorActions::from_findings(&mut r2);
        apply(
            tmp.path(),
            actions2,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let after_second = fs::read_to_string(dir.join("item.md")).unwrap();
        assert_eq!(after, after_second, "second --fix must not mutate the file");
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
        let actions = DoctorActions::from_findings(&mut r);
        apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
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
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let path = tmp.path().join("issues/.schema.yaml");
        assert!(path.is_file(), "schema file should be auto-written");
        // Should contain the canonical built-in fields.
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("type:"));
        assert!(content.contains("status:"));
        // Bug #3: schema bootstrap must surface in `fix_applied`.
        // Previously a `--fix` that only wrote `.schema.yaml` reported
        // `fix_applied: false`; with `ApplyOutcome::schema_bootstrapped`
        // pulled into the predicate, it now reports `true`.
        assert!(
            outcome.schema_bootstrapped,
            "expected schema bootstrap to be recorded"
        );
        assert!(
            outcome.fix_applied(),
            "schema-only --fix must report fix_applied=true"
        );
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
            r.parse_errors.iter().any(|e| e.message.contains("YAML")
                || e.message.contains("yaml")
                || e.message.contains("invalid")),
            "expected parse error report, got {:?}",
            r.parse_errors
        );
        // Bug #6: hard parse errors are typed at the source — no
        // substring matching. Re-wording the parser message no longer
        // reclassifies a hard fail as a soft warn.
        assert!(
            r.parse_errors
                .iter()
                .any(|e| e.severity == ParseSeverity::Hard),
            "unparseable frontmatter must classify as Hard: {:?}",
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
        // shape. `issuectl new` writes a `slug:` field for new issues,
        // and that `slug:` is the typed signal that suppresses the
        // numbered-legacy classification — without it, doctor would
        // queue this dir for the NN-rename pipeline and skip schema
        // checks. With `slug:` present, schema validation must run.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/12-things-to-do");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\nslug: 12-things-to-do\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.legacy_dirs.is_empty(),
            "modern flat issue with `slug:` must not be queued for NN-rename, got {:?}",
            r.legacy_dirs
        );
        assert!(
            r.schema_violations
                .iter()
                .any(|(_, msg)| msg.contains("priority")),
            "expected violation on flat NN-shaped slug, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn flat_issue_with_legacy_shape_name_and_no_slug_field_is_legacy() {
        // Mirror image of the test above: a flat-layout dir whose
        // name matches `<NN>-<slug>` but whose frontmatter omits the
        // `slug:` field is classified legacy and queued for NN-rename.
        // This is the canonical "old hand-authored issue" case — the
        // user's intended dir name is canonicalised by `--fix`.
        // Lints (schema/refs/timestamps/...) are SUPPRESSED on this
        // dir because `--fix` rewrites its frontmatter wholesale;
        // surfacing them would refuse the very fix designed to heal
        // them.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/12-things-to-do");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert_eq!(
            r.legacy_dirs.len(),
            1,
            "flat NN-shape with no `slug:` must be classified legacy, got {:?}",
            r.legacy_dirs
        );
        assert_eq!(r.legacy_dirs[0].old_number, 12);
        assert!(
            r.schema_violations.is_empty(),
            "lints must be suppressed for legacy-classified dirs, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn stray_number_field_with_slug_does_not_classify_modern_issue_as_legacy() {
        // Regression: a modern flat issue carrying `slug:` AND a stray
        // `number:` field (left over from a botched manual edit) must
        // NOT be classified legacy. `slug:` short-circuits before
        // `number:` in `legacy_number_from_mapping`, so this dir keeps
        // its name and gets full schema validation.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\nslug: quiet-brave-otter\nnumber: 7\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.legacy_dirs.is_empty(),
            "modern issue with stray `number:` must not be queued for NN-rename, got {:?}",
            r.legacy_dirs
        );
    }

    fn put_flat(tmp: &TempDir, slug: &str, body: &str) {
        let dir = tmp.path().join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), body).unwrap();
    }

    #[test]
    fn unknown_reviewer_is_flagged_unless_a_known_user_elsewhere() {
        // alice is the assignee of issue-one → known user.
        // bob is only ever referenced as a reviewer → flagged.
        // alice as reviewer on issue-two → accepted.
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "alpha-known-user",
            "---\ntype: bug\nstatus: open\npriority: normal\nassignee: alice\n---\n# One\n",
        );
        put_flat(
            &tmp,
            "beta-known-reviewer",
            "---\ntype: bug\nstatus: open\npriority: normal\nreviewer: alice\nreview_status: requested\n---\n# Two\n",
        );
        put_flat(
            &tmp,
            "gamma-unknown-reviewer",
            "---\ntype: bug\nstatus: open\npriority: normal\nreviewer: bob\nreview_status: in-review\n---\n# Three\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.unknown_reviewers
                .iter()
                .any(|(slug, who)| slug == "gamma-unknown-reviewer" && who == "bob"),
            "expected bob flagged, got {:?}",
            r.unknown_reviewers
        );
        assert!(
            !r.unknown_reviewers.iter().any(|(_, who)| who == "alice"),
            "alice is a known user; must not be flagged: {:?}",
            r.unknown_reviewers
        );
        // review_status must not show up under unknown_keys — the schema
        // declares it.
        assert!(
            !r.unknown_keys
                .iter()
                .any(|(_, k)| k == "review_status" || k == "reviewer"),
            "reviewer/review_status are schema-known: {:?}",
            r.unknown_keys
        );
    }

    #[test]
    fn flags_custom_closing_status_without_closed_field() {
        // Schema declares `archived` as closing. An issue at
        // `status: archived` without a `closed:` date must be flagged
        // by status_consistency, just like a built-in `done` would be.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\n    enum: [open, archived]\nstatus_classes:\n  archived: closing\n",
        )
        .unwrap();
        put_flat(
            &tmp,
            "alpha-issue-here",
            "---\ntype: bug\nstatus: archived\npriority: normal\n---\n# A\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.status_consistency
                .iter()
                .any(|(slug, msg)| slug == "alpha-issue-here" && msg.contains("archived")),
            "expected archived-without-closed flagged, got {:?}",
            r.status_consistency
        );
    }

    #[test]
    fn flags_custom_active_status_carrying_closed_field() {
        // Schema declares `verified` as active. An issue with
        // `status: verified` AND `closed: <date>` must be flagged —
        // active statuses must not carry `closed:`.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\n    enum: [open, verified]\nstatus_classes:\n  verified: active\n",
        )
        .unwrap();
        put_flat(
            &tmp,
            "beta-issue-here",
            "---\ntype: bug\nstatus: verified\nclosed: 2026-05-06\npriority: normal\n---\n# B\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.status_consistency
                .iter()
                .any(|(slug, msg)| slug == "beta-issue-here" && msg.contains("verified")),
            "expected verified-with-closed flagged, got {:?}",
            r.status_consistency
        );
    }

    #[test]
    fn flags_active_status_carrying_closed_by() {
        // A `closed_by:` on an active issue is self-inconsistent (the
        // close path scrubs it on the active edge), so doctor flags it
        // alongside the `closed:` heal — even when `closed:` itself is
        // already absent.
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "stranded-closer-here",
            "---\ntype: bug\nstatus: open\npriority: normal\nclosed_by: jari\n---\n# S\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.status_consistency
                .iter()
                .any(|(slug, msg)| slug == "stranded-closer-here" && msg.contains("closed_by")),
            "expected stranded closed_by flagged, got {:?}",
            r.status_consistency
        );
    }

    #[test]
    fn read_only_detects_status_alias_and_suppresses_enum_violation() {
        // A legacy `status: closed` value (built-in alias → done) must
        // show up as a pending coercion, NOT as an enum schema
        // violation (the coercion is the fix).
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "legacy-status-issue",
            "---\ntype: bug\nstatus: closed\npriority: normal\n---\n# L\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.alias_coercions
                .iter()
                .any(|(slug, field, from, to, _)| slug == "legacy-status-issue"
                    && field == "status"
                    && from == "closed"
                    && to == "done"),
            "expected status closed→done coercion, got {:?}",
            r.alias_coercions
        );
        assert!(
            !r.schema_violations
                .iter()
                .any(|(_, msg)| msg.contains("\"closed\"") && msg.contains("status")),
            "aliasable status must not be reported as an enum violation, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn fix_coerces_status_alias_and_stamps_closed() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "legacy-status-issue",
            "---\ntype: bug\nstatus: closed\npriority: normal\n---\n# L\n",
        );
        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(
            outcome
                .alias_coercions_applied
                .iter()
                .any(|(slug, field, from, to)| slug == "legacy-status-issue"
                    && field == "status"
                    && from == "closed"
                    && to == "done"),
            "expected applied coercion, got {:?}",
            outcome.alias_coercions_applied
        );
        let after =
            fs::read_to_string(tmp.path().join("issues/legacy-status-issue/item.md")).unwrap();
        assert!(
            after.contains("status: done"),
            "status not coerced:\n{after}"
        );
        // `done` is a closing status, so `closed:` must be backfilled.
        assert!(
            after.contains("closed:"),
            "closed: not stamped on coerced closing status:\n{after}"
        );
    }

    #[test]
    fn fix_coerces_type_alias_without_touching_closed() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "legacy-type-issue",
            "---\ntype: enhancement\nstatus: open\npriority: normal\n---\n# T\n",
        );
        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(
            outcome
                .alias_coercions_applied
                .iter()
                .any(|(_, field, from, to)| field == "type"
                    && from == "enhancement"
                    && to == "improvement"),
            "expected type coercion, got {:?}",
            outcome.alias_coercions_applied
        );
        let after =
            fs::read_to_string(tmp.path().join("issues/legacy-type-issue/item.md")).unwrap();
        assert!(
            after.contains("type: improvement"),
            "type not coerced:\n{after}"
        );
        // Active status → no `closed:` stamped.
        assert!(
            !after.contains("closed:"),
            "closed: must not appear:\n{after}"
        );
    }

    #[test]
    fn fix_stamps_closed_from_git_commit_date_not_today() {
        // A legacy issue closed long ago: coercing `status: closed` →
        // `done` must backfill `closed:` from the file's last git commit
        // date, not today() — otherwise a years-old issue gets a brand
        // new closed date.
        let tmp = fresh_repo();
        let git = |args: &[&str]| {
            let st = std::process::Command::new("git")
                .arg("-C")
                .arg(tmp.path())
                .args(args)
                .output()
                .expect("git");
            assert!(st.status.success(), "git {args:?} failed");
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "Test"]);
        put_flat(
            &tmp,
            "ancient-closed-issue",
            "---\ntype: bug\nstatus: closed\npriority: normal\n---\n# A\n",
        );
        git(&["add", "."]);
        // Pin BOTH author and committer date so `%aI` is deterministic.
        std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args([
                "commit",
                "--quiet",
                "-m",
                "import",
                "--date=2020-01-15T12:00:00",
            ])
            .env("GIT_COMMITTER_DATE", "2020-01-15T12:00:00")
            .output()
            .expect("git commit");

        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let after =
            fs::read_to_string(tmp.path().join("issues/ancient-closed-issue/item.md")).unwrap();
        assert!(
            after.contains("status: done"),
            "status not coerced:\n{after}"
        );
        assert!(
            after.contains("closed: 2020-01-15"),
            "expected closed date from git commit, got:\n{after}"
        );
    }

    #[test]
    fn derive_closed_date_falls_back_to_mtime_when_untracked() {
        // Not a git repo (no .git): git_last_commit_date returns None,
        // so the mtime fallback supplies a valid YYYY-MM-DD.
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "untracked-issue",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# U\n",
        );
        let path = tmp.path().join("issues/untracked-issue/item.md");
        let date = derive_closed_date(&path);
        assert!(
            chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_ok(),
            "expected a valid YYYY-MM-DD, got {date:?}"
        );
        // A file just created in this test has an mtime of ~now, so the
        // mtime tier should resolve to today (both use local time).
        assert_eq!(date, write::today(), "mtime fallback should be today");
    }

    #[test]
    fn fix_batches_status_and_type_coercions_for_one_issue() {
        // An issue with BOTH a status and a type coercion is read once
        // and written once. The behavioral proof: both fields land
        // correctly in the same file and `closed:` is stamped exactly
        // once (a per-field read+write could double-stamp or clobber).
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "double-coercion-issue",
            "---\ntype: enhancement\nstatus: closed\npriority: normal\n---\n# D\n",
        );
        let mut r = scan(tmp.path()).unwrap();
        // Scan must plan both coercions for the one issue.
        let planned: Vec<_> = r
            .alias_coercions
            .iter()
            .filter(|(slug, ..)| slug == "double-coercion-issue")
            .map(|(_, field, ..)| field.clone())
            .collect();
        assert!(
            planned.contains(&"status".to_string()) && planned.contains(&"type".to_string()),
            "expected both status+type coercions planned, got {planned:?}"
        );

        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let applied_fields: Vec<_> = outcome
            .alias_coercions_applied
            .iter()
            .filter(|(slug, ..)| slug == "double-coercion-issue")
            .map(|(_, field, ..)| field.clone())
            .collect();
        assert!(
            applied_fields.contains(&"status".to_string())
                && applied_fields.contains(&"type".to_string()),
            "expected both coercions applied, got {applied_fields:?}"
        );

        let after =
            fs::read_to_string(tmp.path().join("issues/double-coercion-issue/item.md")).unwrap();
        assert!(
            after.contains("status: done"),
            "status not coerced:\n{after}"
        );
        assert!(
            after.contains("type: improvement"),
            "type not coerced:\n{after}"
        );
        assert_eq!(
            after.matches("closed:").count(),
            1,
            "closed: must be stamped exactly once:\n{after}"
        );
    }

    #[test]
    fn reconciliation_stamps_closed_from_git_commit_date() {
        // Status/folder reconciliation (closed/<slug> carrying an active
        // status) must also derive its backfilled `closed:` from git
        // history rather than today().
        let tmp = fresh_repo();
        let git = |args: &[&str]| {
            let st = std::process::Command::new("git")
                .arg("-C")
                .arg(tmp.path())
                .args(args)
                .output()
                .expect("git");
            assert!(st.status.success(), "git {args:?} failed");
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "Test"]);
        // Legacy layout: an active status sitting under issues/closed/.
        let dir = tmp.path().join("issues/closed/legacy-folder-issue");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: in-progress\npriority: normal\n---\n# F\n",
        )
        .unwrap();
        git(&["add", "."]);
        std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args([
                "commit",
                "--quiet",
                "-m",
                "import",
                "--date=2019-06-10T09:00:00",
            ])
            .env("GIT_COMMITTER_DATE", "2019-06-10T09:00:00")
            .output()
            .expect("git commit");

        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        // After flat-layout migration the file lives at issues/<slug>/.
        let after =
            fs::read_to_string(tmp.path().join("issues/legacy-folder-issue/item.md")).unwrap();
        assert!(
            after.contains("closed: 2019-06-10"),
            "expected reconciled closed date from git, got:\n{after}"
        );
    }

    #[test]
    fn custom_required_when_field_surfaces_as_schema_violation() {
        // Regression: doctor must NOT swallow a user-declared
        // `required_when` on a field other than `closed` — only the
        // built-in closed/closing rule is suppressed (it has a separate
        // reporting channel). A custom `resolution` required-when-closing
        // field has no other channel, so a missing value must show up in
        // `schema_violations`.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  resolution:\n    required_when:\n      status_class: closing\n",
        )
        .unwrap();
        put_flat(
            &tmp,
            "needs-resolution",
            "---\ntype: bug\nstatus: done\npriority: normal\nclosed: 2026-05-06\n---\n# R\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.schema_violations
                .iter()
                .any(|(loc, msg)| loc.contains("needs-resolution") && msg.contains("resolution")),
            "custom required_when must surface in schema_violations, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn value_valid_in_user_enum_is_not_coerced() {
        // A repo that adds a built-in alias KEY to its own status enum
        // makes that value canonical — it must not be silently coerced.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\n    enum: [open, in-progress, resolved, done]\n",
        )
        .unwrap();
        put_flat(
            &tmp,
            "keeps-resolved",
            "---\ntype: bug\nstatus: resolved\npriority: normal\n---\n# K\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.alias_coercions.is_empty(),
            "value present in the user enum must not be coerced, got {:?}",
            r.alias_coercions
        );
        assert!(
            !r.schema_violations
                .iter()
                .any(|(loc, _)| loc.contains("keeps-resolved")),
            "a canonical (enum-valid) value must not be flagged, got {:?}",
            r.schema_violations
        );
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
                .any(|(s, k, t)| s == "quiet-brave-otter"
                    && k == "epic"
                    && t == "nonexistent-ghost-fox"),
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
        assert!(
            r.conflict_markers.is_empty(),
            "got {:?}",
            r.conflict_markers
        );
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
    fn detects_blocked_by_self_dependency() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "self-loop-target",
            "---\ntype: bug\nstatus: open\npriority: normal\nblocked_by: ['@self-loop-target']\n---\n# S\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.blocked_by_self, vec!["self-loop-target".to_string()]);
        // The 1-node "cycle" must not also be reported as a cycle:
        // the self-dep branch claims it as its own finding so the user
        // gets a focused message.
        assert!(
            r.blocked_by_cycles.is_empty(),
            "self-dep should be deduped from cycle list: {:?}",
            r.blocked_by_cycles
        );
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
        assert!(
            r.status_consistency.is_empty(),
            "{:?}",
            r.status_consistency
        );
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
        assert!(r.timestamp_issues.iter().any(|(_, m)| m.contains("future")));
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
        let actions = DoctorActions::from_findings(&mut r);
        // Bug #1: preflight blockers MUST NOT bail — they ride on
        // ApplyOutcome.blockers so `--json --fix` consumers receive
        // structured output instead of an anyhow stderr blob.
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(!outcome.blockers.is_empty(), "expected preflight blockers");
        let joined = outcome.blockers.join("\n");
        assert!(
            joined.contains("BOTH"),
            "missing both-folders blocker: {joined}"
        );
        assert!(
            joined.contains("merge-conflict markers"),
            "missing conflict-marker blocker: {joined}"
        );
        // Schema bootstrap fires unconditionally before preflight
        // refusal (issue: @unreasonably-attractive-star), so a fresh
        // repo always reports `schema_bootstrapped: true` even on the
        // preflight-blocked path. The contract is that NO OTHER write
        // landed — the preflight blockers still gate every other
        // phase.
        assert!(
            outcome.schema_bootstrapped,
            "schema bootstrap is unconditional, must run even on preflight bail"
        );
        assert!(
            outcome.legacy_dirs_migrated.is_empty()
                && outcome.flat_layout_migrated.is_empty()
                && outcome.notes_renamed.is_empty()
                && outcome.orphan_tempfiles_removed.is_empty()
                && outcome.status_reconciled.is_empty()
                && outcome.files_rewritten == 0
                && !outcome.agents_md_regenerated
                && !outcome.issues_agents_md_rewritten,
            "preflight-blocked apply must not run any phase beyond schema bootstrap"
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
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .expect("--fix should not refuse on soft parse warnings");
        assert!(
            outcome.blockers.is_empty(),
            "soft parse warnings must not block: {:?}",
            outcome.blockers
        );
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
        let before =
            fs::read_to_string(tmp.path().join("issues/quiet-brave-otter/item.md")).unwrap();
        // Preflight blocks before any mutation against the conflict
        // file — but produces a structured `ApplyOutcome.blockers`
        // rather than an Err. Schema bootstrap can land (it precedes
        // preflight, see @unreasonably-attractive-star); the conflict
        // file itself MUST NOT be touched.
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(
            outcome
                .blockers
                .iter()
                .any(|b| b.contains("merge-conflict markers")),
            "got: {:?}",
            outcome.blockers
        );
        let after =
            fs::read_to_string(tmp.path().join("issues/quiet-brave-otter/item.md")).unwrap();
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
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(!orphan.exists(), "tempfile should be removed by --fix");
        assert!(outcome
            .orphan_tempfiles_removed
            .iter()
            .any(|p| p == &orphan));
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
        let actions = DoctorActions::from_findings(&mut r);
        apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
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
        let actions = DoctorActions::from_findings(&mut r);
        apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let migrated = tmp.path().join("issues/quiet-brave-otter/item.md");
        let after = fs::read_to_string(&migrated).unwrap();
        assert!(after.contains("status: open"), "got: {after}");
        assert!(
            !after.contains("closed:"),
            "closed should be dropped: {after}"
        );
    }

    /// Issue @doctor-fix-noop: notes/comments conflicts are NOT
    /// preflight blockers. They surface in `outcome.notes_conflicts_at_apply`
    /// and let other phases (NN-rename, alias coercion, AGENTS.md
    /// regen) run normally. The post-apply rescan still picks up the
    /// conflict via `findings.notes_conflicts` so the user sees it.
    #[test]
    fn post_flat_layout_notes_conflict_surfaces_without_blocking() {
        let tmp = fresh_repo();
        let foo = tmp.path().join("issues/open/foo-bar");
        fs::create_dir_all(&foo).unwrap();
        // Multiple `## Notes` — an ambiguous shape that stays a manual
        // conflict (the unambiguous both-exist case now auto-merges).
        fs::write(
            foo.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-01-01\n---\n# T\n\n## Notes\n\nfirst\n\n## Notes\n\nsecond\n",
        )
        .unwrap();
        let old = tmp.path().join("issues/closed/3-old");
        fs::create_dir_all(&old).unwrap();
        fs::write(
            old.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# Old\n",
        )
        .unwrap();

        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        assert_eq!(
            outcome.stop_phase,
            StopPhase::Ok,
            "notes/comments conflict must not bail the pipeline: {outcome:?}"
        );
        assert!(
            outcome.blockers.is_empty(),
            "no blockers: {:?}",
            outcome.blockers
        );
        assert_eq!(outcome.flat_layout_migrated.len(), 2);
        assert!(
            outcome
                .notes_conflicts_at_apply
                .iter()
                .any(|s| s == "foo-bar"),
            "conflict must surface in notes_conflicts_at_apply, got {:?}",
            outcome.notes_conflicts_at_apply
        );
        // NN-rename of `3-old` MUST run despite the unrelated
        // notes conflict (the whole point of this fix).
        assert_eq!(
            outcome.legacy_dirs_migrated.len(),
            1,
            "NN-rename must proceed despite an unrelated notes conflict"
        );
        // Post-fix scan still surfaces the conflict so the user
        // sees it as forward work (drives exit 1 via critical_blockers).
        let after = scan(tmp.path()).unwrap();
        assert!(
            after.notes_conflicts.iter().any(|s| s == "foo-bar"),
            "post-fix scan must still report the conflict"
        );
        let decision = classify_exit(&after, Some(&outcome), true);
        assert_eq!(decision.code, 1);
        assert_eq!(decision.error_code, "doctor-partial");
    }

    #[test]
    fn apply_renames_notes_for_pre_migration_legacy_folder_in_one_pass() {
        // Regression: `## Notes` in a body still under
        // `issues/open/<slug>/` is invisible to the pre-migration
        // scan (`populate_notes_migration` walks only flat-folder
        // dirs). The phase-3 rename in `apply` therefore did
        // nothing for this issue, and the user had to invoke
        // `doctor --fix` a second time. After the post-migration
        // re-scan now feeds `rename_notes_to_comments`, a single
        // `--fix` invocation must lift the dir AND rename the
        // heading.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/legacy-notes-slug");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-01-01\n---\n# T\n\n## Notes\n\nhello\n",
        )
        .unwrap();

        let mut r = scan(tmp.path()).unwrap();
        assert!(
            r.notes_to_rename.is_empty(),
            "pre-migration scan must not see the legacy-folder Notes heading, got {:?}",
            r.notes_to_rename
        );
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        let migrated = tmp.path().join("issues/legacy-notes-slug/item.md");
        assert!(migrated.is_file(), "flat-layout migration must run");
        let after = fs::read_to_string(&migrated).unwrap();
        assert!(
            after.contains("## Comments"),
            "## Notes must be renamed in the same --fix pass, got: {after}"
        );
        assert!(
            !after.contains("## Notes\n"),
            "## Notes heading must be gone, got: {after}"
        );
        assert_eq!(
            outcome.notes_renamed,
            vec!["legacy-notes-slug".to_string()],
            "outcome must record the post-migration rename"
        );
    }

    #[test]
    fn apply_renames_notes_for_numbered_legacy_folder_in_one_pass() {
        // Companion to the slug-named regression above: verify the
        // post-migration rename also fires for a numbered-legacy
        // dir that lived under `issues/open/` and goes through
        // BOTH flat-layout migration AND NN-rename in the same
        // `--fix` pass. The body must end up at the canonical slug
        // path with `## Comments`. We intentionally do not assert
        // on `outcome.notes_renamed` slug identity here — that's a
        // pre-existing reporting skew tracked separately.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/3-foo-bar");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-01-01\n---\n# T\n\n## Notes\n\nhello\n",
        )
        .unwrap();

        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        assert_eq!(
            outcome.legacy_dirs_migrated.len(),
            1,
            "NN-rename must run on the lifted numbered-legacy dir"
        );
        let new_slug = &outcome.legacy_dirs_migrated[0].new_slug;
        let final_item = tmp.path().join("issues").join(new_slug).join("item.md");
        assert!(
            final_item.is_file(),
            "expected file at canonical slug path, got missing: {}",
            final_item.display()
        );
        let after = fs::read_to_string(&final_item).unwrap();
        assert!(
            after.contains("## Comments"),
            "## Notes must be renamed at the canonical slug location, got: {after}"
        );
        assert!(
            !after.contains("## Notes\n"),
            "## Notes heading must be gone, got: {after}"
        );
    }

    #[test]
    fn apply_preserves_partial_flat_layout_migration_on_mid_loop_failure() {
        // Phase-5 mid-loop failure must surface as `Ok(outcome)` with
        // `flat_layout_migrated` carrying the move(s) that landed and
        // `apply_error` carrying the failure cause — NOT propagate as
        // `Err` (which would strand the partial progress inside an
        // anyhow text blob and bypass `--json` consumers).
        let tmp = fresh_repo();
        // Two flat-eligible issues. Slugs sorted alphabetically by the
        // BTreeMap inside `plan_migrate_layout` → `aaa-foo` is move #1,
        // `zzz-bar` is move #2.
        for slug in ["aaa-foo", "zzz-bar"] {
            let dir = tmp.path().join("issues/open").join(slug);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
            )
            .unwrap();
        }

        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);

        // Sabotage move #2 *after* the planner has classified the
        // moves: planting a regular file at the destination of the
        // second rename. `fs::rename(<dir>, <regular_file>)` returns
        // ENOTDIR on Unix / equivalent on Windows. Pre-creating before
        // planning would have been caught by `plan_migrate_layout`'s
        // `symlink_metadata` conflict check; doing it after lets the
        // failure surface inside `execute_migrate_layout_plan`.
        fs::write(tmp.path().join("issues/zzz-bar"), "blocker\n").unwrap();

        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .expect("apply must return Ok with partial outcome, not Err");

        assert_eq!(
            outcome.flat_layout_migrated.len(),
            1,
            "first move should have landed before the failure, got {:?}",
            outcome.flat_layout_migrated
        );
        assert_eq!(outcome.flat_layout_migrated[0].slug, "aaa-foo");
        assert!(
            tmp.path().join("issues/aaa-foo/item.md").is_file(),
            "first move should be visible on disk"
        );
        let err_msg = outcome
            .apply_error
            .as_ref()
            .expect("apply_error must carry the failure cause");
        assert!(
            err_msg.contains("zzz-bar") || err_msg.contains("rename"),
            "apply_error should mention the failed rename, got {err_msg:?}"
        );

        // The structured `--json --fix` envelope must echo both pieces
        // so scripts can recover without parsing stderr.
        let json = render_json(&scan(tmp.path()).unwrap(), Some(&outcome), true, tmp.path());
        let envelope = json
            .get("apply_outcome")
            .expect("apply_outcome present on --fix runs");
        assert_eq!(
            envelope
                .get("flat_layout_migrated")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(1),
        );
        assert!(envelope
            .get("apply_error")
            .map(|v| !v.is_null())
            .unwrap_or(false));
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
        let actions = DoctorActions::from_findings(&mut report);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(outcome.agents_md_regenerated);

        let after = fs::read_to_string(&path).unwrap();
        assert!(after.starts_with("# My custom heading\n\nMy hand-written notes.\n\n"));
        assert!(after.contains("Closing prose.\n"));
        assert!(!after.contains("stale body"));
        assert!(after.contains(agents::MANAGED_START));
        assert!(after.contains(agents::MANAGED_END));
    }

    #[test]
    fn legacy_issues_agents_md_is_detected_and_rewritten() {
        let tmp = fresh_repo();
        let issues_dir = tmp.path().join("issues");
        fs::create_dir_all(&issues_dir).unwrap();
        let path = issues_dir.join("AGENTS.md");
        // Pre-v0.5.0 scaffold marker.
        fs::write(
            &path,
            "# Issues\n\n## Issue Numbering\n\nIssue numbers are sequential...\n",
        )
        .unwrap();

        let mut report = scan(tmp.path()).unwrap();
        assert!(report.legacy_issues_agents_md);

        let actions = DoctorActions::from_findings(&mut report);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(outcome.issues_agents_md_rewritten);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            crate::skill::ISSUES_AGENTS_TEMPLATE
        );
    }

    #[test]
    fn customized_issues_agents_md_is_left_alone() {
        let tmp = fresh_repo();
        let issues_dir = tmp.path().join("issues");
        fs::create_dir_all(&issues_dir).unwrap();
        let path = issues_dir.join("AGENTS.md");
        let custom = "# Our team's policy\n\nWe write our own rules here.\n";
        fs::write(&path, custom).unwrap();

        let report = scan(tmp.path()).unwrap();
        assert!(!report.legacy_issues_agents_md);
        assert_eq!(fs::read_to_string(&path).unwrap(), custom);
    }

    /// Single-pass `scan_issues` powers every check. This fixture wires
    /// up many independent findings in one repo and asserts the merged
    /// `DoctorFindings` looks the same as the multi-walk produced — a
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
            tmp.path()
                .join("issues/quiet-brave-otter/.issuectl-tmp-XYZ"),
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
            r.symlinked_dirs
                .iter()
                .any(|s| s.contains("symlinked-thing")),
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

    /// Golden-snapshot test for the `render_json` output. Intentionally
    /// avoids any non-deterministic input (no legacy `<NN>-<slug>` dirs
    /// — those go through `slug::generate_unique`, no symlinks — paths
    /// differ across platforms). Verifies the byte shape downstream
    /// JSON consumers depend on.
    #[test]
    fn render_json_matches_golden_snapshot() {
        let tmp = fresh_repo();
        // Issue with: broken epic ref + future timestamp + unknown key.
        put_flat(
            &tmp,
            "alpha-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\n\
             epic: nonexistent-ghost-fox\ncreated: 2999-01-01\n\
             whimsy: 1\n---\n# A\n",
        );
        // Issue with closing status but no `closed:` (status consistency).
        put_flat(
            &tmp,
            "beta-quiet-otter",
            "---\ntype: bug\nstatus: done\npriority: normal\n---\n# B\n",
        );
        // Empty dir → missing_item_md.
        fs::create_dir_all(tmp.path().join("issues/charlie-empty-dir")).unwrap();

        let report = scan(tmp.path()).unwrap();
        let json = render_json(&report, None, false, tmp.path());
        let actual = serde_json::to_string_pretty(&json).unwrap();
        // Normalise the tempdir prefix so the snapshot is portable.
        let actual = actual.replace(tmp.path().to_str().unwrap(), "<TMP>");

        let expected = r#"{
  "agents_md_check_skipped": null,
  "agents_md_drift": false,
  "agents_md_malformed": null,
  "agents_md_missing": true,
  "agents_md_regenerated": false,
  "alias_coercions": [],
  "blocked_by_cycles": [],
  "both_open_and_closed": [],
  "broken_attachment_refs": [],
  "broken_refs": [
    {
      "kind": "epic",
      "slug": "alpha-bright-cat",
      "target": "nonexistent-ghost-fox"
    }
  ],
  "closed_with_active_status": [],
  "conflict_markers": [],
  "duplicate_slugs": [],
  "files_rewritten": 0,
  "fix_applied": false,
  "flat_layout_conflicts": [],
  "flat_layout_migrated": [],
  "flat_layout_planned": [],
  "gitignored_paths": [],
  "invalid_slugs": [],
  "issues_agents_md_rewritten": false,
  "large_binaries": [],
  "legacy_issues_agents_md": false,
  "migrations": [],
  "missing_body_sections": [],
  "missing_item_md": [
    "flat/charlie-empty-dir"
  ],
  "non_avif_images": [],
  "notes_conflicts": [],
  "notes_renamed": [],
  "notes_to_rename": [],
  "open_with_closing_status": [],
  "orphan_epic_refs": [
    {
      "epic": "nonexistent-ghost-fox",
      "slug": "alpha-bright-cat"
    }
  ],
  "orphan_tempfiles": [],
  "orphan_tempfiles_removed": [],
  "parse_errors": [],
  "schema_missing": true,
  "schema_parse_error": null,
  "schema_violations": [],
  "status_consistency": [
    {
      "message": "closing status \"done\" requires `closed:` date",
      "slug": "beta-quiet-otter"
    }
  ],
  "status_reconciled": [],
  "symlinked_dirs": [],
  "timestamp_issues": [
    {
      "message": "created date 2999-01-01 is in the future",
      "slug": "alpha-bright-cat"
    }
  ],
  "transition_warnings": [],
  "unknown_keys": [
    {
      "key": "whimsy",
      "slug": "alpha-bright-cat"
    }
  ],
  "unknown_reviewers": []
}"#;
        assert_eq!(
            actual, expected,
            "render_json output drifted from the golden snapshot.\n\
             If the change is intentional, update the snapshot."
        );
    }

    /// Bug #1 (`apply()` returns `Result<()>` and `bail!`s — `--json
    /// --fix` when preflight blocks → no JSON, anyhow text on stderr):
    /// the new `apply` returns `Ok(outcome)` with `outcome.blockers`
    /// populated instead of `Err`, and the JSON envelope carries the
    /// blockers under `apply_outcome` so scripted callers can read a
    /// structured response.
    #[test]
    fn json_fix_with_preflight_block_emits_structured_outcome() {
        let tmp = fresh_repo();
        // Slug present in BOTH legacy folders → preflight blocker.
        for f in ["open", "closed"] {
            let dir = tmp.path().join("issues").join(f).join("quiet-brave-otter");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
            )
            .unwrap();
        }
        let mut findings = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(!outcome.blockers.is_empty());
        assert_eq!(outcome.stop_phase, StopPhase::Preflight);
        // `fix_applied: true` here reflects the unconditional schema
        // bootstrap that fires before preflight refusal (issue:
        // @unreasonably-attractive-star). No other phase ran — the
        // BOTH-folders blocker still gates everything else.
        assert!(outcome.schema_bootstrapped);
        assert!(outcome.legacy_dirs_migrated.is_empty());
        assert!(outcome.flat_layout_migrated.is_empty());

        let json = render_json(&findings, Some(&outcome), true, tmp.path());
        let ao = json
            .get("apply_outcome")
            .expect("apply_outcome must be present on --fix");
        assert_eq!(ao["fix_applied"], serde_json::Value::Bool(true));
        assert_eq!(
            ao["stop_phase"],
            serde_json::Value::String("preflight".into())
        );
        let blockers = ao["blockers"].as_array().unwrap();
        assert!(!blockers.is_empty(), "blockers must surface in JSON");
        assert!(
            blockers
                .iter()
                .any(|v| v.as_str().unwrap_or("").contains("BOTH")),
            "expected `BOTH issues/open/...` blocker, got {blockers:?}"
        );
    }

    /// Clean-success path with writes: when `apply` runs to completion
    /// with no blockers AND at least one write happened (fresh repo
    /// triggers schema bootstrap), `stop_phase: "ok"` MUST coexist
    /// with `fix_applied: true`. JSON consumers should not have to
    /// infer the success case from `blockers.is_empty()`.
    #[test]
    fn clean_success_with_writes_envelope_carries_ok_and_fix_applied_true() {
        let tmp = fresh_repo();
        let mut findings = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(outcome.blockers.is_empty(), "clean repo: no blockers");
        assert_eq!(outcome.stop_phase, StopPhase::Ok);
        assert!(
            outcome.schema_bootstrapped,
            "fresh repo: schema bootstrap landed"
        );
        assert!(
            outcome.fix_applied(),
            "schema_bootstrapped flips fix_applied"
        );

        let scan_after = scan(tmp.path()).unwrap();
        let json = render_json(&scan_after, Some(&outcome), true, tmp.path());
        assert_eq!(
            json["apply_outcome"]["stop_phase"],
            serde_json::Value::String("ok".into())
        );
        assert_eq!(
            json["apply_outcome"]["fix_applied"],
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            json["apply_outcome"]["schema_bootstrapped"],
            serde_json::Value::Bool(true)
        );
        let blockers = json["apply_outcome"]["blockers"].as_array().unwrap();
        assert!(blockers.is_empty(), "no blockers on clean success");
    }

    /// Clean-success path with NO writes: schema already bootstrapped
    /// from a prior run, no findings ⇒ `apply` is a no-op.
    /// `stop_phase: "ok"` MUST coexist with `fix_applied: false`.
    /// This pins the second `(ok, fix_applied)` combination — the
    /// matrix is undertested without it.
    #[test]
    fn clean_success_no_writes_envelope_carries_ok_and_fix_applied_false() {
        let tmp = fresh_repo();
        // Pre-bootstrap the schema so the second `apply` writes nothing.
        schema::ensure_default_written(tmp.path()).unwrap();

        let mut findings = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(outcome.blockers.is_empty());
        assert_eq!(outcome.stop_phase, StopPhase::Ok);
        assert!(
            !outcome.fix_applied(),
            "no-op apply must not flip fix_applied"
        );

        let scan_after = scan(tmp.path()).unwrap();
        let json = render_json(&scan_after, Some(&outcome), true, tmp.path());
        assert_eq!(
            json["apply_outcome"]["stop_phase"],
            serde_json::Value::String("ok".into())
        );
        assert_eq!(
            json["apply_outcome"]["fix_applied"],
            serde_json::Value::Bool(false)
        );
    }

    /// Bug #4 (manual splice list): the post-apply rendering pulls
    /// from `ApplyOutcome` directly. Adding a new applied-action
    /// variant means extending `ApplyOutcome` + `DoctorActions::
    /// fix_applied` — no field-by-field copy in `run`.
    #[test]
    fn fix_applied_predicate_is_centralised_on_outcome() {
        let mut o = ApplyOutcome::default();
        assert!(!o.fix_applied(), "default outcome reports false");
        o.schema_bootstrapped = true;
        assert!(
            o.fix_applied(),
            "schema_bootstrapped alone must flip fix_applied (bug #3)"
        );
        let mut o = ApplyOutcome::default();
        o.notes_renamed.push("foo".into());
        assert!(o.fix_applied(), "notes_renamed must flip fix_applied");
    }

    /// Bug #5 (preflight ↔ has_critical_findings drift): the two
    /// call sites share a single `blockers_for` core. The alignment
    /// is now intentional-and-narrower: preflight uses the layout-
    /// fatal subset (`apply_blockers`), which is a strict subset of
    /// the exit-code set (`critical_blockers`). Layout-fatal
    /// findings (here: conflict markers) appear in BOTH lists —
    /// preflight refuses on them. Schema-shape findings appear in
    /// `critical_blockers` only — they drive exit-1 but do NOT
    /// refuse `--fix` (issue: @staggeringly-important-zoo). The
    /// drift bug is still gone because both lists derive from one
    /// function with one set of decisions.
    #[test]
    fn critical_blockers_aligns_preflight_with_exit_code() {
        let tmp = fresh_repo();
        // Conflict markers: layout-fatal AND exit-1. The two sets
        // agree on this class.
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> branch\n",
        );
        let findings = scan(tmp.path()).unwrap();
        let crit = critical_blockers(&findings);
        let pre = apply_blockers(&findings);
        assert!(!crit.is_empty(), "conflict markers should be a blocker");
        assert_eq!(crit, pre, "layout-fatal blockers appear in both views");

        let mut findings_for_apply = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings_for_apply);
        assert_eq!(
            pre, actions.preflight_blockers,
            "preflight_blockers must equal apply_blockers output"
        );
    }

    /// Issue @staggeringly-important-zoo: schema violations no
    /// longer block `--fix`. The flat-layout migration is the
    /// safest, most mechanical operation in the toolbox; gating it
    /// on schema cleanliness inverted the priority. After this
    /// change a repo with concurrent layout AND schema violations
    /// migrates the layout in one pass and reports the remaining
    /// schema violations on the post-migration state.
    #[test]
    fn schema_violations_do_not_block_layout_migration() {
        let tmp = fresh_repo();
        // Issue at legacy `issues/open/<slug>/` with a body missing
        // the schema-required `priority` field. Pre-fix scan must
        // see both: a layout migration AND a schema violation. The
        // schema violation is in `critical_blockers` (exit-1) but
        // NOT in `apply_blockers` (layout-fatal), so `--fix` should
        // run the layout migration anyway.
        let dir = tmp.path().join("issues/open/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();

        let mut findings = scan(tmp.path()).unwrap();
        assert!(
            !findings.schema_violations.is_empty(),
            "expected schema violation pre-fix"
        );
        assert!(
            !critical_blockers(&findings).is_empty(),
            "schema violation must remain in exit-1 set"
        );
        assert!(
            apply_blockers(&findings).is_empty(),
            "schema violation must NOT be in apply-preflight set"
        );

        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        // Layout migration ran despite the schema violation.
        assert!(
            !outcome.flat_layout_migrated.is_empty(),
            "flat-layout migration must run when only schema violations remain"
        );
        assert!(tmp
            .path()
            .join("issues/quiet-brave-otter/item.md")
            .is_file());
        assert_eq!(outcome.stop_phase, StopPhase::Ok);

        // Post-migration scan still surfaces the unresolved schema
        // violation against the post-migration path — exit-1 still
        // fires, but as forward work rather than a blocker.
        let after = scan(tmp.path()).unwrap();
        assert!(
            !after.schema_violations.is_empty(),
            "schema violation persists post-fix and surfaces against post-migration path"
        );
        assert!(!critical_blockers(&after).is_empty());
    }

    /// Issue @ridiculously-outrageous-fold: long warning lists
    /// collapse to a one-liner when not `--verbose`. The 3DBear
    /// migration printed 240 layout-migration entries every
    /// iteration of "fix-something-rerun-doctor" loops; this
    /// verifies the actual rendered text on both sides of the
    /// threshold and the verbose escape hatch. Asserting against
    /// the rendered string (rather than just pinning the constant)
    /// catches regressions in wording, formatting, or wiring of the
    /// `verbose` flag through `print_section`'s callers.
    #[test]
    fn render_section_collapses_long_lists_unless_verbose() {
        let exactly_at_limit: Vec<i32> = (0..RENDER_FULL_LIST_LIMIT as i32).collect();
        let one_over: Vec<i32> = (0..(RENDER_FULL_LIST_LIMIT + 1) as i32).collect();

        // Empty: nothing rendered, no leading newline.
        let mut buf = String::new();
        render_section(
            &mut buf,
            "Title:",
            &Vec::<i32>::new(),
            false,
            "thing(s)",
            |i| i.to_string(),
        );
        assert!(
            buf.is_empty(),
            "empty list must render nothing, got {buf:?}"
        );

        // Exactly LIMIT entries: full list, not collapsed.
        let mut buf = String::new();
        render_section(
            &mut buf,
            "Title:",
            &exactly_at_limit,
            false,
            "thing(s)",
            |i| i.to_string(),
        );
        assert!(buf.contains("Title:"), "expected title, got {buf:?}");
        for i in &exactly_at_limit {
            assert!(buf.contains(&i.to_string()), "missing entry {i}: {buf:?}");
        }
        assert!(
            !buf.contains("re-run with --verbose"),
            "must not collapse at exactly LIMIT entries: {buf:?}"
        );

        // LIMIT+1 entries, non-verbose: collapsed to a one-liner
        // with the count and the verb phrase.
        let mut buf = String::new();
        render_section(&mut buf, "Title:", &one_over, false, "thing(s)", |i| {
            i.to_string()
        });
        assert!(
            buf.contains(&format!("{} thing(s)", one_over.len())),
            "expected collapsed count line, got {buf:?}"
        );
        assert!(
            buf.contains("re-run with --verbose to list"),
            "expected verbose hint, got {buf:?}"
        );
        assert!(
            !buf.contains("Title:"),
            "collapsed render must omit the title, got {buf:?}"
        );

        // LIMIT+1 entries, verbose: full list, no collapse hint.
        let mut buf = String::new();
        render_section(&mut buf, "Title:", &one_over, true, "thing(s)", |i| {
            i.to_string()
        });
        assert!(buf.contains("Title:"), "verbose must print title: {buf:?}");
        for i in &one_over {
            assert!(buf.contains(&i.to_string()), "verbose missing {i}: {buf:?}");
        }
        assert!(
            !buf.contains("re-run with --verbose"),
            "verbose must not show the collapse hint: {buf:?}"
        );
    }

    /// `apply_blockers` must always be a SUBSET of
    /// `critical_blockers`. The two functions share a single
    /// `blockers_for(scope)` core, but the manual `!layout_only`
    /// guards on each schema-shape branch make it possible for a
    /// future change to accidentally classify a finding as
    /// preflight-only (would refuse `--fix` for something that
    /// doesn't drive exit-1) or omit it from both. Pinning the
    /// subset relation with a fixture that produces every
    /// schema-shape finding catches the most likely regression
    /// shape: a new finding category added to one branch of
    /// `blockers_for` and forgotten in the other.
    #[test]
    fn apply_blockers_is_a_subset_of_critical_blockers() {
        let tmp = fresh_repo();
        // Issue with multiple schema-shape problems: missing
        // required `priority`, broken `epic` cross-reference (a
        // valid slug shape but no such issue), and timestamps that
        // disagree with status (`closed:` set while `status: open`).
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\nepic: alpha-bright-cat\nclosed: 2026-01-02\ncreated: 2026-01-01\nupdated: 2026-01-01\n---\n# T\n",
        )
        .unwrap();
        let findings = scan(tmp.path()).unwrap();
        // Sanity: at least one schema-shape finding fired.
        assert!(
            !findings.schema_violations.is_empty()
                || !findings.broken_refs.is_empty()
                || !findings.status_consistency.is_empty(),
            "fixture must produce a schema-shape finding"
        );

        let crit: BTreeSet<String> = critical_blockers(&findings).into_iter().collect();
        let pre: BTreeSet<String> = apply_blockers(&findings).into_iter().collect();
        assert!(
            pre.is_subset(&crit),
            "apply_blockers must be a subset of critical_blockers.\n  pre: {pre:?}\n  crit: {crit:?}"
        );
        // The schema-shape findings must be in `critical_blockers`
        // (drive exit-1) but NOT in `apply_blockers` (don't refuse
        // `--fix`). At least one ExitCode-only finding must exist
        // in this fixture.
        assert!(
            crit.len() > pre.len(),
            "fixture must exercise an ExitCode-only finding (crit > pre): crit={crit:?} pre={pre:?}"
        );
    }

    /// `--fix` must run the legacy NN-rename phase against a
    /// post-flat-layout fresh scan even when the fresh scan
    /// surfaces schema-shape findings (schema violations, broken
    /// refs, status inconsistencies, timestamp issues). Before
    /// this bundle, the post-apply re-check used
    /// `critical_blockers` and would bail with `StopPhase::PostApply`
    /// the moment any of those appeared on the post-migration
    /// state. Now it uses `apply_blockers`, so NN-rename proceeds
    /// — schema findings remain visible as forward work in the
    /// final scan and drive exit-1, but they don't strand the
    /// migration.
    #[test]
    fn nn_rename_runs_when_post_migration_scan_has_schema_findings() {
        let tmp = fresh_repo();
        // Numbered-legacy issue under `issues/closed/`. Body has no
        // schema-required `priority`, so the post-migration rescan
        // (after both flat-layout migration AND the NN-rename's
        // `rewrite_item_frontmatter`) will surface a schema
        // violation against the new canonical slug — proving the
        // path completed end-to-end despite the schema-shape
        // finding.
        let old = tmp.path().join("issues/closed/3-old");
        fs::create_dir_all(&old).unwrap();
        fs::write(
            old.join("item.md"),
            "---\nnumber: 3\ntype: bug\nstatus: open\n---\n# Old\n",
        )
        .unwrap();

        let mut findings = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        // Layout migration ran AND NN-rename ran (post-migration
        // schema findings did not bail PostApply).
        assert_eq!(outcome.stop_phase, StopPhase::Ok);
        assert_eq!(outcome.flat_layout_migrated.len(), 1);
        assert_eq!(
            outcome.legacy_dirs_migrated.len(),
            1,
            "NN-rename must run despite post-migration schema findings: {outcome:?}"
        );

        // Post-fix scan still surfaces the unresolved schema
        // violation against the new canonical slug — exit-1 still
        // fires (caller asserts), but as forward work, not a
        // pipeline bail.
        let after = scan(tmp.path()).unwrap();
        assert!(
            !after.schema_violations.is_empty(),
            "expected lingering schema violation against post-rename path"
        );
    }

    /// Issue @unreasonably-attractive-star: schema bootstrap fires
    /// unconditionally on `--fix`, even when other preflight
    /// blockers are present. Prior behavior advertised
    /// auto-creation in the read-only output but failed to deliver
    /// because preflight bailed before bootstrap.
    #[test]
    fn schema_bootstrap_runs_even_when_preflight_blocks() {
        let tmp = fresh_repo();
        // Slug present in both legacy folders → preflight blocker.
        for f in ["open", "closed"] {
            let dir = tmp.path().join("issues").join(f).join("quiet-brave-otter");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
            )
            .unwrap();
        }
        // Sanity: `.schema.yaml` does not exist yet.
        assert!(!tmp.path().join("issues/.schema.yaml").exists());

        let mut findings = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        assert_eq!(outcome.stop_phase, StopPhase::Preflight);
        assert!(!outcome.blockers.is_empty());
        // The promise: bootstrap landed despite preflight refusal.
        assert!(
            outcome.schema_bootstrapped,
            "schema bootstrap must precede preflight refusal"
        );
        assert!(
            tmp.path().join("issues/.schema.yaml").is_file(),
            "schema file must be on disk after preflight-blocked --fix"
        );
    }

    /// Round-2 regression #1 (state destruction on preflight-blocked
    /// path): `DoctorActions::from_findings` drains the to-do data via
    /// `mem::take`. Before the fix, `run` only re-scanned when
    /// `outcome.fix_applied()` was true — so a preflight-blocked run
    /// rendered an empty findings object and the user saw the blocker
    /// message but none of the pending lists. The fix unconditionally
    /// re-scans after apply.
    #[test]
    fn preflight_blocked_render_path_does_not_lose_pending_work() {
        let tmp = fresh_repo();
        // One legitimate legacy migration that scan should surface,
        // plus a slug present in BOTH legacy folders → preflight
        // blocker. After apply returns Ok(outcome) with blockers
        // populated, the rescanned `findings` MUST still contain the
        // legacy migration entry.
        put_legacy(
            &tmp,
            "open",
            7,
            "alpha",
            "---\nnumber: 7\nstatus: open\n---\n# A\n",
        );
        for f in ["open", "closed"] {
            let dir = tmp.path().join("issues").join(f).join("quiet-brave-otter");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
            )
            .unwrap();
        }

        let mut findings = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(!outcome.blockers.is_empty(), "expected preflight blocker");
        // Simulate `run`'s post-apply rescan.
        let final_findings = scan(tmp.path()).unwrap();
        assert!(
            !final_findings.legacy_dirs.is_empty(),
            "rescan must surface the legacy migration even when preflight blocked"
        );
        // The JSON envelope on this path must carry both the blockers
        // AND the to-do lists.
        let json = render_json(&final_findings, Some(&outcome), true, tmp.path());
        let migrations = json["migrations"].as_array().unwrap();
        assert!(
            !migrations.is_empty(),
            "migrations field must not be empty on preflight-blocked path"
        );
        let blockers = json["apply_outcome"]["blockers"].as_array().unwrap();
        assert!(!blockers.is_empty());
    }

    /// Round-2 regression #2 (legacy numeric refs in flat-layout
    /// blocking `--fix`): a flat-layout issue with `epic: 7` produces
    /// a `broken_refs` entry of kind "(legacy numeric ref)". Before
    /// the fix, `critical_blockers` treated this as a refusal. The
    /// migration is supposed to heal it via `rewrite_item_frontmatter`.
    #[test]
    fn legacy_numeric_refs_in_flat_layout_do_not_block_fix() {
        let tmp = fresh_repo();
        // Flat-layout issue with a stale numeric epic ref — this is
        // exactly the state a partially-migrated repo will have.
        put_flat(
            &tmp,
            "alpha-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\nepic: 7\n---\n# A\n",
        );
        let findings = scan(tmp.path()).unwrap();
        // Sanity: scan flagged it.
        assert!(
            findings
                .broken_refs
                .iter()
                .any(|(_, k, t)| k == "epic" && t.contains("(legacy numeric ref)")),
            "expected legacy-numeric ref to be flagged: {:?}",
            findings.broken_refs
        );
        // critical_blockers must NOT contain a "broken cross-references"
        // entry that would refuse `--fix`.
        let blockers = critical_blockers(&findings);
        assert!(
            !blockers
                .iter()
                .any(|b| b.contains("broken cross-references")),
            "legacy numeric refs must not block --fix: {:?}",
            blockers
        );
    }

    /// Round-2 regression #3 (notes apply-time conflict silently
    /// dropped): when scan classified a file as `SafeRename` but a
    /// concurrent edit between scan and apply added a `## Comments`
    /// heading, the fix is skipped. The skip MUST be recorded in
    /// `outcome.notes_conflicts_at_apply` so JSON consumers see that
    /// planned work was deferred.
    #[test]
    fn notes_conflict_at_apply_is_recorded_in_outcome() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/legacy-notes-here");
        fs::create_dir_all(&dir).unwrap();
        let item = dir.join("item.md");
        // Initially: SafeRename (one ## Notes, no ## Comments).
        fs::write(
            &item,
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n## Notes\n\nold\n",
        )
        .unwrap();
        let mut findings = scan(tmp.path()).unwrap();
        assert!(findings
            .notes_to_rename
            .iter()
            .any(|s| s == "legacy-notes-here"));

        // Concurrent edit: a user appends a SECOND `## Notes` section
        // before apply runs — an ambiguous shape. `migrate_notes_heading`
        // will now classify this as Conflict at apply time. (Adding a
        // single `## Comments` would instead auto-merge; multiple
        // `## Notes` is the shape that still needs a human.)
        fs::write(
            &item,
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n## Notes\n\nold\n\n## Notes\n\nnewer\n",
        )
        .unwrap();

        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(
            outcome
                .notes_conflicts_at_apply
                .iter()
                .any(|s| s == "legacy-notes-here"),
            "TOCTOU conflict must surface in outcome: {:?}",
            outcome
        );
        assert!(outcome.notes_renamed.is_empty(), "no rename when conflict");

        // The conflict MUST also surface in the JSON envelope, not
        // only on the typed outcome — the field's docstring promises
        // `--json --fix` consumers a signal that some planned work
        // didn't run.
        let scan_after = scan(tmp.path()).unwrap();
        let json = render_json(&scan_after, Some(&outcome), true, tmp.path());
        let conflicts = json["apply_outcome"]["notes_conflicts_at_apply"]
            .as_array()
            .expect("notes_conflicts_at_apply must be in apply_outcome envelope");
        assert!(
            conflicts
                .iter()
                .any(|v| v.as_str() == Some("legacy-notes-here")),
            "JSON envelope must surface notes_conflicts_at_apply, got {conflicts:?}"
        );
    }

    /// Round-2 regression #4 (hard parse errors on legacy dirs): a
    /// legacy issue with unparseable frontmatter MUST surface as a
    /// Hard parse_error so `critical_blockers` refuses `--fix`. The
    /// alternative is mid-apply panic when `write::read_item` hits
    /// the same broken YAML.
    #[test]
    fn hard_parse_errors_on_legacy_dirs_block_fix() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/7-broken-legacy");
        fs::create_dir_all(&dir).unwrap();
        // Legacy frontmatter that parses-as-mapping fails.
        fs::write(dir.join("item.md"), "---\nfoo: : :\n---\n# T\n").unwrap();
        let r = scan(tmp.path()).unwrap();
        // Soft warnings on legacy issues are still suppressed (the
        // intentional skip), but Hard errors must surface.
        assert!(
            r.parse_errors
                .iter()
                .any(|e| e.severity == ParseSeverity::Hard),
            "Hard parse error on legacy must be surfaced: {:?}",
            r.parse_errors
        );
        let blockers = critical_blockers(&r);
        assert!(
            blockers
                .iter()
                .any(|b| b.contains("unparseable issue file")),
            "Hard parse error must block --fix: {:?}",
            blockers
        );
    }

    /// Bug #6 (substring matcher): typed `ParseSeverity` set at the
    /// push site means re-wording the parser's message no longer
    /// reclassifies a hard fail as a soft warn. The legacy-numeric
    /// epic-ref warning is emitted Soft, the unparseable frontmatter
    /// is emitted Hard.
    #[test]
    fn parse_error_severity_is_typed_not_substring_matched() {
        let tmp = fresh_repo();
        // Soft: legacy numeric epic ref on a flat-layout issue.
        put_flat(
            &tmp,
            "alpha-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\nepic: 7\n---\n# A\n",
        );
        // Hard: unparseable frontmatter.
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\nfoo: : :\n---\n# T\n").unwrap();

        let r = scan(tmp.path()).unwrap();
        let any_soft = r
            .parse_errors
            .iter()
            .any(|e| e.severity == ParseSeverity::Soft);
        let any_hard = r
            .parse_errors
            .iter()
            .any(|e| e.severity == ParseSeverity::Hard);
        assert!(
            any_soft,
            "legacy numeric ref must be Soft: {:?}",
            r.parse_errors
        );
        assert!(
            any_hard,
            "unparseable YAML must be Hard: {:?}",
            r.parse_errors
        );

        // Soft alone does NOT block; only the Hard entries appear in
        // critical_blockers.
        let blockers = critical_blockers(&r);
        let hard_blocker = blockers
            .iter()
            .find(|b| b.contains("unparseable issue file"));
        assert!(
            hard_blocker.is_some(),
            "hard parse error must produce a blocker, got {:?}",
            blockers
        );
    }

    #[test]
    fn flags_large_binaries_non_avif_and_broken_refs() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "noisy-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             ![shot](attachments/shot.png)\n\
             [missing](attachments/gone.avif)\n",
        );
        let issue_dir = tmp.path().join("issues/noisy-bright-cat");
        let att = issue_dir.join("attachments");
        fs::create_dir_all(&att).unwrap();
        // Non-AVIF image AND > 1 MiB → flagged by both checks.
        fs::write(att.join("shot.png"), vec![0u8; (1 << 20) + 10]).unwrap();
        // Small AVIF fixture that the body does NOT reference: clean.
        fs::create_dir_all(issue_dir.join("fixtures")).unwrap();
        fs::write(issue_dir.join("fixtures/ok.bin"), b"tiny").unwrap();

        let r = scan(tmp.path()).unwrap();

        assert_eq!(
            r.non_avif_images,
            vec![(
                "noisy-bright-cat".to_string(),
                "issues/noisy-bright-cat/attachments/shot.png".to_string()
            )]
        );
        assert_eq!(r.large_binaries.len(), 1);
        assert_eq!(r.large_binaries[0].0, "noisy-bright-cat");
        assert!(r.large_binaries[0].2 > (1 << 20));
        // `shot.png` resolves; `gone.avif` does not.
        assert_eq!(
            r.broken_attachment_refs,
            vec![(
                "noisy-bright-cat".to_string(),
                "attachments/gone.avif".to_string()
            )]
        );

        // Warning-only: none of these are critical blockers.
        assert!(critical_blockers(&r).is_empty());
    }

    #[test]
    fn clean_issue_with_avif_attachment_has_no_attachment_warnings() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "calm-quiet-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             ![shot](attachments/shot.avif)\n",
        );
        let att = tmp.path().join("issues/calm-quiet-otter/attachments");
        fs::create_dir_all(&att).unwrap();
        fs::write(att.join("shot.avif"), b"small").unwrap();

        let r = scan(tmp.path()).unwrap();
        assert!(r.non_avif_images.is_empty());
        assert!(r.large_binaries.is_empty());
        assert!(r.broken_attachment_refs.is_empty());
    }

    /// Regression: `broken_attachment_refs` must not flag link/image
    /// syntax that lives inside a backtick code span. The author is
    /// describing the syntax, not using it. Class 1 of issue
    /// @doctor-attachment-refs-false-positives.
    #[test]
    fn broken_refs_skips_link_syntax_inside_code_span() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "code-span-with-image-syntax",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             Use `![alt](path)` syntax for images, and `[text](url)` for links.\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.broken_attachment_refs.is_empty(),
            "link syntax inside backticks must not be flagged: {:?}",
            r.broken_attachment_refs
        );
    }

    /// Regression: a `..`-escaping link target (the path leaves the
    /// issue dir) is already rejected by `normalize_relative_ref`'s
    /// component check, regardless of the cross-file-pointer logic.
    /// Pin that path explicitly so a future change to either layer
    /// doesn't silently start flagging cross-dir links.
    #[test]
    fn broken_refs_skips_parent_dir_escape_link() {
        let tmp = fresh_repo();
        fs::write(tmp.path().join("foo.ts"), b"// stub\n").unwrap();
        put_flat(
            &tmp,
            "parent-dir-escape",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             See [foo.ts:10-20](../foo.ts#L10-L20).\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.broken_attachment_refs.is_empty(),
            "parent-dir escape must not surface as a broken ref: {:?}",
            r.broken_attachment_refs
        );
    }

    /// Regression: a non-escaping repo-relative pointer with a
    /// GitHub-style `#L<n>` line anchor — the actual Class-2 shape
    /// from the 3DBear bug report. The path resolves under the issue
    /// dir (not via `..`), so the heuristic that gates the repo-root
    /// existence check on the anchor shape is what saves it.
    #[test]
    fn broken_refs_skips_repo_relative_code_pointer_with_line_anchor() {
        let tmp = fresh_repo();
        // Mirror the 3DBear shape: a real source file lives at the
        // repo root and is referenced from the issue body with a
        // `#L<n>` permalink fragment.
        fs::create_dir_all(tmp.path().join("kurssi-ai-server/src/cli")).unwrap();
        fs::write(
            tmp.path().join("kurssi-ai-server/src/cli/sops.ts"),
            b"// stub\n",
        )
        .unwrap();
        put_flat(
            &tmp,
            "code-pointer-with-line-anchor",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             `loadMoodleAdminPassword()` in \
             [sops.ts:87-98](kurssi-ai-server/src/cli/sops.ts#L87-L98).\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.broken_attachment_refs.is_empty(),
            "GitHub-permalink-shaped code pointer must not be flagged: {:?}",
            r.broken_attachment_refs
        );
    }

    /// Regression for the silent-false-negative class identified in
    /// review: a missing sibling attachment whose filename collides
    /// with a repo-root file (`README.md`, `Cargo.toml`, …) must STILL
    /// be flagged. The earlier "exists at repo root → skip" heuristic
    /// silently masked these; the line-anchor gate is what keeps the
    /// bare-filename case honest.
    #[test]
    fn broken_refs_still_flags_when_filename_collides_with_repo_root() {
        let tmp = fresh_repo();
        fs::write(tmp.path().join("README.md"), b"# repo readme\n").unwrap();
        put_flat(
            &tmp,
            "collides-with-repo-root",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             ![logo](README.md)\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert_eq!(
            r.broken_attachment_refs,
            vec![(
                "collides-with-repo-root".to_string(),
                "README.md".to_string()
            )],
            "missing sibling attachment must not be masked by a repo-root collision"
        );
    }

    /// Positive case: a genuinely missing sibling attachment must
    /// still be flagged after the parser/scope refactor.
    #[test]
    fn broken_refs_flags_legit_missing_sibling_attachment() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "legit-missing-attachment",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             ![screenshot](missing.png)\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert_eq!(
            r.broken_attachment_refs,
            vec![(
                "legit-missing-attachment".to_string(),
                "missing.png".to_string()
            )]
        );
    }

    /// Sibling attachment that exists must not be flagged.
    #[test]
    fn broken_refs_clean_for_existing_sibling_attachment() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "legit-existing-attachment",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             ![ok](existing.avif)\n",
        );
        fs::write(
            tmp.path()
                .join("issues/legit-existing-attachment/existing.avif"),
            b"x",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r.broken_attachment_refs.is_empty());
    }

    /// Issue @doctor-fix-noop, success criterion D: pin the exit-code
    /// contract via `classify_exit`. Unit-testable so the mapping
    /// doesn't drift behind `run`'s `std::process::exit` site.
    #[test]
    fn classify_exit_maps_apply_outcomes_to_envelope_codes() {
        // Clean Ok + no manual leftovers → exit 0.
        let findings = DoctorFindings::default();
        let oc = ApplyOutcome::default();
        let d = classify_exit(&findings, Some(&oc), true);
        assert_eq!(d.code, 0, "clean Ok must exit 0");

        // Realistic Ok + notes leftovers: `findings.notes_conflicts`
        // ALSO contains the slug (because the post-apply rescan
        // surfaces unmergeable bodies), so the dead-code regression
        // (gemini #1, opus 1.2) requires this assertion to hit the
        // specific notes-merge branch despite `crit` being non-empty.
        let mut findings_with_notes = DoctorFindings::default();
        findings_with_notes.notes_conflicts.push("foo".into());
        let mut oc = ApplyOutcome::default();
        oc.notes_conflicts_at_apply.push("foo".into());
        let d = classify_exit(&findings_with_notes, Some(&oc), true);
        assert_eq!(d.code, 1);
        assert_eq!(d.error_code, "doctor-partial");
        assert!(
            d.message.contains("manual") && d.message.contains("Notes"),
            "message must call out the manual notes/comments merge, got: {}",
            d.message
        );

        // Preflight → doctor-blocked.
        let oc = ApplyOutcome {
            stop_phase: StopPhase::Preflight,
            blockers: vec!["dup".into()],
            ..ApplyOutcome::default()
        };
        let d = classify_exit(&findings, Some(&oc), true);
        assert_eq!(d.code, 1);
        assert_eq!(d.error_code, "doctor-blocked");
        assert!(d.message.contains("preflight"));

        // PostApply → doctor-partial.
        let oc = ApplyOutcome {
            stop_phase: StopPhase::PostApply,
            blockers: vec!["x".into()],
            ..ApplyOutcome::default()
        };
        let d = classify_exit(&findings, Some(&oc), true);
        assert_eq!(d.code, 1);
        assert_eq!(d.error_code, "doctor-partial");
        assert!(d.message.contains("post-apply"));

        // apply_error → doctor-apply-error.
        let oc = ApplyOutcome {
            apply_error: Some("oops".into()),
            ..ApplyOutcome::default()
        };
        let d = classify_exit(&findings, Some(&oc), true);
        assert_eq!(d.code, 1);
        assert_eq!(d.error_code, "doctor-apply-error");

        // Ok + generic critical findings (no notes leftover) →
        // doctor-partial with the generic "unfixable" message.
        let mut findings = DoctorFindings::default();
        findings.duplicate_slugs.push("dup".into());
        let oc = ApplyOutcome::default();
        let d = classify_exit(&findings, Some(&oc), true);
        assert_eq!(d.code, 1);
        assert_eq!(d.error_code, "doctor-partial");
        assert!(d.message.contains("unfixable"));

        // Read-only with critical findings → doctor-unhealthy.
        let d = classify_exit(&findings, None, false);
        assert_eq!(d.code, 1);
        assert_eq!(d.error_code, "doctor-unhealthy");
    }
}
