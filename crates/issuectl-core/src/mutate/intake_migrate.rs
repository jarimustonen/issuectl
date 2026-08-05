//! Legacy-intake **data migration** — the dry-run-first, cross-field pass
//! from `docs/design/intake-flow.md` §6 that lifts label-encoded intake
//! state onto the first-class statuses/fields the rest of the intake flow
//! now owns.
//!
//! This is deliberately **not** a `status_alias` coercion. `status_alias`
//! rewrites one *value* of one field to another value of the *same* field;
//! here we read a **label** and decide a **status** and/or a **provenance**
//! field from it — new cross-field logic doctor does not have.
//!
//! Design guarantees this pass upholds:
//!
//! - **Dry-run by default.** [`migrate`] with `apply: false` computes and
//!   reports exactly what it *would* do and writes nothing; the caller
//!   re-runs with `apply: true` to commit.
//! - **Refuses ambiguity.** An issue whose legacy labels disagree
//!   (`needs-triage` *and* `deferred`, or `via:telegram` against a
//!   different already-set `provenance`) is **skipped whole** and reported
//!   for manual review — never guessed at.
//! - **No status regression, ever.** A status change is emitted *only* from
//!   `open`. An in-flight (`in-progress`/`testing`) or already-closed item
//!   keeps its status; the stale label is dropped with a warning. A closed
//!   item is never reopened.
//! - **Idempotent + per-issue atomic.** Each issue's write goes through the
//!   shared [`super::update_issue_under_lock`] (one tempfile-rename). A
//!   second run finds the legacy labels already gone and is a no-op.
//!
//! ### Why the graph rules are bypassed (but not the invariants)
//!
//! The default transition matrix (`transitions.yaml`) does **not** list
//! `open` in `untriaged.allowed_from` — a normal `set status untriaged`
//! from `open` is rejected. That is correct for the *lifecycle*, but this
//! is a one-time *repair* of mis-encoded state, not a lifecycle move. So
//! the migration writes through `update_issue_under_lock` with an **empty**
//! [`TransitionRules`], skipping the repo's opt-in graph while still
//! enforcing the always-on intrinsic invariants (no `untriaged →
//! in-progress`, type × status completion) and schema validation. Since
//! every emitted status change is `open → untriaged|deferred`, no intrinsic
//! invariant is tripped in practice.
//!
//! Conflicts are reported **in-band** (a skipped action with a reason), not
//! raised as errors, so the pass introduces no new `--json` error code.

use std::path::Path;

use crate::models::Issue;
use crate::repo_config::ConfigSource;
use crate::schema::{is_closing, Schema};
use crate::transitions::TransitionRules;

use super::intake::IntakeError;
use super::{MutateError, Patch, UpdateIssueRequest, WriteLock};

// ── Legacy label vocabulary (the ad-hoc Telegram-path encoding) ──────────
const L_NEEDS_TRIAGE: &str = "needs-triage";
const L_DEFERRED: &str = "deferred";
const L_TRIAGED: &str = "triaged";
const L_VIA_TELEGRAM: &str = "via:telegram";

/// Provenance value the `via:telegram` label maps onto.
const PROV_TELEGRAM: &str = "telegram";
const F_PROVENANCE: &str = "provenance";

/// One issue's planned (or performed) migration. Either a `conflict`
/// (skipped whole for manual review) or a set of concrete changes.
#[derive(Debug, Clone, Default)]
pub struct MigrateAction {
    pub slug: String,
    /// When set: the issue is ambiguous and was **skipped** — no field was
    /// touched. The string is the reason for manual review.
    pub conflict: Option<String>,
    /// `(from, to)` status change. Only ever emitted from `open`.
    pub status_change: Option<(String, String)>,
    /// Legacy labels removed.
    pub dropped_labels: Vec<String>,
    /// Provenance value written (only when none was set before).
    pub set_provenance: Option<String>,
    /// Non-fatal notes (e.g. "dropped stale label; item already closed").
    pub warnings: Vec<String>,
    /// True once the write has actually happened (apply mode). Always
    /// false for a conflict and for a dry run.
    pub applied: bool,
}

impl MigrateAction {
    fn conflict(slug: &str, reason: impl Into<String>) -> Self {
        MigrateAction {
            slug: slug.to_string(),
            conflict: Some(reason.into()),
            ..Default::default()
        }
    }

