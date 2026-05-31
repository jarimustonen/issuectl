//! Lightweight estimates — optional `size: S|M|L|XL` or
//! `estimate: <number>` on an issue.
//!
//! Storage rides on `Issue.extra`, same trick as [`crate::cycle`] —
//! repos that never adopt estimates keep their wire/version-hash
//! surface zero-delta. Schema (`issues/.schema.yaml`) declares both
//! fields optional, with `size` constrained to the four-value enum
//! and `estimate` as a free-form scalar (numbers serialize as
//! strings through YAML → JSON, so we parse on read).
//!
//! Per the campaign brief, an issue may carry **either** a `size` or
//! an `estimate`, never both. Mixing is not a hard schema error
//! (the schema validates fields independently) but is surfaced as
//! [`mixed`] / [`mixed_issues`] for callers that want to flag it.
//! `issue_points` prefers a numeric `estimate` over `size` when both
//! are set so a downstream rollup stays deterministic.
//!
//! Sizes convert to story-point-style numbers via a fixed table
//! (S=1, M=3, L=5, XL=8). The mapping is intentionally hard-coded
//! in v1 — a schema-driven table would invite divergence across
//! repos and the per-cycle burndown gains nothing from it. Revisit
//! if real users ask for it.

use std::collections::BTreeMap;

use serde_json::Value as JsonValue;

use crate::cycle::issue_cycle;
use crate::models::Issue;

pub const SIZE_KEY: &str = "size";
pub const ESTIMATE_KEY: &str = "estimate";

/// Allowed values for the `size:` enum, in increasing magnitude.
pub const SIZE_VALUES: &[&str] = &["S", "M", "L", "XL"];

/// Story-point equivalent of each [`SIZE_VALUES`] entry. Fixed in
/// v1 — see module docs.
pub fn size_to_points(size: &str) -> Option<f64> {
    match size {
        "S" => Some(1.0),
        "M" => Some(3.0),
        "L" => Some(5.0),
        "XL" => Some(8.0),
        _ => None,
    }
}

/// Estimate carried by a single issue.
#[derive(Debug, Clone, PartialEq)]
pub enum Estimate {
    /// No `size:` and no `estimate:` field set.
    None,
    /// `size: S|M|L|XL` only.
    Size(String),
    /// `estimate: <number>` only.
    Points(f64),
}

impl Estimate {
    /// Numeric value (sizes converted via [`size_to_points`]). `None`
    /// only when [`Estimate::None`].
    pub fn points(&self) -> Option<f64> {
        match self {
            Estimate::None => None,
            Estimate::Size(s) => size_to_points(s),
            Estimate::Points(p) => Some(*p),
        }
    }
}

/// Read the `size:` field off an issue. Returns `None` if absent,
/// empty, or not a string. The value is NOT normalized — schema
/// validation rejects out-of-enum strings, so an unexpected value
/// here is surfaced upstream.
pub fn issue_size(issue: &Issue) -> Option<&str> {
    match issue.extra.get(SIZE_KEY)? {
        JsonValue::String(s) if !s.is_empty() => Some(s.as_str()),
        _ => None,
    }
}

