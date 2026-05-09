//! Declarative status-transition rules.
//!
//! Lives at `.issuectl/transitions.yaml` (sibling to `write.lock`).
//! Per-target-status preconditions enforced by the mutation layer
//! (write paths return `MutateError::TransitionViolation`, → 422).
//! `doctor` re-runs the same checks against existing issues and
//! surfaces them as *warnings* — those issues may be legacy data and
//! forcing their correction on read would create churn.
//!
//! Per-type required body sections live in `issues/.schema.yaml`
//! under a `body_sections:` key (see `crate::schema`). Body shape is
//! a structural declaration about what an issue *is*, so it sits
//! beside the frontmatter-shape rules; transition rules are workflow
//! and live separately.
//!
//! When the file is missing the loader returns empty defaults — i.e.
//! today's lenient behaviour. There is no built-in opinionated rule
//! set. Users opt in by writing the file.
//!
//! # File shape
//!
//! Status keys, `forbidden_from`, and `allowed_from` values must be
//! statuses declared in `issues/.schema.yaml`'s `status` enum (the
//! built-in default covers `open`, `in-progress`, `testing`, `done`,
//! `fixed`, `wontfix`, `duplicate`, `cannot-reproduce`, `obsolete`).
//!
//! ```yaml
//! version: 1
//! status_rules:
//!   done:
//!     requires_assignee: true
//!     requires_acceptance_criteria_checked: true
//!     # Either, or both — `allowed_from` is a whitelist (default-deny
//!     # when set); `forbidden_from` is a blacklist applied on top.
//!     allowed_from: [testing]
//!     forbidden_from: [open]
//!   testing:
//!     requires_assignee: true
//!   fixed:
//!     requires_commits: true
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::body_sections;
use crate::models::Issue;

pub const RULES_RELATIVE_PATH: &str = ".issuectl/transitions.yaml";
pub const SUPPORTED_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionRules {
    /// Explicit version. Required on disk — we deliberately do not
    /// `#[serde(default)]` it. Without an explicit declaration there
    /// is no way to evolve the schema later (today's missing-version
    /// files would silently re-bind to a future v2). Loading a file
    /// without `version:` is a parse error.
    pub version: u32,
    #[serde(default)]
    pub status_rules: BTreeMap<String, StatusRule>,
}

