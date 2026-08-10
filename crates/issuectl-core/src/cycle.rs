//! Linear-style lightweight cycles.
//!
//! A cycle is just an opaque label (typically an ISO week tag like
//! `2026-W22`) recorded on an issue via the optional `cycle:`
//! frontmatter key. There is no schema-side cycle catalog and no
//! start/end dates — the cycle is whatever string the team chose, and
//! the "current" cycle is computed from today's ISO week.
//!
//! Storage rides on `Issue.extra["cycle"]` rather than a typed field
//! on `Issue` so the wire and version-hash surface stay zero-delta
//! for repos that never adopt cycles. Sibling features (EST's
//! `--cycle <name>`, REPORT's per-cycle rollups) read through the
//! [`issue_cycle`] accessor below.
//!
//! The reverse mapping ([`group_by_cycle`]) and per-cycle counts
//! ([`status_for`]) feed `issuectl cycle status` and any future
//! report surface — keep the rollup logic here so there is one
//! authoritative source.

use std::collections::BTreeMap;

use chrono::{Datelike, Local, NaiveDate};
use serde_json::Value as JsonValue;

use crate::models::Issue;

/// The frontmatter key that holds an issue's cycle label.
pub const CYCLE_KEY: &str = "cycle";

/// Read the optional `cycle:` label off an issue. Returns `None`
/// when the field is missing, null, or not a string — callers do not
/// need to distinguish "absent" from "wrong type" here; the doctor
/// surface is the right place for that.
pub fn issue_cycle(issue: &Issue) -> Option<&str> {
    match issue.extra.get(CYCLE_KEY)? {
        JsonValue::String(s) if !s.is_empty() => Some(s.as_str()),
        _ => None,
    }
}

/// Format an ISO-week date as `YYYY-Www` (e.g. `2026-W22`). Matches
/// the example in the user-facing docs and is the format `cycle
/// current` prints.
pub fn iso_week_label(date: NaiveDate) -> String {
    let iso = date.iso_week();
    format!("{}-W{:02}", iso.year(), iso.week())
}

/// The cycle label for "today" in the local timezone.
pub fn current_cycle() -> String {
    iso_week_label(Local::now().date_naive())
}

/// Rollup counts for a single cycle.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct CycleStatus {
    pub cycle: String,
    pub open: usize,
    pub closed: usize,
    pub total: usize,
    /// Open-issue breakdown by `status` value. Sorted lexicographically.
    pub by_status: BTreeMap<String, usize>,
    /// Open-issue breakdown by `type`. Sorted lexicographically.
    pub by_type: BTreeMap<String, usize>,
}

/// Compute the [`CycleStatus`] for `cycle` across the given issues.
/// Issues whose `cycle:` label does not match are ignored.
pub fn status_for(issues: &[Issue], cycle: &str) -> CycleStatus {
    let mut out = CycleStatus {
        cycle: cycle.to_string(),
        ..CycleStatus::default()
    };
    for i in issues {
        if issue_cycle(i) != Some(cycle) {
            continue;
        }
        out.total += 1;
        if i.folder == "closed" {
            out.closed += 1;
        } else {
            out.open += 1;
            *out.by_status.entry(i.status.clone()).or_default() += 1;
            *out.by_type.entry(i.issue_type.clone()).or_default() += 1;
        }
    }
    out
}

/// Group issues by their cycle label, dropping issues without one.
/// The returned map is sorted by cycle name (BTreeMap), so callers
/// get a deterministic order suitable for both human and JSON output.
pub fn group_by_cycle(issues: &[Issue]) -> BTreeMap<String, Vec<&Issue>> {
    let mut out: BTreeMap<String, Vec<&Issue>> = BTreeMap::new();
    for i in issues {
        if let Some(c) = issue_cycle(i) {
            out.entry(c.to_string()).or_default().push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serde_json::json;

    fn mk(slug: &str, folder: &str, status: &str, cycle: Option<&str>) -> Issue {
        let mut extra = std::collections::BTreeMap::new();
        if let Some(c) = cycle {
            extra.insert(CYCLE_KEY.to_string(), json!(c));
        }
        Issue {
            slug: slug.into(),
            folder: folder.into(),
            created: None,
            status: status.into(),
            updated: None,
            priority: "normal".into(),
            issue_type: "bug".into(),
            reporter: None,
            assignee: None,
            owner: None,
            epic: None,
            related: None,
            labels: None,
            closed: None,
            closed_by: None,
            lane: None,
            collision: None,
            commits: None,
            title: slug.into(),
            body: String::new(),
            extra,
        }
    }

    #[test]
    fn iso_week_label_pads_week() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(); // Mon, ISO 2026-W02
        assert_eq!(iso_week_label(d), "2026-W02");
    }

    #[test]
    fn iso_week_label_crosses_year_boundary() {
        // 2025-12-29 (Mon) is ISO 2026-W01 — verifies we use ISO year, not calendar year.
        let d = NaiveDate::from_ymd_opt(2025, 12, 29).unwrap();
        assert_eq!(iso_week_label(d), "2026-W01");
    }

    #[test]
    fn issue_cycle_reads_extra() {
        let i = mk("a", "open", "open", Some("2026-W22"));
        assert_eq!(issue_cycle(&i), Some("2026-W22"));
        let i = mk("b", "open", "open", None);
        assert_eq!(issue_cycle(&i), None);
    }

    #[test]
    fn issue_cycle_ignores_empty_and_wrong_type() {
        let mut i = mk("a", "open", "open", None);
        i.extra
            .insert(CYCLE_KEY.into(), serde_json::Value::String("".into()));
        assert_eq!(issue_cycle(&i), None);
        i.extra.insert(CYCLE_KEY.into(), json!(42));
        assert_eq!(issue_cycle(&i), None);
        i.extra.insert(CYCLE_KEY.into(), json!(null));
        assert_eq!(issue_cycle(&i), None);
    }

    #[test]
    fn status_for_rolls_up_open_and_closed() {
        let issues = vec![
            mk("a", "open", "in-progress", Some("W1")),
            mk("b", "open", "open", Some("W1")),
            mk("c", "closed", "done", Some("W1")),
            mk("d", "open", "open", Some("W2")),
            mk("e", "open", "open", None),
        ];
        let s = status_for(&issues, "W1");
        assert_eq!(s.open, 2);
        assert_eq!(s.closed, 1);
        assert_eq!(s.total, 3);
        assert_eq!(s.by_status.get("open").copied(), Some(1));
        assert_eq!(s.by_status.get("in-progress").copied(), Some(1));
        // Closed issues do not show up in by_status / by_type — the
        // breakdown is open-only, matching `stats`.
        assert!(!s.by_status.contains_key("done"));
    }

    #[test]
    fn group_by_cycle_skips_uncycled() {
        let issues = vec![
            mk("a", "open", "open", Some("W1")),
            mk("b", "open", "open", Some("W2")),
            mk("c", "open", "open", None),
        ];
        let g = group_by_cycle(&issues);
        assert_eq!(g.len(), 2);
        assert_eq!(g.get("W1").map(|v| v.len()), Some(1));
        assert_eq!(g.get("W2").map(|v| v.len()), Some(1));
    }
}
