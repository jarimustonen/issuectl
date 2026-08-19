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
//!   (`needs-triage` *and* `deferred`, multiple distinct `via:<channel>`
//!   channels, a malformed `via:` label, or a channel that conflicts with
//!   already-set `provenance`) is **skipped whole** and reported
//!   for manual review — never guessed at.
//! - **No status regression, ever.** A status change is emitted *only* from
//!   `open`. An in-flight (`in-progress`/`testing`) or already-closed item
//!   keeps its status; the stale label is dropped with a warning. A closed
//!   item is never reopened.
//! - **Idempotent + per-issue atomic.** Each issue's write goes through the
//!   shared `super::update_issue_under_lock` (one tempfile-rename). A
//!   second run finds the legacy labels already gone and is a no-op.
//! - **Per-issue error isolation.** In `--apply`, a schema rejection or I/O
//!   failure on one item records an `error` on that action and leaves it
//!   untouched; the pass continues so every other legacy item still
//!   migrates. The full [`MigrateReport`] is always returned (never an
//!   opaque early abort), and a failed write makes the command exit
//!   non-zero. Because each write is atomic and the plan idempotent, a
//!   re-run retries only the still-unmigrated items.
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
use crate::schema::{is_closing, Schema};
use crate::transitions::TransitionRules;

use super::intake::IntakeError;
use super::{MutateError, Patch, UpdateIssueRequest, WriteLock};

// ── Legacy label vocabulary ──────────────────────────────────────────────
const L_NEEDS_TRIAGE: &str = "needs-triage";
const L_DEFERRED: &str = "deferred";
const L_TRIAGED: &str = "triaged";
const VIA_PREFIX: &str = "via:";
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
    /// Set when this item was planned but its `--apply` write **failed**
    /// (schema rejection, I/O error). The plan is sound but the item was
    /// left untouched; other items still migrate. A re-run retries it.
    pub error: Option<String>,
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
    /// Non-conflict, non-error actions: in dry-run these are *planned*, in
    /// apply they are the ones that succeeded (`error` is `None`).
    pub fn migrated_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| a.conflict.is_none() && a.error.is_none())
            .count()
    }
    pub fn skipped_count(&self) -> usize {
        self.actions.iter().filter(|a| a.conflict.is_some()).count()
    }
    /// Items whose `--apply` write failed (apply mode only). Non-zero here
    /// means the command must exit non-zero even though conflicts alone do
    /// not — a failed write is an error, an ambiguous item is expected.
    pub fn failed_count(&self) -> usize {
        self.actions.iter().filter(|a| a.error.is_some()).count()
    }
}

fn has_label(issue: &Issue, label: &str) -> bool {
    issue
        .labels
        .as_ref()
        .is_some_and(|ls| ls.iter().any(|l| l == label))
}

/// Classification of legacy `via:` labels shared by migration and read-side
/// queue projection.
#[derive(Debug, PartialEq, Eq)]
pub enum LegacyVia<'a> {
    None,
    Unique {
        channel: &'a str,
        labels: Vec<&'a str>,
    },
    Ambiguous {
        labels: Vec<&'a str>,
    },
    Malformed {
        labels: Vec<&'a str>,
    },
}

/// Classify the service-neutral legacy `via:<channel>` label convention.
///
/// Every historical channel, including `agent-*`, follows the same conversion.
/// Exact duplicate labels collapse into one removal operation. Empty or padded
/// suffixes are malformed rather than normalized: migration never guesses at
/// provenance, and a recognized legacy-shaped label is never silently stranded.
pub fn classify_legacy_via(issue: &Issue) -> LegacyVia<'_> {
    let mut labels = Vec::new();
    let mut channels = Vec::new();
    let mut malformed = Vec::new();

    for label in issue.labels.iter().flatten() {
        let Some(channel) = label.strip_prefix(VIA_PREFIX) else {
            continue;
        };
        if channel.is_empty() || channel.trim() != channel {
            if !malformed.contains(&label.as_str()) {
                malformed.push(label.as_str());
            }
            continue;
        }
        if !labels.contains(&label.as_str()) {
            labels.push(label.as_str());
        }
        if !channels.contains(&channel) {
            channels.push(channel);
        }
    }

    if !malformed.is_empty() {
        LegacyVia::Malformed { labels: malformed }
    } else if channels.len() > 1 {
        LegacyVia::Ambiguous { labels }
    } else if let Some(channel) = channels.first() {
        LegacyVia::Unique { channel, labels }
    } else {
        LegacyVia::None
    }
}