impl Default for TransitionRules {
    fn default() -> Self {
        // The in-memory default — used when `.issuectl/transitions.yaml`
        // is absent — must carry the supported version so any code that
        // reads `version` later (diagnostics, doctor, future migrations)
        // sees the same value as a freshly-parsed file. Deriving
        // `Default` would land on `version: 0`, which is a latent
        // contract bug.
        Self {
            version: SUPPORTED_VERSION,
            status_rules: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusRule {
    /// True when an issue must have a non-empty `assignee:` to enter
    /// this status. `owner:` (epics) satisfies the rule too — see
    /// `Issue::effective_assignee`. Documented here because the YAML
    /// key name says "assignee" but the rule reads "responsible
    /// party".
    #[serde(default)]
    pub requires_assignee: bool,
    /// Checks the body for an `## Acceptance Criteria` H2 section
    /// containing at least one `- [x]` (or `- [X]`) checkbox AND no
    /// remaining `- [ ]` checkboxes. Heading detection is fence-aware
    /// (a `## …` line inside a code fence is content, not a
    /// boundary), and so is the checkbox scan inside the section
    /// (`- [ ]` inside a fenced block does *not* count).
    #[serde(default)]
    pub requires_acceptance_criteria_checked: bool,
    /// `commits:` must be present and non-empty.
    #[serde(default)]
    pub requires_commits: bool,
    /// Whitelist of source statuses that may transition to this
    /// target. When set, default is *deny*: a transition from any
    /// status not in the list is rejected. When unset (empty),
    /// `allowed_from` does not constrain anything. Combined with
    /// `forbidden_from`, the deny rule wins (a status listed in both
    /// is rejected). `doctor` cannot evaluate this (no history) and
    /// skips it.
    #[serde(default)]
    pub allowed_from: Vec<String>,
    /// Disallow this target status when the previous status is in
    /// this list. Applied on top of `allowed_from` — useful for
    /// "carve out one specific source from an otherwise-allowed
    /// graph". `doctor` cannot evaluate this and skips it.
    #[serde(default)]
    pub forbidden_from: Vec<String>,
}

pub fn rules_path(root: &Path) -> PathBuf {
    root.join(RULES_RELATIVE_PATH)
}

/// Load rules from `.issuectl/transitions.yaml`. Returns empty defaults
/// when the file is missing (lenient — matches pre-feature behaviour).
///
/// `version` is mandatory in the YAML (no `#[serde(default)]`) so a
/// future v2 cannot silently rebind today's untagged files. Likewise,
/// each loaded rule is validated for empty/duplicated values.
pub fn load(root: &Path) -> Result<TransitionRules> {
    if let Some(cache) = crate::repo_config::current() {
        return Ok((*cache.rules(root)?).clone());
    }
    load_uncached(root)
}

/// Direct, unconditional parse. Used by `repo_config::RepoConfigCache`
/// to populate cache entries; recursing through `load` via the
/// thread-local would deadlock conceptually. Also the fallback when
/// no cache is active.
pub(crate) fn load_uncached(root: &Path) -> Result<TransitionRules> {
    let path = rules_path(root);
    if !path.is_file() {
        return Ok(TransitionRules::default());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let rules: TransitionRules = serde_yaml::from_str(&text)
        .with_context(|| format!("cannot parse {}", path.display()))?;
    if rules.version != SUPPORTED_VERSION {
        anyhow::bail!(
            "{}: unsupported version {} (this build supports {})",
            path.display(),
            rules.version,
            SUPPORTED_VERSION
        );
    }
    validate_loadable(&rules).with_context(|| format!("{}", path.display()))?;
    Ok(rules)
}

/// Cross-validate every status referenced in the rules against the
/// project's status enum (taken from `issues/.schema.yaml`). A typo
/// like `done` → `doen` silently no-ops the entire rule today;
/// likewise a typo in `allowed_from` denies every real transition
/// into the target. Both fail open in confusing ways. Failing at
/// load time gives the operator a precise pointer.
///
/// Callers (mutate.rs, doctor.rs, do_new) load the schema separately
/// and project its status enum through `valid_statuses` — keeps this
/// module independent of `crate::schema`'s shape so a future schema
/// rework doesn't ripple here.
pub fn validate_status_refs(
    rules: &TransitionRules,
    valid_statuses: &std::collections::BTreeSet<String>,
) -> Result<()> {
    for (target, rule) in &rules.status_rules {
        if !valid_statuses.contains(target) {
            anyhow::bail!(
                "status_rules.{target}: unknown status (not in schema's `status` enum)",
            );
        }
        for s in &rule.allowed_from {
            if !valid_statuses.contains(s) {
                anyhow::bail!(
                    "status_rules.{target}.allowed_from: unknown status {s:?} (not in schema's `status` enum)",
                );
            }
        }
        for s in &rule.forbidden_from {
            if !valid_statuses.contains(s) {
                anyhow::bail!(
                    "status_rules.{target}.forbidden_from: unknown status {s:?} (not in schema's `status` enum)",
                );
            }
        }
    }
    Ok(())
}

/// Reject configurations that are syntactically valid but semantically
/// nonsensical: empty status keys, empty / control-character
/// allowed_from / forbidden_from entries.
fn validate_loadable(rules: &TransitionRules) -> Result<()> {
    for (status, rule) in &rules.status_rules {
        if status.trim().is_empty() {
            anyhow::bail!("status_rules: status key cannot be empty");
        }
        for label in rule.allowed_from.iter().chain(rule.forbidden_from.iter()) {
            if label.trim().is_empty() {
                anyhow::bail!(
                    "status_rules.{status}: allowed_from / forbidden_from entry cannot be empty",
                );
            }
            if label.chars().any(|c| c.is_control()) {
                anyhow::bail!(
                    "status_rules.{status}: entry {label:?} contains a control character",
                );
            }
        }
    }
    Ok(())
}

/// Evaluate transition preconditions for a write path. `prev_status`
/// is the on-disk status before the mutation; the *target* status is
/// read from `issue_after.status` (eliminates a redundant parameter
/// that callers were already deriving from the same place).
///
/// Returns one human-readable message per violation. Empty vec ⇒
/// allowed. `requires_*` checks always run (a no-op PATCH against an
/// already-non-compliant `done` issue surfaces the violation —
/// arguably a feature: PATCH is the moment the user touched it). The
/// graph rules (`allowed_from` / `forbidden_from`) only apply when
/// `prev_status != new_status` — re-asserting the same status is not
/// a transition.
pub fn evaluate_transition(
    rules: &TransitionRules,
    issue_after: &Issue,
    prev_status: &str,
) -> Vec<String> {
    let new_status = issue_after.status.as_str();
    let mut out = Vec::new();
    let Some(rule) = rules.status_rules.get(new_status) else {
        return out;
    };
    if rule.requires_assignee && issue_after.effective_assignee().is_empty() {
        out.push(format!(
            "status {new_status:?} requires an assignee (or owner)"
        ));
    }
    if rule.requires_acceptance_criteria_checked {
        if let Some(msg) = acceptance_criteria_message(&issue_after.body, new_status) {
            out.push(msg);
        }
    }
    if rule.requires_commits
        && issue_after
            .commits
            .as_ref()
            .map(|c| c.is_empty())
            .unwrap_or(true)
    {
        out.push(format!(
            "status {new_status:?} requires at least one entry in `commits:` (use `update --add-commit`)"
        ));
    }
    if prev_status != new_status {
        if !rule.allowed_from.is_empty()
            && !rule.allowed_from.iter().any(|s| s == prev_status)
        {
            out.push(format!(
                "transition {prev_status:?} → {new_status:?} is not in `allowed_from` (allowed: [{}])",
                rule.allowed_from.join(", ")
            ));
        }
        if rule.forbidden_from.iter().any(|s| s == prev_status) {
            out.push(format!(
                "transition {prev_status:?} → {new_status:?} is forbidden by `forbidden_from`"
            ));
        }
    }
    out
}

/// Doctor-side variant of `evaluate_transition` — same `requires_*`
/// checks minus the graph rules (`allowed_from` / `forbidden_from`),
/// which need the prior status that doctor doesn't have. Surfaces
/// violations as warnings rather than errors so legacy data doesn't
/// block CI.
pub fn evaluate_existing(rules: &TransitionRules, issue: &Issue) -> Vec<String> {
    let mut out = Vec::new();
    let Some(rule) = rules.status_rules.get(&issue.status) else {
        return out;
    };
    if rule.requires_assignee && issue.effective_assignee().is_empty() {
        out.push(format!(
            "status {:?} requires an assignee (or owner)",
            issue.status
        ));
    }
    if rule.requires_acceptance_criteria_checked {
        if let Some(msg) = acceptance_criteria_message(&issue.body, &issue.status) {
            out.push(msg);
        }
    }
    if rule.requires_commits
        && issue.commits.as_ref().map(|c| c.is_empty()).unwrap_or(true)
    {
        out.push(format!(
            "status {:?} requires at least one entry in `commits:`",
            issue.status
        ));
    }
    out
}

/// Returns `Some(message)` when the `## Acceptance Criteria` section
/// fails the rule, or `None` when the section is present and every
/// task-list checkbox is checked. The message distinguishes the two
/// failure modes (no section / no checkboxes / unchecked items
/// remain) so users get an actionable error.
///
/// Heading detection and the in-section line walk are both
/// fence-aware: a `- [ ]` inside a fenced code block (` ``` ` /
/// `~~~`) is content, not a checklist item. Without this guard,
/// pasting an example into AC would falsely flag the rule.
fn acceptance_criteria_message(body: &str, status: &str) -> Option<String> {
    let Some(section) = body_sections::extract_section_text(body, "Acceptance Criteria") else {
        return Some(format!(
            "status {status:?} requires `## Acceptance Criteria` with at least one checked item, but the section is missing"
        ));
    };
    let mut total = 0usize;
    let mut checked = 0usize;
    let mut in_fence: Option<&'static str> = None;
    for line in section.lines() {
        let trimmed = line.trim_start();

        if let Some(marker) = in_fence {
            if trimmed.starts_with(marker) {
                in_fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            in_fence = Some("```");
            continue;
        }
        if trimmed.starts_with("~~~") {
            in_fence = Some("~~~");
            continue;
        }

        // Task-list shape: `- [ ]`, `- [x]`, `- [X]`, plus `*` / `+`
        // bullet variants. Intentionally not CommonMark-strict — the
        // rule is keyed on the shape users type when they want to
        // express a checklist.
        let bullet = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "));
        let Some(rest) = bullet else { continue };
        if let Some(inner) = rest.strip_prefix('[').and_then(|r| {
            r.split_once(']').and_then(|(c, _)| {
                if c.chars().count() == 1 {
                    Some(c)
                } else {
                    None
                }
            })
        }) {
            total += 1;
            if matches!(inner, "x" | "X") {
                checked += 1;
            }
        }
    }
    if total == 0 {
        return Some(format!(
            "status {status:?} requires `## Acceptance Criteria` to contain at least one task-list item (`- [ ]` / `- [x]`)"
        ));
    }
    if total != checked {
        return Some(format!(
            "status {status:?} requires every item in `## Acceptance Criteria` to be checked ({} of {} unchecked)",
            total - checked,
            total,
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_issue(status: &str, issue_type: &str, body: &str) -> Issue {
        Issue {
            slug: "x".into(),
            folder: "open".into(),
            created: None,
            status: status.into(),
            updated: None,
            priority: "normal".into(),
            issue_type: issue_type.into(),
            reporter: None,
            assignee: None,
            owner: None,
            epic: None,
            related: None,
            labels: None,
            closed: None,
            commits: None,
            extra: BTreeMap::new(),
            title: String::new(),
            body: body.into(),
        }
    }

    fn rule_with(spec: StatusRule) -> TransitionRules {
        let mut rules = TransitionRules::default();
        rules.status_rules.insert("done".into(), spec);
        rules
    }

    #[test]
    fn default_uses_supported_version() {
        // S2 fix: deriving Default would land on `version: 0`, which
        // would mismatch a freshly-loaded file. Lock the contract.
        assert_eq!(TransitionRules::default().version, SUPPORTED_VERSION);
    }

    #[test]
    fn load_returns_empty_when_missing() {
        let tmp = TempDir::new().unwrap();
        let r = load(tmp.path()).unwrap();
        assert!(r.status_rules.is_empty());
    }

    #[test]
    fn load_parses_yaml() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(
            tmp.path().join(RULES_RELATIVE_PATH),
            "version: 1\nstatus_rules:\n  done:\n    requires_assignee: true\n    forbidden_from: [open]\n    allowed_from: [testing]\n",
        )
        .unwrap();
        let r = load(tmp.path()).unwrap();
        assert!(r.status_rules["done"].requires_assignee);
        assert_eq!(r.status_rules["done"].forbidden_from, vec!["open"]);
        assert_eq!(r.status_rules["done"].allowed_from, vec!["testing"]);
    }

    #[test]
    fn load_rejects_unsupported_version() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(
            tmp.path().join(RULES_RELATIVE_PATH),
            "version: 999\nstatus_rules: {}\n",
        )
        .unwrap();
        assert!(load(tmp.path()).is_err());
    }

    #[test]
    fn load_rejects_missing_version() {
        // M13 fix: a future v2 must not silently rebind today's
        // untagged files. Without `#[serde(default)]` on `version`,
        // serde rejects this at parse time.
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(
            tmp.path().join(RULES_RELATIVE_PATH),
            "status_rules:\n  done:\n    requires_assignee: true\n",
        )
        .unwrap();
        assert!(load(tmp.path()).is_err(), "missing `version` must be rejected");
    }

    #[test]
    fn load_rejects_unknown_keys() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(
            tmp.path().join(RULES_RELATIVE_PATH),
            "version: 1\nstatus_rules:\n  done:\n    requries_assignee: true\n",
        )
        .unwrap();
        assert!(load(tmp.path()).is_err(), "typo'd key must be rejected");
    }

    #[test]
    fn load_rejects_empty_status_key() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(
            tmp.path().join(RULES_RELATIVE_PATH),
            "version: 1\nstatus_rules:\n  '':\n    requires_assignee: true\n",
        )
        .unwrap();
        assert!(load(tmp.path()).is_err());
    }

    #[test]
    fn evaluate_transition_flags_missing_assignee() {
        let rules = rule_with(StatusRule {
            requires_assignee: true,
            ..Default::default()
        });
        let i = make_issue("done", "bug", "");
        let v = evaluate_transition(&rules, &i, "in-progress");
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("assignee"));
    }

    #[test]
    fn evaluate_transition_owner_satisfies_assignee_requirement() {
        let rules = rule_with(StatusRule {
            requires_assignee: true,
            ..Default::default()
        });
        let mut i = make_issue("done", "epic", "");
        i.owner = Some("jari".into());
        assert!(evaluate_transition(&rules, &i, "in-progress").is_empty());
    }

    #[test]
    fn evaluate_transition_flags_forbidden_from() {
        let rules = rule_with(StatusRule {
            forbidden_from: vec!["open".into()],
            ..Default::default()
        });
        let i = make_issue("done", "bug", "");
        let v = evaluate_transition(&rules, &i, "open");
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("forbidden"));
        // `in-progress` → `done` is fine.
        assert!(evaluate_transition(&rules, &i, "in-progress").is_empty());
    }

    #[test]
    fn evaluate_transition_allowed_from_default_deny() {
        // C1: `allowed_from` is a whitelist — entries not in the list
        // are denied even though `forbidden_from` is empty.
        let rules = rule_with(StatusRule {
            allowed_from: vec!["testing".into()],
            ..Default::default()
        });
        let i = make_issue("done", "bug", "");
        let v = evaluate_transition(&rules, &i, "in-progress");
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("allowed_from"), "{:?}", v);
        assert!(evaluate_transition(&rules, &i, "testing").is_empty());
    }

    #[test]
    fn evaluate_transition_skips_graph_rules_on_noop() {
        // S1: a PATCH that re-asserts the current status is not a
        // transition; `forbidden_from` / `allowed_from` must not fire.
        let rules = rule_with(StatusRule {
            forbidden_from: vec!["done".into()],
            allowed_from: vec!["testing".into()],
            ..Default::default()
        });
        let i = make_issue("done", "bug", "");
        // prev == new == "done" — no graph violation expected.
        assert!(evaluate_transition(&rules, &i, "done").is_empty());
    }

    #[test]
    fn evaluate_transition_requires_commits() {
        let rules = rule_with(StatusRule {
            requires_commits: true,
            ..Default::default()
        });
        let mut i = make_issue("done", "bug", "");
        assert_eq!(evaluate_transition(&rules, &i, "in-progress").len(), 1);
        i.commits = Some(vec![crate::models::Commit {
            hash: "abc".into(),
            summary: "fix".into(),
        }]);
        assert!(evaluate_transition(&rules, &i, "in-progress").is_empty());
    }

    #[test]
    fn acceptance_criteria_messages_are_specific() {
        // Three distinct failure modes get three distinct messages.
        let m_missing = acceptance_criteria_message("", "done").unwrap();
        assert!(m_missing.contains("missing"));

        let m_no_items = acceptance_criteria_message(
            "## Acceptance Criteria\n\nProse only.\n",
            "done",
        )
        .unwrap();
        assert!(m_no_items.contains("at least one task-list item"));

        let m_unchecked = acceptance_criteria_message(
            "## Acceptance Criteria\n\n- [x] one\n- [ ] two\n",
            "done",
        )
        .unwrap();
        assert!(m_unchecked.contains("1 of 2 unchecked"));

        // All checked ⇒ no message.
        assert!(acceptance_criteria_message(
            "## Acceptance Criteria\n\n- [x] one\n- [X] two\n",
            "done"
        )
        .is_none());
    }

    #[test]
    fn acceptance_criteria_ignores_fenced_checkboxes() {
        // C2 fix: a checked-off real item plus an unchecked fenced
        // example must be considered satisfied.
        let body = "## Acceptance Criteria\n\n- [x] real\n\n```text\n- [ ] not real\n```\n";
        assert!(acceptance_criteria_message(body, "done").is_none());
        // Only the fenced item present ⇒ counts as zero items.
        let only_fenced = "## Acceptance Criteria\n\n```\n- [ ] inside\n```\n";
        let msg = acceptance_criteria_message(only_fenced, "done").unwrap();
        assert!(msg.contains("at least one task-list item"));
    }

    #[test]
    fn evaluate_existing_skips_graph_rules() {
        // `forbidden_from` / `allowed_from` are not enforced on
        // existing-issue scans — doctor has no prior status.
        let mut rules = TransitionRules::default();
        rules.status_rules.insert(
            "done".into(),
            StatusRule {
                forbidden_from: vec!["open".into()],
                allowed_from: vec!["never".into()],
                requires_assignee: true,
                ..Default::default()
            },
        );
        let i = make_issue("done", "bug", "");
        let v = evaluate_existing(&rules, &i);
        // Only the assignee message — the graph checks are skipped.
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("assignee"));
    }
}
