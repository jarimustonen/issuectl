//! Stale-issue detector. Surfaces active issues that have gone cold —
//! no frontmatter `updated` bump and no recent commit touching their
//! `item.md` within a threshold window. Long-running `in-progress`
//! issues are flagged specially: a WIP nobody has touched in weeks is
//! the highest-signal rot. Read-only; never mutates the store.

use std::path::Path;
use std::process::Command;

use chrono::{Local, NaiveDate};
use serde::Serialize;

use crate::models::Issue;

/// Default staleness window in days when `--days` is omitted.
pub const DEFAULT_STALE_DAYS: i64 = 30;

/// One issue judged stale, with the evidence behind the verdict.
#[derive(Debug, Clone, Serialize)]
pub struct StaleIssue {
    pub slug: String,
    pub status: String,
    pub assignee: Option<String>,
    /// Date (`YYYY-MM-DD`) of the most recent activity we could find.
    pub last_activity: String,
    /// Where `last_activity` came from: `updated` (frontmatter),
    /// `git` (last commit touching item.md), or `created`.
    pub source: &'static str,
    pub days_inactive: i64,
    /// `true` when the issue's status is `in-progress` — a stale WIP is
    /// worth surfacing ahead of stale-but-untouched `open` issues.
    pub in_progress: bool,
}

/// Result of a stale scan, sorted most-stale-first.
#[derive(Debug, Clone, Serialize)]
pub struct StaleReport {
    pub days: i64,
    pub stale: Vec<StaleIssue>,
}

/// Scan the repo for stale active issues. Closed/archived issues are
/// excluded — staleness is about active work going cold, not cold
/// storage. Uses today's local date as the anchor.
pub fn find_stale(repo_root: &Path, days: i64) -> StaleReport {
    let issues = crate::repo::load_issues(repo_root);
    find_stale_at(repo_root, &issues, days, Local::now().date_naive())
}

/// Testable core: takes the loaded issues and an explicit `today`
/// anchor. The git fallback still shells out, so unit tests cover the
/// frontmatter path (no git) by always supplying `updated`.
pub fn find_stale_at(
    repo_root: &Path,
    issues: &[Issue],
    days: i64,
    today: NaiveDate,
) -> StaleReport {
    let mut stale = Vec::new();
    for issue in issues {
        // Only active (open-bucket) issues can be stale.
        if issue.folder != "open" {
            continue;
        }
        let Some((last, source)) = last_activity(repo_root, issue) else {
            continue;
        };
        let days_inactive = (today - last).num_days();
        if days_inactive < days {
            continue;
        }
        stale.push(StaleIssue {
            slug: issue.slug.clone(),
            status: issue.status.clone(),
            assignee: issue.assignee.clone().or_else(|| issue.owner.clone()),
            last_activity: last.format("%Y-%m-%d").to_string(),
            source,
            days_inactive,
            in_progress: issue.status == "in-progress",
        });
    }
    // Most stale first; in-progress wins ties so WIP rot floats up.
    stale.sort_by(|a, b| {
        b.days_inactive
            .cmp(&a.days_inactive)
            .then(b.in_progress.cmp(&a.in_progress))
            .then(a.slug.cmp(&b.slug))
    });
    StaleReport { days, stale }
}

/// Best available last-activity date for an issue, with its provenance.
/// Preference order: frontmatter `updated` → last commit touching the
/// issue's `item.md` → frontmatter `created`. `None` when no signal
/// exists at all (issue can't be assessed and is omitted from the scan).
fn last_activity(repo_root: &Path, issue: &Issue) -> Option<(NaiveDate, &'static str)> {
    if let Some(d) = issue.updated.as_deref().and_then(parse_date) {
        return Some((d, "updated"));
    }
    if let Some(d) = git_last_commit_date(repo_root, &issue.slug) {
        return Some((d, "git"));
    }
    if let Some(d) = issue.created.as_deref().and_then(parse_date) {
        return Some((d, "created"));
    }
    None
}

/// Parse a `YYYY-MM-DD` date, tolerating an RFC3339 timestamp by taking
/// its date prefix. Shared with the archive path (age computation).
pub(crate) fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.get(0..10).unwrap_or(s), "%Y-%m-%d").ok()
}

/// Committer date (`YYYY-MM-DD`) of the most recent commit touching the
/// issue's `item.md`. Returns `None` outside a git repo, for an
/// uncommitted issue, or if the slug can't be located.
fn git_last_commit_date(repo_root: &Path, slug: &str) -> Option<NaiveDate> {
    let located = crate::repo::locate_issue_full(repo_root, slug).ok()?;
    let rel = located.item_path.strip_prefix(repo_root).ok()?;
    let out = Command::new("git")
        .args(["log", "-1", "--format=%cs", "--"])
        .arg(rel)
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    parse_date(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn issue(slug: &str, status: &str, updated: &str) -> Issue {
        Issue {
            slug: slug.to_string(),
            folder: if crate::issue_fields::is_closing_status(status) {
                "closed".to_string()
            } else {
                "open".to_string()
            },
            created: Some("2020-01-01".to_string()),
            status: status.to_string(),
            updated: Some(updated.to_string()),
            priority: "normal".to_string(),
            issue_type: "feature".to_string(),
            reporter: None,
            assignee: None,
            owner: None,
            epic: None,
            related: None,
            labels: None,
            closed: None,
            commits: None,
            title: slug.to_string(),
            body: String::new(),
            extra: BTreeMap::new(),
        }
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
    }

    #[test]
    fn flags_issues_past_threshold_and_skips_fresh() {
        let issues = vec![
            issue("old-open-fox", "open", "2026-01-01"),
            issue("fresh-open-owl", "open", "2026-05-30"),
        ];
        let report = find_stale_at(Path::new("/nonexistent"), &issues, 30, today());
        let slugs: Vec<&str> = report.stale.iter().map(|s| s.slug.as_str()).collect();
        assert_eq!(slugs, vec!["old-open-fox"]);
        assert_eq!(report.stale[0].source, "updated");
        assert!(report.stale[0].days_inactive > 30);
    }

    #[test]
    fn excludes_closed_issues() {
        let issues = vec![issue("done-old-elk", "fixed", "2020-01-01")];
        let report = find_stale_at(Path::new("/nonexistent"), &issues, 30, today());
        assert!(report.stale.is_empty(), "closed issues are never stale");
    }

    #[test]
    fn in_progress_flag_and_sort_order() {
        let issues = vec![
            issue("wip-old-stag", "in-progress", "2026-04-01"),
            issue("open-older-newt", "open", "2026-03-01"),
        ];
        let report = find_stale_at(Path::new("/nonexistent"), &issues, 30, today());
        // most-stale-first: the older `open` sorts ahead of the newer WIP
        assert_eq!(report.stale[0].slug, "open-older-newt");
        let wip = report.stale.iter().find(|s| s.slug == "wip-old-stag").unwrap();
        assert!(wip.in_progress);
        let plain = report
            .stale
            .iter()
            .find(|s| s.slug == "open-older-newt")
            .unwrap();
        assert!(!plain.in_progress);
    }

    #[test]
    fn parse_date_accepts_bare_and_rfc3339_prefix() {
        assert_eq!(parse_date("2026-05-06"), NaiveDate::from_ymd_opt(2026, 5, 6));
        assert_eq!(
            parse_date("2026-05-06T12:30:00Z"),
            NaiveDate::from_ymd_opt(2026, 5, 6)
        );
        assert!(parse_date("not-a-date").is_none());
    }
}