/// How an existing `provenance` frontmatter value classifies. Distinguishes
/// "absent" (safe to set from `via:<channel>`) from "present but not a
/// string" (an object/number/etc. a later tool or schema wrote) — the
/// latter must never be silently overwritten.
enum ExistingProvenance<'a> {
    Absent,
    Str(&'a str),
    NonString,
}

fn provenance_of(issue: &Issue) -> ExistingProvenance<'_> {
    match issue.extra.get(F_PROVENANCE) {
        None | Some(serde_json::Value::Null) => ExistingProvenance::Absent,
        Some(serde_json::Value::String(s)) => ExistingProvenance::Str(s),
        Some(_) => ExistingProvenance::NonString,
    }
}

/// Compute the migration plan for one issue against `schema` (needed to
/// classify custom closing statuses correctly). Returns `None` when there
/// is nothing to do — the issue carries no recognised legacy label, so it
/// is left byte-for-byte untouched (this is what leaves legacy deterministic
/// slugs and already-migrated items alone).
///
/// Pure and side-effect-free so every row of the §6 table is unit-testable
/// without touching disk.
pub(crate) fn plan_issue(issue: &Issue, schema: &Schema) -> Option<MigrateAction> {
    let needs_triage = has_label(issue, L_NEEDS_TRIAGE);
    let deferred_label = has_label(issue, L_DEFERRED);
    let triaged = has_label(issue, L_TRIAGED);
    let legacy_via = classify_legacy_via(issue);

    if !(needs_triage || deferred_label || triaged || !matches!(&legacy_via, LegacyVia::None)) {
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
    // Distinct or malformed legacy channels are ambiguous even if provenance
    // is absent. Duplicate copies of one exact label remain safe and collapse
    // into one removal operation.
    match &legacy_via {
        LegacyVia::Malformed { labels } => {
            return Some(MigrateAction::conflict(
                &issue.slug,
                format!(
                    "carries malformed legacy `via:` label(s) {labels:?} — no usable channel token; migrate by hand"
                ),
            ));
        }
        LegacyVia::Ambiguous { labels } => {
            return Some(MigrateAction::conflict(
                &issue.slug,
                format!(
                    "carries multiple distinct `via:<channel>` labels {labels:?} — ambiguous source; migrate by hand"
                ),
            ));
        }
        LegacyVia::Unique { channel, .. } => {
            // A via label against an already-set provenance that is not a matching
            // string is a conflict. Structured values must never be overwritten.
            match provenance {
                ExistingProvenance::Str(p) if p != *channel => {
                    return Some(MigrateAction::conflict(
                        &issue.slug,
                        format!(
                            "`via:{channel}` label but provenance is already {p:?} — conflicting source; migrate by hand"
                        ),
                    ));
                }
                ExistingProvenance::NonString => {
                    return Some(MigrateAction::conflict(
                        &issue.slug,
                        format!(
                            "`via:{channel}` label but provenance is already set to a non-string value — migrate by hand"
                        ),
                    ));
                }
                _ => {}
            }
        }
        LegacyVia::None => {}
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
        } else if status == "untriaged" || status == "needs-info" {
            // Already in an intake state — the label is redundant; drop it
            // quietly (idempotent cleanup), no status change.
        } else {
            // Some other active status (a project-defined one). Never
            // regress it, but do not drop the label silently — warn so the
            // mis-encoded item is auditable, mirroring the `deferred` branch.
            action.warnings.push(format!(
                "dropped `needs-triage` — item is `{status}` (no status change)"
            ));
        }
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

    // ── via:<channel> → provenance:<channel> (drop label) ────────────────
    // A conflicting/non-string provenance already returned a conflict above,
    // so here provenance is either absent or already the matching string.
    if let LegacyVia::Unique { channel, labels } = &legacy_via {
        action
            .dropped_labels
            .extend(labels.iter().map(|label| (*label).to_string()));
        if matches!(provenance, ExistingProvenance::Absent) {
            action.set_provenance = Some((*channel).to_string());
        }
        // Matching provenance means each legacy label is redundant and dropped.
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
pub fn migrate(root: &Path, apply: bool) -> Result<MigrateReport, IntakeError> {
    // A single lock for the whole pass: for apply it serializes the batch;
    // for dry-run it gives a consistent snapshot (no writer slips in
    // between load and report).
    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    let schema =
        crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    // Empty rules ⇒ the repo's opt-in transition graph is bypassed for this
    // repair pass; the intrinsic invariants inside `update_issue_under_lock`
    // still apply. See the module docs.
    let empty_rules = TransitionRules::default();

    let issues = crate::repo::load_issues(root);

    let mut actions: Vec<MigrateAction> = Vec::new();
    for issue in &issues {
        let Some(mut action) = plan_issue(issue, &schema) else {
            continue;
        };
        // Defensive: this pass bypasses the transition graph, so a planner
        // bug that emitted anything other than `open → untriaged|deferred`
        // would slip past the graph and rely solely on the intrinsic
        // invariants. Assert the invariant the module docs promise, so such
        // a bug fails loudly in tests/debug rather than mutating state.
        debug_assert!(
            match &action.status_change {
                None => true,
                Some((from, to)) => from == "open" && (to == "untriaged" || to == "deferred"),
            },
            "migration emitted an unexpected status change: {:?}",
            action.status_change
        );
        if apply && action.conflict.is_none() {
            // Per-issue error isolation: a schema rejection or I/O failure on
            // one item records an `error` and leaves it untouched; the pass
            // continues so every other legacy item still migrates. Each write
            // is atomic and the plan is idempotent, so a re-run retries only
            // the failed items. A whole-batch `?` would instead abandon the
            // report after some items were already committed.
            match apply_one(root, &action, &schema, &empty_rules) {
                Ok(()) => action.applied = true,
                Err(e) => action.error = Some(format!("{e}")),
            }
        }
        actions.push(action);
    }

    Ok(MigrateReport {
        applied: apply,
        actions,
    })
}

/// Write one planned action through the shared under-lock path with the
/// flock already held by [`migrate`]. `rules` are empty (graph bypassed);
/// intrinsic invariants + schema validation still run inside
/// `update_issue_under_lock`.
fn apply_one(
    root: &Path,
    action: &MigrateAction,
    schema: &Schema,
    rules: &TransitionRules,
) -> Result<(), IntakeError> {
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
    let item_path = super::locate_for_dry_run(root, &action.slug)?;
    super::update_issue_under_lock(
        root,
        &action.slug,
        item_path,
        req,
        schema,
        rules,
        &crate::clock::SystemClock,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn schema() -> Schema {
        crate::schema::load(&std::env::temp_dir()).unwrap()
    }

    /// Load the schema a repo actually declares (for tests that write a
    /// custom `.schema.yaml` — e.g. custom statuses / provenance enum).
    fn schema_at(root: &Path) -> Schema {
        crate::schema::load(root).unwrap()
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
    fn via_chat_no_provenance_sets_provenance() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "via-item",
            "slug: via-item\ntype: bug\nstatus: open\nlabels: [via:chat]\n",
        );
        let a = plan_issue(&load(tmp.path(), "via-item"), &schema()).unwrap();
        assert_eq!(a.set_provenance.as_deref(), Some("chat"));
        assert_eq!(a.dropped_labels, vec!["via:chat"]);
    }

    #[test]
    fn via_agent_channel_is_generalized_to_provenance() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "agent-item",
            "slug: agent-item\ntype: bug\nstatus: open\nlabels: [via:agent-support]\n",
        );
        let a = plan_issue(&load(tmp.path(), "agent-item"), &schema()).unwrap();
        assert_eq!(a.set_provenance.as_deref(), Some("agent-support"));
        assert_eq!(a.dropped_labels, vec!["via:agent-support"]);
    }

    #[test]
    fn via_chat_matching_provenance_just_drops_label() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "via-duplicate",
            "slug: via-duplicate\ntype: bug\nstatus: open\nprovenance: chat\nlabels: [via:chat]\n",
        );
        let a = plan_issue(&load(tmp.path(), "via-duplicate"), &schema()).unwrap();
        assert!(a.set_provenance.is_none(), "already set, no rewrite");
        assert_eq!(a.dropped_labels, vec!["via:chat"]);
    }

    #[test]
    fn via_chat_conflicting_provenance_is_conflict() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "via-conflict",
            "slug: via-conflict\ntype: bug\nstatus: open\nprovenance: email\nlabels: [via:chat]\n",
        );
        let a = plan_issue(&load(tmp.path(), "via-conflict"), &schema()).unwrap();
        assert!(a.conflict.is_some());
        assert!(a.dropped_labels.is_empty());
    }

    #[test]
    fn multiple_via_channels_are_a_conflict() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "ambiguous-source",
            "slug: ambiguous-source\ntype: bug\nstatus: open\nlabels: [via:chat, via:webform]\n",
        );
        let a = plan_issue(&load(tmp.path(), "ambiguous-source"), &schema()).unwrap();
        assert!(a.conflict.is_some());
        assert!(a.dropped_labels.is_empty());
        assert!(a.set_provenance.is_none());
    }

    #[test]
    fn malformed_via_labels_are_whole_issue_conflicts() {
        for (slug, label) in [("bare-via", "via:"), ("padded-via", "via: chat")] {
            let tmp = fresh_repo();
            write_issue(
                tmp.path(),
                slug,
                &format!(
                    "slug: {slug}\ntype: bug\nstatus: open\nlabels: [needs-triage, {label:?}]\n"
                ),
            );
            let a = plan_issue(&load(tmp.path(), slug), &schema()).unwrap();
            assert!(a.conflict.is_some(), "{label:?} must be reported");
            assert!(a.status_change.is_none(), "conflict touches nothing");
            assert!(a.dropped_labels.is_empty(), "conflict touches nothing");
        }
    }

    #[test]
    fn channel_shaped_slug_without_legacy_labels_is_untouched() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "legacy-shaped-alice-1-2",
            "slug: legacy-shaped-alice-1-2\ntype: bug\nstatus: open\nprovenance: chat\n",
        );
        assert!(
            plan_issue(&load(tmp.path(), "legacy-shaped-alice-1-2"), &schema()).is_none(),
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
            "slug: dr-item\ntype: bug\nstatus: open\nlabels: [needs-triage, via:chat]\n",
        );
        let report = migrate(tmp.path(), false).unwrap();
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
            "slug: ap-item\ntype: bug\nstatus: open\nlabels: [needs-triage, via:chat]\n",
        );
        let r1 = migrate(tmp.path(), true).unwrap();
        assert!(r1.applied);
        assert_eq!(r1.migrated_count(), 1);
        assert!(r1.actions[0].applied);

        let after = load(tmp.path(), "ap-item");
        assert_eq!(after.status, "untriaged", "open → untriaged");
        assert!(!has_label(&after, L_NEEDS_TRIAGE), "label dropped");
        assert!(!has_label(&after, "via:chat"), "label dropped");
        assert!(
            matches!(provenance_of(&after), ExistingProvenance::Str("chat")),
            "provenance set"
        );

        // Second run: nothing left to do.
        let r2 = migrate(tmp.path(), true).unwrap();
        assert!(r2.actions.is_empty(), "idempotent — no-op on re-run");
    }

    #[test]
    fn duplicate_via_labels_apply_once_and_are_idempotent() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "duplicate-via",
            "slug: duplicate-via\ntype: bug\nstatus: open\nlabels: [via:chat, via:chat]\n",
        );

        let r1 = migrate(tmp.path(), true).unwrap();
        assert_eq!(r1.failed_count(), 0);
        assert_eq!(r1.actions[0].dropped_labels, vec!["via:chat"]);
        let after = load(tmp.path(), "duplicate-via");
        assert!(
            after
                .labels
                .iter()
                .flatten()
                .all(|label| label != "via:chat"),
            "all duplicate legacy labels are removed in one pass"
        );
        assert!(matches!(
            provenance_of(&after),
            ExistingProvenance::Str("chat")
        ));
        assert!(migrate(tmp.path(), true).unwrap().actions.is_empty());
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
        let r = migrate(tmp.path(), true).unwrap();
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
        let r = migrate(tmp.path(), true).unwrap();
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
        let r = migrate(tmp.path(), true).unwrap();
        assert_eq!(r.migrated_count(), 1);
        let after = load(tmp.path(), "wip-item");
        assert_eq!(after.status, "in-progress", "no regression");
        assert!(!has_label(&after, L_NEEDS_TRIAGE), "stale label dropped");
    }

    #[test]
    fn via_chat_non_string_provenance_is_conflict() {
        // A structured/non-string provenance must never be silently
        // overwritten with "chat" — it is a whole-issue conflict.
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "via-structured",
            "type: bug\nstatus: open\nprovenance:\n  system: chat\n  account: foo\nlabels: [via:chat]\n",
        );
        let a = plan_issue(&load(tmp.path(), "via-structured"), &schema()).unwrap();
        assert!(a.conflict.is_some(), "non-string provenance → conflict");
        assert!(a.set_provenance.is_none(), "never overwrites");
        assert!(a.dropped_labels.is_empty(), "conflict touches nothing");
    }

    #[test]
    fn needs_triage_on_custom_active_status_warns() {
        // A project-defined active status is not `open`; never regress it,
        // but do not drop `needs-triage` silently — warn for auditability.
        // `blocked` is not a built-in closing status, so the default schema
        // classifies it active — no custom schema needed for plan-only.
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "blocked-item",
            "type: bug\nstatus: blocked\nlabels: [needs-triage]\n",
        );
        let a = plan_issue(&load(tmp.path(), "blocked-item"), &schema()).unwrap();
        assert!(a.status_change.is_none(), "custom active never regressed");
        assert_eq!(a.dropped_labels, vec![L_NEEDS_TRIAGE]);
        assert_eq!(a.warnings.len(), 1, "warns rather than dropping silently");
    }

    #[test]
    fn needs_triage_on_custom_closing_status_is_schema_aware() {
        // The closing classification is schema-driven: a custom closing
        // status must be treated like a built-in one (drop label, warn, no
        // reopen), proving `plan_issue` honours `status_classes`.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nstatus_classes:\n  archived: closing\n",
        )
        .unwrap();
        write_issue(
            tmp.path(),
            "archived-item",
            "type: bug\nstatus: archived\nclosed: 2026-01-01\nlabels: [needs-triage]\n",
        );
        let a = plan_issue(&load(tmp.path(), "archived-item"), &schema_at(tmp.path())).unwrap();
        assert!(a.status_change.is_none(), "closed item never reopened");
        assert_eq!(a.dropped_labels, vec![L_NEEDS_TRIAGE]);
        assert!(
            a.warnings[0].contains("closed"),
            "warns closed: {:?}",
            a.warnings
        );
    }

    #[test]
    fn apply_isolates_a_failing_write_and_migrates_the_rest() {
        // A repo whose schema constrains `provenance` to an enum excluding
        // `chat`: the via:chat write fails schema validation, but a
        // sibling needs-triage item still migrates. The failure is reported
        // on its own action, not raised as a whole-batch abort.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  provenance:\n    enum: [email]\n",
        )
        .unwrap();
        write_issue(
            tmp.path(),
            "good-item",
            "type: bug\nstatus: open\nlabels: [needs-triage]\n",
        );
        write_issue(
            tmp.path(),
            "bad-item",
            "type: bug\nstatus: open\nlabels: [via:chat]\n",
        );
        let r = migrate(tmp.path(), true).unwrap();
        assert_eq!(r.failed_count(), 1, "the chat write failed");
        assert_eq!(r.migrated_count(), 1, "the other item still migrated");
        // The good one is committed; the bad one is untouched (still open,
        // label intact) so a re-run retries it.
        assert_eq!(load(tmp.path(), "good-item").status, "untriaged");
        let bad = load(tmp.path(), "bad-item");
        assert_eq!(bad.status, "open");
        assert!(has_label(&bad, "via:chat"), "failed write left intact");
    }
}