/// Read the `estimate:` field off an issue as a `f64`. Accepts JSON
/// numbers and numeric strings (`"3"`, `"2.5"`) so frontmatter
/// authored as either YAML number or quoted string parses the same.
/// Negative or NaN values are rejected (treated as absent).
pub fn issue_estimate_points(issue: &Issue) -> Option<f64> {
    match issue.extra.get(ESTIMATE_KEY)? {
        JsonValue::Number(n) => {
            let f = n.as_f64()?;
            if f.is_finite() && f >= 0.0 {
                Some(f)
            } else {
                None
            }
        }
        JsonValue::String(s) => {
            let f = s.trim().parse::<f64>().ok()?;
            if f.is_finite() && f >= 0.0 {
                Some(f)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Resolve an issue's [`Estimate`]. When both `size` and `estimate`
/// are present, prefers `estimate` (numeric is more specific) — see
/// module docs. Callers wanting to flag the conflict should consult
/// [`mixed`] separately.
pub fn issue_estimate(issue: &Issue) -> Estimate {
    if let Some(p) = issue_estimate_points(issue) {
        return Estimate::Points(p);
    }
    if let Some(s) = issue_size(issue) {
        return Estimate::Size(s.to_string());
    }
    Estimate::None
}

/// True when an issue carries both `size:` and `estimate:`. The
/// schema allows either, not both per issue (campaign convention).
pub fn mixed(issue: &Issue) -> bool {
    issue_size(issue).is_some() && issue_estimate_points(issue).is_some()
}

/// Slugs of issues that violate the "either/or" rule.
pub fn mixed_issues(issues: &[Issue]) -> Vec<String> {
    issues
        .iter()
        .filter(|i| mixed(i))
        .map(|i| i.slug.clone())
        .collect()
}

/// One row of a `workload` rollup.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct WorkloadRow {
    /// Group key (assignee name, priority value, cycle label, epic slug).
    pub key: String,
    /// Number of issues in the group.
    pub count: usize,
    /// Sum of point-equivalents for the group. Sizes are mapped via
    /// [`size_to_points`]; missing estimates contribute 0.
    pub points: f64,
    /// Number of issues in the group with no `size`/`estimate` set —
    /// surfaces the "this rollup undercounts" case.
    pub unestimated: usize,
}

/// Rollup of open + in-progress workload across the four canonical
/// axes. Closed issues are excluded — they don't represent pending
/// load. "Open + in-progress" is implemented as `folder != "closed"`,
/// matching the rest of the CLI's open-issue convention.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Workload {
    pub total: usize,
    pub total_points: f64,
    pub unestimated: usize,
    /// Grouped by `assignee` (falls back to `owner`; empty string for
    /// unassigned issues, rendered as `(none)` upstream).
    pub by_assignee: Vec<WorkloadRow>,
    /// Grouped by `priority`.
    pub by_priority: Vec<WorkloadRow>,
    /// Grouped by `cycle` label. Issues without a cycle land in
    /// `(none)`.
    pub by_cycle: Vec<WorkloadRow>,
    /// Grouped by `epic` slug. Issues with no epic land in `(none)`.
    pub by_epic: Vec<WorkloadRow>,
}

fn bucket<'a, F>(open: &'a [&'a Issue], key_fn: F) -> Vec<WorkloadRow>
where
    F: Fn(&'a Issue) -> String,
{
    let mut groups: BTreeMap<String, WorkloadRow> = BTreeMap::new();
    for i in open {
        let key = key_fn(i);
        let est = issue_estimate(i);
        let entry = groups.entry(key.clone()).or_insert_with(|| WorkloadRow {
            key,
            ..Default::default()
        });
        entry.count += 1;
        match est.points() {
            Some(p) => entry.points += p,
            None => entry.unestimated += 1,
        }
    }
    // Sort: highest point load first, then highest count, then key
    // (so the table reads "biggest first").
    let mut out: Vec<_> = groups.into_values().collect();
    out.sort_by(|a, b| {
        b.points
            .partial_cmp(&a.points)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.count.cmp(&a.count))
            .then(a.key.cmp(&b.key))
    });
    out
}

/// Compute the [`Workload`] over `issues`. Excludes closed issues.
pub fn workload(issues: &[Issue]) -> Workload {
    let open: Vec<&Issue> = issues.iter().filter(|i| i.folder != "closed").collect();

    let mut total_points = 0.0_f64;
    let mut unestimated = 0;
    for i in &open {
        match issue_estimate(i).points() {
            Some(p) => total_points += p,
            None => unestimated += 1,
        }
    }

    Workload {
        total: open.len(),
        total_points,
        unestimated,
        by_assignee: bucket(&open, |i| {
            let a = i.effective_assignee();
            if a.is_empty() {
                "(none)".to_string()
            } else {
                a.to_string()
            }
        }),
        by_priority: bucket(&open, |i| i.priority.clone()),
        by_cycle: bucket(&open, |i| {
            issue_cycle(i)
                .map(|c| c.to_string())
                .unwrap_or_else(|| "(none)".to_string())
        }),
        by_epic: bucket(&open, |i| {
            i.epic.clone().unwrap_or_else(|| "(none)".to_string())
        }),
    }
}

// ── Burndown ────────────────────────────────────────────────────────────────

use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};

/// One day of the burndown chart.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct BurndownDay {
    /// ISO-8601 calendar date, formatted `YYYY-MM-DD`.
    #[serde(serialize_with = "serialize_date")]
    pub date: NaiveDate,
    /// Total in-scope points still open at end of `date`.
    pub remaining: f64,
    /// Linear ideal trend from `total` on day 0 to 0 on the last day.
    pub ideal: f64,
}

fn serialize_date<S: serde::Serializer>(d: &NaiveDate, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&d.format("%Y-%m-%d").to_string())
}