    fn is_empty(&self) -> bool {
        self.conflict.is_none()
            && self.status_change.is_none()
            && self.dropped_labels.is_empty()
            && self.set_provenance.is_none()
            && self.warnings.is_empty()
    }
}

/// The whole pass's outcome. Only issues that need work (or are skipped as
/// conflicts) appear in `actions`; clean issues are omitted.
#[derive(Debug, Clone)]
pub struct MigrateReport {
    /// Whether the writes were actually performed (`--apply`).
    pub applied: bool,
    pub actions: Vec<MigrateAction>,
}

impl MigrateReport {
    pub fn migrated_count(&self) -> usize {
        self.actions.iter().filter(|a| a.conflict.is_none()).count()
    }
    pub fn skipped_count(&self) -> usize {
        self.actions.iter().filter(|a| a.conflict.is_some()).count()
    }
}

fn has_label(issue: &Issue, label: &str) -> bool {
    issue
        .labels
        .as_ref()
        .is_some_and(|ls| ls.iter().any(|l| l == label))
}

fn provenance_of(issue: &Issue) -> Option<&str> {
    issue.extra.get(F_PROVENANCE).and_then(|v| v.as_str())
}

/// Compute the migration plan for one issue against `schema` (needed to
/// classify custom closing statuses correctly). Returns `None` when there
/// is nothing to do — the issue carries no recognised legacy label, so it
/// is left byte-for-byte untouched (this is what leaves `tg-bug-*` slugs
/// and already-migrated items alone).
///
/// Pure and side-effect-free so every row of the §6 table is unit-testable
/// without touching disk.
pub(crate) fn plan_issue(issue: &Issue, schema: &Schema) -> Option<MigrateAction> {
    let needs_triage = has_label(issue, L_NEEDS_TRIAGE);
    let deferred_label = has_label(issue, L_DEFERRED);
    let triaged = has_label(issue, L_TRIAGED);
    let via_telegram = has_label(issue, L_VIA_TELEGRAM);

    if !(needs_triage || deferred_label || triaged || via_telegram) {
        return None;
    }

    let status = issue.status.as_str();
    let provenance = provenance_of(issue);

    // ── Conflicts: refuse rather than guess; skip the whole issue ────────
    // needs-triage + deferred disagree on the target status.
    if needs_triage && deferred_label {
        return Some(MigrateAction::conflict(
            &issue.slug,
            "carries both `needs-triage` and `deferred` labels — ambiguous target status; migrate by hand",
        ));
    }
    // via:telegram against a *different* already-set provenance.
    if via_telegram {
        if let Some(p) = provenance {
            if p != PROV_TELEGRAM {
                return Some(MigrateAction::conflict(
                    &issue.slug,
                    format!(
                        "`via:telegram` label but provenance is already {p:?} — conflicting source; migrate by hand"
                    ),
                ));
            }
        }
    }

    let mut action = MigrateAction {
        slug: issue.slug.clone(),
        ..Default::default()
    };

    // ── needs-triage → untriaged (only from `open`) ──────────────────────
    if needs_triage {
        action.dropped_labels.push(L_NEEDS_TRIAGE.to_string());
        if status == "open" {
            action.status_change = Some(("open".to_string(), "untriaged".to_string()));
        } else if status == "in-progress" || status == "testing" {
            action.warnings.push(format!(
                "dropped stale `needs-triage` — item is already `{status}` (no status change)"
            ));
        } else if is_closing(schema, status) {
            action.warnings.push(format!(
                "dropped stale `needs-triage` — item is closed (`{status}`); not reopened"
            ));
        }
        // else: already `untriaged`/`needs-info`/other active — just drop
        // the redundant label (idempotent cleanup), no status change.
    }

    // ── deferred → deferred (only from `open`) ───────────────────────────
    if deferred_label {
        action.dropped_labels.push(L_DEFERRED.to_string());
        if status == "open" {
            action.status_change = Some(("open".to_string(), "deferred".to_string()));
        } else if status == "deferred" {
            // idempotent: status already right, drop redundant label.
        } else if is_closing(schema, status) {
            action.warnings.push(format!(
                "dropped stale `deferred` label — item is closed (`{status}`); not reopened"
            ));
        } else {
            action.warnings.push(format!(
                "dropped stale `deferred` label — item is `{status}` (no status change)"
            ));
        }
    }

    // ── triaged (old "presented" marker) → drop; never invent state ──────
    if triaged {
        action.dropped_labels.push(L_TRIAGED.to_string());
    }

    // ── via:telegram → provenance:telegram (drop label) ──────────────────
    if via_telegram {
        action.dropped_labels.push(L_VIA_TELEGRAM.to_string());
        if provenance.is_none() {
            action.set_provenance = Some(PROV_TELEGRAM.to_string());
        }
        // provenance already == telegram: label is redundant, just dropped.
    }

    if action.is_empty() {
        None
    } else {
        Some(action)
    }
}

