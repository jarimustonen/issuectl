//! Issue-domain enumerations: type, priority, and status sets, plus
//! status classification helpers. These describe the shape of issue
//! frontmatter — both the CLI (`main.rs`) and the domain modules
//! (`mutate`, `repo`, `doctor`, `schema`) consult them, so they live
//! at the top of the lib's dependency graph rather than at the binary
//! root.

pub const ISSUE_TYPES: &[&str] = &["bug", "task", "feature", "improvement", "chore", "epic"];
/// Intentionally two-valued: `high` = jumps the queue, `normal` = everything
/// else. The schema does NOT ship `low`/`medium`/`critical`; see the
/// "Priority enum is two-valued" decision in
/// `docs/design/frontmatter-schema.md`. Repos that need more gradations can
/// widen the enum in their `issues/.schema.yaml`.
pub const PRIORITIES: &[&str] = &["normal", "high"];
pub const ACTIVE_STATUSES: &[&str] = &["open", "in-progress", "testing"];
pub const CLOSING_STATUSES: &[&str] = &[
    "done",
    "fixed",
    "wontfix",
    "duplicate",
    "cannot-reproduce",
    "obsolete",
];

/// Concatenation of `ACTIVE_STATUSES` and `CLOSING_STATUSES`. Static so
/// callers can borrow without allocating a `Vec` per validation.
pub const ALL_STATUSES: &[&str] = &[
    "open",
    "in-progress",
    "testing",
    "done",
    "fixed",
    "wontfix",
    "duplicate",
    "cannot-reproduce",
    "obsolete",
];

pub fn all_statuses() -> &'static [&'static str] {
    ALL_STATUSES
}

pub fn is_closing_status(status: &str) -> bool {
    CLOSING_STATUSES.contains(&status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn is_closing_status_classifies_correctly() {
        for s in CLOSING_STATUSES {
            assert!(is_closing_status(s));
        }
        for s in ACTIVE_STATUSES {
            assert!(!is_closing_status(s));
        }
    }

    #[test]
    fn all_statuses_is_active_plus_closing_with_no_overlap() {
        let active: BTreeSet<_> = ACTIVE_STATUSES.iter().copied().collect();
        let closing: BTreeSet<_> = CLOSING_STATUSES.iter().copied().collect();
        assert!(
            active.is_disjoint(&closing),
            "ACTIVE and CLOSING must not overlap"
        );

        let all: BTreeSet<_> = ALL_STATUSES.iter().copied().collect();
        let union: BTreeSet<_> = active.union(&closing).copied().collect();
        assert_eq!(all, union, "ALL_STATUSES must equal ACTIVE ∪ CLOSING");
        assert_eq!(
            ALL_STATUSES.len(),
            ACTIVE_STATUSES.len() + CLOSING_STATUSES.len(),
            "ALL_STATUSES must contain no duplicates"
        );
    }
}
