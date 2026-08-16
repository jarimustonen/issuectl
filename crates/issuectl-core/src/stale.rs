//! Stale-issue detector. Surfaces active issues that have gone cold —
//! no frontmatter `updated` bump and no recent commit touching their
//! `item.md` within a threshold window. Long-running `in-progress`
//! issues are flagged specially: a WIP nobody has touched in weeks is
//! the highest-signal rot. Read-only; never mutates the store.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::NaiveDate;
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
    find_stale_via(repo_root, days, &crate::clock::SystemClock)
}

/// Clock-injected variant of [`find_stale`].
pub fn find_stale_via(repo_root: &Path, days: i64, clock: &dyn crate::clock::Clock) -> StaleReport {
    let issues = crate::repo::load_issues(repo_root);
    find_stale_at(repo_root, &issues, days, clock.today())
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
    // One git walk yields last-commit dates for every issue that needs
    // the git fallback, instead of one `git log` process per issue.
    let git_dates = batch_git_last_commit_dates(repo_root, issues);
    let mut stale = Vec::new();
    for issue in issues {
        // Only active (open-bucket) issues can be stale.
        if issue.folder != "open" {
            continue;
        }
        let Some((last, source)) = last_activity(issue, &git_dates) else {
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
/// `git_dates` is the precomputed slug → last-commit-date map from
/// [`batch_git_last_commit_dates`]; only issues lacking a usable
/// `updated` appear in it.
fn last_activity(
    issue: &Issue,
    git_dates: &HashMap<String, NaiveDate>,
) -> Option<(NaiveDate, &'static str)> {
    if let Some(d) = issue.updated.as_deref().and_then(parse_date) {
        return Some((d, "updated"));
    }
    if let Some(d) = git_dates.get(&issue.slug) {
        return Some((*d, "git"));
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

/// Committer dates (`YYYY-MM-DD`) of the most recent commit touching each
/// issue's `item.md`, keyed by slug. Computed with a single `git log`
/// walk over all relevant paths instead of one process per issue:
/// `git log` reports commits newest-first, so the first time a path
/// appears in the `--name-only` output is its latest commit.
///
/// Only open-bucket issues lacking a usable frontmatter `updated` are
/// queried — exactly the set that would fall through to the git
/// fallback in [`last_activity`]. Returns an empty map outside a git
/// repo (the walk fails or finds nothing), so callers fall back to
/// `created`, matching the per-issue behavior this replaced.
fn batch_git_last_commit_dates(repo_root: &Path, issues: &[Issue]) -> HashMap<String, NaiveDate> {
    // Map each candidate's repo-relative item.md path back to its slug.
    let mut slug_by_rel: HashMap<PathBuf, String> = HashMap::new();
    for issue in issues {
        if issue.folder != "open" {
            continue;
        }
        if issue.updated.as_deref().and_then(parse_date).is_some() {
            continue;
        }
        let Ok(located) = crate::repo::locate_issue_full(repo_root, &issue.slug) else {
            continue;
        };
        let Ok(rel) = located.item_path.strip_prefix(repo_root) else {
            continue;
        };
        slug_by_rel.insert(rel.to_path_buf(), issue.slug.clone());
    }
    if slug_by_rel.is_empty() {
        return HashMap::new();
    }

    let mut cmd = Command::new("git");
    cmd.args(["log", "--format=%cs", "--name-only"])
        .arg("--")
        .current_dir(repo_root);
    for rel in slug_by_rel.keys() {
        cmd.arg(rel);
    }
    let Ok(out) = cmd.output() else {
        return HashMap::new();
    };
    if !out.status.success() {
        return HashMap::new();
    }

    // The pathspec restricts `--name-only` to our item.md paths, so
    // every non-date line is one of them. Walk newest-first and keep the
    // first date seen per path — that is its latest commit.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut dates: HashMap<String, NaiveDate> = HashMap::new();
    let mut current: Option<NaiveDate> = None;
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(slug) = slug_by_rel.get(Path::new(line)) {
            if let Some(d) = current {
                dates.entry(slug.clone()).or_insert(d);
            }
        } else if line.len() == 10 {
            if let Some(d) = parse_date(line) {
                current = Some(d);
            }
        }
    }
    dates
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
            closed_by: None,
            lane: None,
            collision: None,
            lane_seq: None,
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
        let wip = report
            .stale
            .iter()
            .find(|s| s.slug == "wip-old-stag")
            .unwrap();
        assert!(wip.in_progress);
        let plain = report
            .stale
            .iter()
            .find(|s| s.slug == "open-older-newt")
            .unwrap();
        assert!(!plain.in_progress);
    }

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?} failed");
    }

    fn fresh_repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-q", "-b", "main"]);
        for (k, v) in [("user.email", "t@example.com"), ("user.name", "t")] {
            git(tmp.path(), &["config", "--local", k, v]);
        }
        tmp
    }

    fn seed_item(root: &Path, slug: &str) {
        let dir = root.join("issues").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        let body = format!(
            "---\ncreated: 2020-01-01\ntype: feature\nstatus: open\npriority: normal\n---\n\n# {slug}\n\nbody\n"
        );
        std::fs::write(dir.join("item.md"), body).unwrap();
    }

    fn commit_with_date(root: &Path, msg: &str, date: &str) {
        git(root, &["add", "."]);
        let env_date = format!("{date}T12:00:00");
        let st = Command::new("git")
            .args(["commit", "-q", "-m", msg])
            .env("GIT_AUTHOR_DATE", &env_date)
            .env("GIT_COMMITTER_DATE", &env_date)
            .current_dir(root)
            .status()
            .unwrap();
        assert!(st.success(), "git commit failed");
    }

    fn issue_no_updated(slug: &str) -> Issue {
        let mut i = issue(slug, "open", "2020-01-01");
        i.updated = None;
        i
    }

    #[test]
    fn batches_git_fallback_into_one_call_per_scan() {
        // Two issues lacking frontmatter `updated` get distinct
        // last-commit dates from a single batched git log walk.
        let tmp = fresh_repo();
        let root = tmp.path();
        seed_item(root, "alpha-old-fox");
        commit_with_date(root, "add alpha", "2025-01-15");
        seed_item(root, "beta-new-owl");
        commit_with_date(root, "add beta", "2026-05-01");

        let issues = vec![
            issue_no_updated("alpha-old-fox"),
            issue_no_updated("beta-new-owl"),
        ];
        let report = find_stale_at(root, &issues, 30, today());
        let by_slug: HashMap<_, _> = report.stale.iter().map(|s| (s.slug.as_str(), s)).collect();
        let alpha = by_slug["alpha-old-fox"];
        assert_eq!(alpha.source, "git");
        assert_eq!(alpha.last_activity, "2025-01-15");
        let beta = by_slug["beta-new-owl"];
        assert_eq!(beta.source, "git");
        assert_eq!(beta.last_activity, "2026-05-01");
    }

    #[test]
    fn batch_picks_latest_commit_per_path() {
        // Multiple commits on the same item.md: the batch must keep the
        // most recent date, just like the per-issue `-1` did.
        let tmp = fresh_repo();
        let root = tmp.path();
        seed_item(root, "twice-touched-elk");
        commit_with_date(root, "create", "2025-02-01");
        std::fs::write(
            root.join("issues/twice-touched-elk/item.md"),
            "---\ncreated: 2020-01-01\ntype: feature\nstatus: open\npriority: normal\n---\n\n# twice-touched-elk\n\nedited\n",
        )
        .unwrap();
        commit_with_date(root, "edit", "2026-03-10");

        let issues = vec![issue_no_updated("twice-touched-elk")];
        let report = find_stale_at(root, &issues, 30, today());
        assert_eq!(report.stale.len(), 1);
        assert_eq!(report.stale[0].source, "git");
        assert_eq!(report.stale[0].last_activity, "2026-03-10");
    }

    #[test]
    fn parse_date_accepts_bare_and_rfc3339_prefix() {
        assert_eq!(
            parse_date("2026-05-06"),
            NaiveDate::from_ymd_opt(2026, 5, 6)
        );
        assert_eq!(
            parse_date("2026-05-06T12:30:00Z"),
            NaiveDate::from_ymd_opt(2026, 5, 6)
        );
        assert!(parse_date("not-a-date").is_none());
    }
}