/// Run the legacy-intake migration. With `apply == false` this is a pure
/// read that reports what it would change; with `apply == true` each
/// planned action is written through the shared under-lock path (one
/// atomic write per issue).
///
/// The whole apply pass holds the repo write lock once, so it is
/// serialized against other writers; each issue's write is still
/// individually atomic, so a mid-pass I/O error leaves earlier issues
/// committed and a re-run resumes cleanly (the pass is idempotent).
pub fn migrate(
    root: &Path,
    apply: bool,
    config: &dyn ConfigSource,
) -> Result<MigrateReport, IntakeError> {
    // A single lock for the whole pass: for apply it serializes the batch;
    // for dry-run it gives a consistent snapshot (no writer slips in
    // between load and report).
    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    let schema = config
        .schema(root)
        .map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    // Empty rules ⇒ the repo's opt-in transition graph is bypassed for this
    // repair pass; the intrinsic invariants inside `update_issue_under_lock`
    // still apply. See the module docs.
    let empty_rules = TransitionRules::default();

    let issues = crate::repo::load_issues_with_config(root, config);

    let mut actions: Vec<MigrateAction> = Vec::new();
    for issue in &issues {
        let Some(mut action) = plan_issue(issue, &schema) else {
            continue;
        };
        if apply && action.conflict.is_none() {
            let mut req = UpdateIssueRequest::default();
            if let Some((_, to)) = &action.status_change {
                req.status = Patch::Set(to.clone());
            }
            req.remove_labels = action.dropped_labels.clone();
            if let Some(p) = &action.set_provenance {
                req.custom_fields
                    .insert(F_PROVENANCE.to_string(), Patch::Set(p.clone()));
            }
            req.validate()?;
            let item_path = super::locate_for_dry_run(root, &issue.slug)?;
            super::update_issue_under_lock(
                root,
                &issue.slug,
                item_path,
                req,
                None,
                &schema,
                &empty_rules,
            )?;
            action.applied = true;
        }
        actions.push(action);
    }

    Ok(MigrateReport {
        applied: apply,
        actions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_config::UncachedConfig;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn fresh_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        tmp
    }

    /// Write a legacy-form issue directly to disk (bypassing the intake
    /// filer, which would refuse to set these legacy labels/status combos).
    /// Injects the schema-required `priority`/`created` fields so the
    /// under-lock write path (which re-validates the whole issue) is happy;
    /// real issues always carry them.
    fn write_issue(root: &Path, slug: &str, frontmatter: &str) {
        let dir = root.join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        let mut fm = String::new();
        if !frontmatter.contains("priority:") {
            fm.push_str("priority: normal\n");
        }
        if !frontmatter.contains("created:") {
            fm.push_str("created: 2026-01-01\n");
        }
        fm.push_str(frontmatter);
        let body = format!("---\n{fm}---\n\n# {slug}\n\nlegacy body\n");
        fs::write(dir.join("item.md"), body).unwrap();
    }

    fn load(root: &Path, slug: &str) -> Issue {
        crate::repo::load_issues(root)
            .into_iter()
            .find(|i| i.slug == slug)
            .unwrap()
    }

    fn schema() -> std::sync::Arc<Schema> {
        UncachedConfig.schema(&std::env::temp_dir()).unwrap()
    }

    // ── plan_issue: one test per §6 table row ────────────────────────────

    #[test]
    fn needs_triage_open_becomes_untriaged() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "nt-open",
            "slug: nt-open\ntype: bug\nstatus: open\nlabels: [needs-triage]\n",
        );
        let issue = load(tmp.path(), "nt-open");
        let a = plan_issue(&issue, &schema()).unwrap();
        assert_eq!(a.status_change, Some(("open".into(), "untriaged".into())));
        assert_eq!(a.dropped_labels, vec![L_NEEDS_TRIAGE]);
        assert!(a.warnings.is_empty());
    }

    #[test]
    fn needs_triage_in_flight_drops_label_no_status_change() {
        let tmp = fresh_repo();
        for status in ["in-progress", "testing"] {
            let slug = format!("nt-{status}");
            write_issue(
                tmp.path(),
                &slug,
                &format!("slug: {slug}\ntype: bug\nstatus: {status}\nassignee: bob\nlabels: [needs-triage]\n"),
            );
            let a = plan_issue(&load(tmp.path(), &slug), &schema()).unwrap();
            assert!(a.status_change.is_none(), "{status}: no status change");
            assert_eq!(a.dropped_labels, vec![L_NEEDS_TRIAGE]);
            assert_eq!(a.warnings.len(), 1, "{status}: warns");
        }
    }

    #[test]
    fn needs_triage_closed_never_reopens() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "nt-closed",
            "slug: nt-closed\ntype: bug\nstatus: wontfix\nclosed: 2026-01-01\nlabels: [needs-triage]\n",
        );
        let a = plan_issue(&load(tmp.path(), "nt-closed"), &schema()).unwrap();
        assert!(a.status_change.is_none(), "closed item never reopened");
        assert_eq!(a.dropped_labels, vec![L_NEEDS_TRIAGE]);
        assert_eq!(a.warnings.len(), 1);
    }

    #[test]
    fn needs_triage_and_deferred_together_is_conflict() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "conflicted-item",
            "slug: conflicted\ntype: bug\nstatus: open\nlabels: [needs-triage, deferred]\n",
        );
        let a = plan_issue(&load(tmp.path(), "conflicted-item"), &schema()).unwrap();
        assert!(a.conflict.is_some());
        assert!(a.status_change.is_none());
        assert!(a.dropped_labels.is_empty(), "conflict touches nothing");
    }

    #[test]
    fn deferred_open_becomes_deferred() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "def-open",
            "slug: def-open\ntype: feature\nstatus: open\nlabels: [deferred]\n",
        );
        let a = plan_issue(&load(tmp.path(), "def-open"), &schema()).unwrap();
        assert_eq!(a.status_change, Some(("open".into(), "deferred".into())));
        assert_eq!(a.dropped_labels, vec![L_DEFERRED]);
    }

    #[test]
    fn triaged_label_is_dropped_without_inventing_state() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "old-triaged",
            "slug: old-triaged\ntype: bug\nstatus: open\nlabels: [triaged]\n",
        );
        let a = plan_issue(&load(tmp.path(), "old-triaged"), &schema()).unwrap();
        assert!(a.status_change.is_none(), "open stays open");
        assert_eq!(a.dropped_labels, vec![L_TRIAGED]);
    }

    #[test]
    fn via_telegram_no_provenance_sets_provenance() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "tg-item",
            "slug: tg-item\ntype: bug\nstatus: open\nlabels: [via:telegram]\n",
        );
        let a = plan_issue(&load(tmp.path(), "tg-item"), &schema()).unwrap();
        assert_eq!(a.set_provenance.as_deref(), Some("telegram"));
        assert_eq!(a.dropped_labels, vec![L_VIA_TELEGRAM]);
    }

    #[test]
    fn via_telegram_matching_provenance_just_drops_label() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "tg-dup",
            "slug: tg-dup\ntype: bug\nstatus: open\nprovenance: telegram\nlabels: [via:telegram]\n",
        );
        let a = plan_issue(&load(tmp.path(), "tg-dup"), &schema()).unwrap();
        assert!(a.set_provenance.is_none(), "already set, no rewrite");
        assert_eq!(a.dropped_labels, vec![L_VIA_TELEGRAM]);
    }

    #[test]
    fn via_telegram_conflicting_provenance_is_conflict() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "tg-conflict",
            "slug: tg-conflict\ntype: bug\nstatus: open\nprovenance: email\nlabels: [via:telegram]\n",
        );
        let a = plan_issue(&load(tmp.path(), "tg-conflict"), &schema()).unwrap();
        assert!(a.conflict.is_some());
        assert!(a.dropped_labels.is_empty());
    }

    #[test]
    fn tg_bug_slug_without_legacy_labels_is_untouched() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "tg-bug-alice-1-2",
            "slug: tg-bug-alice-1-2\ntype: bug\nstatus: open\nprovenance: telegram\n",
        );
        assert!(
            plan_issue(&load(tmp.path(), "tg-bug-alice-1-2"), &schema()).is_none(),
            "no legacy label ⇒ no action, slug never rewritten"
        );
    }

    // ── migrate(): dry-run, apply, idempotency ───────────────────────────

    #[test]
    fn dry_run_reports_but_writes_nothing() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "dr-item",
            "slug: dr-item\ntype: bug\nstatus: open\nlabels: [needs-triage, via:telegram]\n",
        );
        let report = migrate(tmp.path(), false, &UncachedConfig).unwrap();
        assert!(!report.applied);
        assert_eq!(report.migrated_count(), 1);
        // Nothing on disk changed.
        let after = load(tmp.path(), "dr-item");
        assert_eq!(after.status, "open");
        assert!(has_label(&after, L_NEEDS_TRIAGE), "label still present");
    }

    #[test]
    fn apply_migrates_and_is_idempotent() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "ap-item",
            "slug: ap-item\ntype: bug\nstatus: open\nlabels: [needs-triage, via:telegram]\n",
        );
        let r1 = migrate(tmp.path(), true, &UncachedConfig).unwrap();
        assert!(r1.applied);
        assert_eq!(r1.migrated_count(), 1);
        assert!(r1.actions[0].applied);

        let after = load(tmp.path(), "ap-item");
        assert_eq!(after.status, "untriaged", "open → untriaged");
        assert!(!has_label(&after, L_NEEDS_TRIAGE), "label dropped");
        assert!(!has_label(&after, L_VIA_TELEGRAM), "label dropped");
        assert_eq!(provenance_of(&after), Some("telegram"), "provenance set");

        // Second run: nothing left to do.
        let r2 = migrate(tmp.path(), true, &UncachedConfig).unwrap();
        assert!(r2.actions.is_empty(), "idempotent — no-op on re-run");
    }

    #[test]
    fn apply_bypasses_transition_graph_for_open_to_untriaged() {
        // With the default matrix installed, a plain `set status untriaged`
        // from `open` is illegal — the migration must still succeed.
        let tmp = fresh_repo();
        crate::transitions::write_default(tmp.path(), false).unwrap();
        write_issue(
            tmp.path(),
            "graphed-item",
            "slug: graphed\ntype: bug\nstatus: open\nlabels: [needs-triage]\n",
        );
        let r = migrate(tmp.path(), true, &UncachedConfig).unwrap();
        assert_eq!(r.migrated_count(), 1);
        assert_eq!(load(tmp.path(), "graphed-item").status, "untriaged");
    }

    #[test]
    fn apply_skips_conflicts_without_writing() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "skip-me",
            "slug: skip-me\ntype: bug\nstatus: open\nlabels: [needs-triage, deferred]\n",
        );
        let r = migrate(tmp.path(), true, &UncachedConfig).unwrap();
        assert_eq!(r.skipped_count(), 1);
        assert!(!r.actions[0].applied);
        // Untouched on disk.
        let after = load(tmp.path(), "skip-me");
        assert_eq!(after.status, "open");
        assert!(has_label(&after, L_NEEDS_TRIAGE));
        assert!(has_label(&after, L_DEFERRED));
    }

    #[test]
    fn in_flight_apply_drops_label_keeps_status() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "wip-item",
            "type: bug\nstatus: in-progress\nassignee: bob\nlabels: [needs-triage]\n",
        );
        let r = migrate(tmp.path(), true, &UncachedConfig).unwrap();
        assert_eq!(r.migrated_count(), 1);
        let after = load(tmp.path(), "wip-item");
        assert_eq!(after.status, "in-progress", "no regression");
        assert!(!has_label(&after, L_NEEDS_TRIAGE), "stale label dropped");
    }
}