/// Full burndown payload for a cycle.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Burndown {
    pub cycle: String,
    /// Sum of in-scope point-equivalents at the start of the cycle.
    pub total: f64,
    /// Open-issue count in the cycle (informational; the per-day
    /// series uses points, not counts).
    pub scope_issues: usize,
    /// Issues in the cycle with no estimate (contribute 0 points).
    pub unestimated: usize,
    /// `true` when the cycle label parsed as an ISO week (`YYYY-Www`)
    /// and the date range was derived from it; `false` when we fell
    /// back to the earliest-created → today span. Callers should
    /// note the fallback in their output.
    pub iso_week: bool,
    #[serde(serialize_with = "serialize_date")]
    pub start: NaiveDate,
    #[serde(serialize_with = "serialize_date")]
    pub end: NaiveDate,
    pub days: Vec<BurndownDay>,
}

/// Parse `YYYY-Www` (e.g. `2026-W22`) into the Monday/Sunday bounds
/// of that ISO week. Returns `None` when the label doesn't parse or
/// the week number is out of range for the year.
pub fn parse_iso_week(label: &str) -> Option<(NaiveDate, NaiveDate)> {
    let (year, week) = label.split_once("-W")?;
    let year: i32 = year.parse().ok()?;
    let week: u32 = week.parse().ok()?;
    let start = NaiveDate::from_isoywd_opt(year, week, Weekday::Mon)?;
    let end = NaiveDate::from_isoywd_opt(year, week, Weekday::Sun)?;
    Some((start, end))
}

fn parse_ymd(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

/// Compute the burndown for `cycle` across `issues`. `today` is
/// passed in (rather than read from the wall clock) so callers can
/// test deterministically; pass `Local::now().date_naive()` in
/// production.
pub fn burndown_for(issues: &[Issue], cycle: &str, today: NaiveDate) -> Burndown {
    let in_scope: Vec<&Issue> = issues
        .iter()
        .filter(|i| issue_cycle(i) == Some(cycle))
        .collect();

    let (start, end, iso_week) = match parse_iso_week(cycle) {
        Some((s, e)) => (s, e, true),
        None => {
            // Fallback: span the earliest `created` date in scope
            // through `today`. A single-day span (or zero in-scope
            // issues) collapses to today..today so the renderer
            // still emits one row instead of an empty list.
            let earliest = in_scope
                .iter()
                .filter_map(|i| i.created.as_deref().and_then(parse_ymd))
                .min()
                .unwrap_or(today);
            let s = earliest.min(today);
            (s, today, false)
        }
    };

    // Precompute per-issue (points, close_date) once. The burndown loop
    // is O(scope * span_days); without precomputing, `issue_estimate`
    // walks `extra` and reallocates a `Size` string on every iteration.
    // Issues closed *before* the cycle start are excluded from `total`
    // entirely — otherwise Day 0 `remaining` would start visibly below
    // the ideal line on day 0 (they "completed" before the cycle began).
    let scope: Vec<(f64, Option<NaiveDate>, bool)> = in_scope
        .iter()
        .map(|i| {
            let pts = issue_estimate(i).points();
            // For a closed issue, fall back to `updated` then to the
            // cycle start: a closed-without-`closed:` issue counts as
            // done at the earliest representable moment rather than
            // floating forever in the burndown.
            let close_date = if i.folder == "closed" {
                Some(
                    i.closed
                        .as_deref()
                        .or(i.updated.as_deref())
                        .and_then(parse_ymd)
                        .unwrap_or(start),
                )
            } else {
                None
            };
            (pts.unwrap_or(0.0), close_date, pts.is_some())
        })
        .collect();
    let unestimated = scope.iter().filter(|(_, _, has)| !*has).count();
    let total: f64 = scope
        .iter()
        .filter(|(_, close, _)| close.map(|d| d >= start).unwrap_or(true))
        .map(|(p, _, _)| *p)
        .sum();

    let mut days = Vec::new();
    let span_days = (end - start).num_days().max(0) as usize;
    for offset in 0..=span_days {
        let day = start + Duration::days(offset as i64);
        let closed_pts: f64 = scope
            .iter()
            .filter(|(_, close, _)| close.map(|d| d >= start && d <= day).unwrap_or(false))
            .map(|(p, _, _)| *p)
            .sum();
        let remaining = (total - closed_pts).max(0.0);
        let ideal = if span_days == 0 {
            0.0
        } else {
            total * (1.0 - offset as f64 / span_days as f64)
        };
        days.push(BurndownDay {
            date: day,
            remaining,
            ideal,
        });
    }

    Burndown {
        cycle: cycle.to_string(),
        total,
        scope_issues: in_scope.len(),
        unestimated,
        iso_week,
        start,
        end,
        days,
    }
}

/// Wall-clock convenience used by the CLI.
pub fn burndown(issues: &[Issue], cycle: &str) -> Burndown {
    burndown_for(issues, cycle, Local::now().date_naive())
}

/// Render a Burndown as a fixed-width ASCII chart suitable for
/// terminal output. Columns: date, remaining bar, ideal marker,
/// trailing numerics.
pub fn render_ascii(b: &Burndown) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let header = if b.iso_week {
        format!("Burndown {}  (ISO week {} → {})\n", b.cycle, b.start, b.end)
    } else {
        format!(
            "Burndown {}  (no ISO week — span {} → {})\n",
            b.cycle, b.start, b.end
        )
    };
    out.push_str(&header);
    writeln!(
        out,
        "Scope: {} issues, {:.1} points  ({} unestimated)",
        b.scope_issues, b.total, b.unestimated
    )
    .ok();
    if b.days.is_empty() {
        return out;
    }
    let max = b.total.max(1.0);
    let bar_width: usize = 30;
    let bar_width_f = bar_width as f64;
    let last_slot = (bar_width - 1) as f64;
    for d in &b.days {
        let bar_len = (d.remaining / max * bar_width_f)
            .round()
            .clamp(0.0, bar_width_f) as usize;
        // Index, not length — clamp to the last char slot so the ideal
        // marker on the very last day lands inside the bar.
        let ideal_pos = (d.ideal / max * last_slot).round().clamp(0.0, last_slot) as usize;
        let mut bar = vec![' '; bar_width];
        for slot in bar.iter_mut().take(bar_len) {
            *slot = '#';
        }
        if ideal_pos < bar.len() {
            bar[ideal_pos] = if bar[ideal_pos] == '#' { '*' } else { '|' };
        }
        let weekday = match d.date.weekday() {
            Weekday::Mon => "Mon",
            Weekday::Tue => "Tue",
            Weekday::Wed => "Wed",
            Weekday::Thu => "Thu",
            Weekday::Fri => "Fri",
            Weekday::Sat => "Sat",
            Weekday::Sun => "Sun",
        };
        let bar_str: String = bar.into_iter().collect();
        writeln!(
            out,
            "{} {}  |{}| {:>5.1} / ideal {:>5.1}",
            d.date, weekday, bar_str, d.remaining, d.ideal
        )
        .ok();
    }
    out.push_str("Legend: # remaining, * intersect, | ideal marker.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cycle::CYCLE_KEY;
    use serde_json::json;

    fn mk(slug: &str) -> Issue {
        Issue {
            slug: slug.into(),
            folder: "open".into(),
            created: None,
            status: "open".into(),
            updated: None,
            priority: "normal".into(),
            issue_type: "task".into(),
            reporter: None,
            assignee: None,
            owner: None,
            epic: None,
            related: None,
            labels: None,
            closed: None,
            commits: None,
            title: slug.into(),
            body: String::new(),
            extra: BTreeMap::new(),
        }
    }

    fn with_size(mut i: Issue, size: &str) -> Issue {
        i.extra.insert(SIZE_KEY.into(), json!(size));
        i
    }

    fn with_estimate(mut i: Issue, est: f64) -> Issue {
        i.extra.insert(ESTIMATE_KEY.into(), json!(est));
        i
    }

    fn with_cycle(mut i: Issue, cycle: &str) -> Issue {
        i.extra.insert(CYCLE_KEY.into(), json!(cycle));
        i
    }

    #[test]
    fn size_to_points_covers_enum() {
        for v in SIZE_VALUES {
            assert!(size_to_points(v).is_some(), "{v} unmapped");
        }
        assert!(size_to_points("XXL").is_none());
    }

    #[test]
    fn issue_estimate_prefers_points_when_both_set() {
        let i = with_estimate(with_size(mk("a"), "L"), 2.0);
        // Mixing flagged separately, but the rollup is deterministic.
        assert!(mixed(&i));
        assert_eq!(issue_estimate(&i), Estimate::Points(2.0));
    }

    #[test]
    fn issue_estimate_reads_size_only() {
        let i = with_size(mk("a"), "M");
        assert_eq!(issue_estimate(&i), Estimate::Size("M".into()));
        assert_eq!(issue_estimate(&i).points(), Some(3.0));
        assert!(!mixed(&i));
    }

    #[test]
    fn issue_estimate_accepts_numeric_string() {
        let mut i = mk("a");
        i.extra.insert(ESTIMATE_KEY.into(), json!("2.5"));
        assert_eq!(issue_estimate(&i), Estimate::Points(2.5));
    }

    #[test]
    fn issue_estimate_rejects_negative_and_nan() {
        let mut i = mk("a");
        i.extra.insert(ESTIMATE_KEY.into(), json!(-1));
        assert_eq!(issue_estimate(&i), Estimate::None);
        i.extra.insert(ESTIMATE_KEY.into(), json!("nope"));
        assert_eq!(issue_estimate(&i), Estimate::None);
    }

    #[test]
    fn workload_excludes_closed_and_sums_points() {
        let mut a = with_size(mk("a"), "S"); // 1
        a.assignee = Some("ada".into());
        let mut b = with_estimate(mk("b"), 5.0);
        b.assignee = Some("ada".into());
        let mut c = with_size(mk("c"), "M"); // 3
        c.assignee = Some("ben".into());
        let mut d = with_estimate(mk("d"), 100.0);
        d.folder = "closed".into();
        let issues = vec![a, b, c, d];
        let w = workload(&issues);
        assert_eq!(w.total, 3);
        assert_eq!(w.total_points, 1.0 + 5.0 + 3.0);
        assert_eq!(w.unestimated, 0);
        // by_assignee sorted by points desc
        assert_eq!(w.by_assignee[0].key, "ada");
        assert_eq!(w.by_assignee[0].points, 6.0);
        assert_eq!(w.by_assignee[1].key, "ben");
        assert_eq!(w.by_assignee[1].points, 3.0);
    }

    #[test]
    fn workload_buckets_unassigned_and_no_cycle() {
        let issues = vec![mk("a"), with_cycle(mk("b"), "2026-W22")];
        let w = workload(&issues);
        assert_eq!(w.unestimated, 2);
        assert_eq!(w.by_assignee[0].key, "(none)");
        let cycles: Vec<_> = w.by_cycle.iter().map(|r| r.key.as_str()).collect();
        assert!(cycles.contains(&"(none)"));
        assert!(cycles.contains(&"2026-W22"));
    }

    #[test]
    fn parse_iso_week_round_trips() {
        let (s, e) = parse_iso_week("2026-W22").unwrap();
        assert_eq!(s.weekday(), Weekday::Mon);
        assert_eq!(e.weekday(), Weekday::Sun);
        assert_eq!((e - s).num_days(), 6);
        assert!(parse_iso_week("not-a-cycle").is_none());
        assert!(parse_iso_week("2026-W99").is_none());
    }

    #[test]
    fn burndown_iso_week_emits_seven_days() {
        let issues = vec![with_cycle(with_size(mk("a"), "M"), "2026-W22")];
        let today = NaiveDate::from_ymd_opt(2026, 5, 28).unwrap();
        let b = burndown_for(&issues, "2026-W22", today);
        assert!(b.iso_week);
        assert_eq!(b.days.len(), 7);
        assert_eq!(b.total, 3.0);
        // No closes → remaining stays at total throughout.
        for d in &b.days {
            assert_eq!(d.remaining, 3.0);
        }
        // Ideal: first = total, last = 0.
        assert_eq!(b.days.first().unwrap().ideal, 3.0);
        assert_eq!(b.days.last().unwrap().ideal, 0.0);
    }

    #[test]
    fn burndown_subtracts_closed_on_close_date() {
        let mut a = with_cycle(with_size(mk("a"), "M"), "2026-W22"); // 3 pts
        let mut b = with_cycle(with_size(mk("b"), "S"), "2026-W22"); // 1 pt
        b.folder = "closed".into();
        b.closed = Some("2026-05-27".into()); // Wed of W22 (Mon=2026-05-25)
        a.folder = "open".into();
        let issues = vec![a, b];
        let today = NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();
        let bd = burndown_for(&issues, "2026-W22", today);
        // Day 0 (Mon) — b still open → remaining 4
        assert_eq!(bd.days[0].remaining, 4.0);
        // Day 2 (Wed, closed date) — b closed → remaining 3
        assert_eq!(bd.days[2].remaining, 3.0);
        // Last day still 3 (a never closed)
        assert_eq!(bd.days.last().unwrap().remaining, 3.0);
    }

    #[test]
    fn burndown_non_iso_label_falls_back_to_created_span() {
        let mut a = with_cycle(with_size(mk("a"), "M"), "alpha");
        a.created = Some("2026-05-20".into());
        let issues = vec![a];
        let today = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
        let b = burndown_for(&issues, "alpha", today);
        assert!(!b.iso_week);
        assert_eq!(b.start, NaiveDate::from_ymd_opt(2026, 5, 20).unwrap());
        assert_eq!(b.end, today);
        assert_eq!(b.days.len(), 6);
    }

    #[test]
    fn burndown_empty_scope_still_renders_a_row() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
        let b = burndown_for(&[], "ghost", today);
        assert_eq!(b.scope_issues, 0);
        assert_eq!(b.total, 0.0);
        assert!(!b.days.is_empty());
    }

    #[test]
    fn render_ascii_includes_header_and_bars() {
        let issues = vec![with_cycle(with_size(mk("a"), "M"), "2026-W22")];
        let today = NaiveDate::from_ymd_opt(2026, 5, 28).unwrap();
        let b = burndown_for(&issues, "2026-W22", today);
        let s = render_ascii(&b);
        assert!(s.contains("Burndown 2026-W22"));
        assert!(s.contains("Scope: 1 issues"));
        // 7 day rows + header + scope line + legend ≥ 10 lines
        assert!(s.lines().count() >= 10);
    }

    #[test]
    fn burndown_excludes_issues_closed_before_cycle_start() {
        // A pre-cycle close shouldn't deflate Day 0 below the ideal —
        // its points are simply out of scope.
        let mut a = with_cycle(with_size(mk("a"), "M"), "2026-W22"); // 3 pts
        a.folder = "closed".into();
        a.closed = Some("2026-05-10".into()); // before W22 (Mon=2026-05-25)
        let b = with_cycle(with_size(mk("b"), "S"), "2026-W22"); // 1 pt, open
        let today = NaiveDate::from_ymd_opt(2026, 5, 28).unwrap();
        let bd = burndown_for(&[a, b], "2026-W22", today);
        assert_eq!(bd.total, 1.0);
        for d in &bd.days {
            assert_eq!(d.remaining, 1.0);
        }
    }

    #[test]
    fn burndown_closed_without_close_date_burns_on_start() {
        // A closed issue with no `closed:` stamp falls back to start
        // rather than floating forever in the chart.
        let mut a = with_cycle(with_size(mk("a"), "M"), "2026-W22"); // 3 pts
        a.folder = "closed".into();
        // No `closed:` field, no `updated:`.
        let today = NaiveDate::from_ymd_opt(2026, 5, 28).unwrap();
        let bd = burndown_for(&[a], "2026-W22", today);
        // Counts in total (close_date defaults to start, which is >= start)
        assert_eq!(bd.total, 3.0);
        // And is subtracted on day 0.
        assert_eq!(bd.days[0].remaining, 0.0);
    }

    #[test]
    fn mixed_issues_reports_offenders() {
        let a = with_estimate(with_size(mk("a"), "L"), 2.0);
        let b = with_size(mk("b"), "S");
        assert_eq!(mixed_issues(&[a, b]), vec!["a".to_string()]);
    }
}
