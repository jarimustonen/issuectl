//! Git-derived reporting: activity, per-issue timeline, changelog,
//! and lightweight metrics. The git history is the event log — there
//! is no separate event database.
//!
//! Caveat: rebases, squashes, and history rewrites can reshape what
//! `git log` shows. Where an issue carries `created:` / `closed:` in
//! its frontmatter, those values are authoritative for metrics;
//! commit timestamps are the fallback when frontmatter is missing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDate;
use serde::Serialize;

use crate::git_trailers;
use crate::models::Issue;

/// Parse a `--since` value as `<N>` or `<N>d`. Returns the day count.
pub fn parse_since_days(s: &str) -> Result<i64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        bail!("--since cannot be empty");
    }
    let digits = trimmed.strip_suffix('d').unwrap_or(trimmed);
    let n: i64 = digits
        .parse()
        .map_err(|_| anyhow!("expected a day count like `7` or `30d`, got {s:?}"))?;
    if n < 0 {
        bail!("--since cannot be negative: {s:?}");
    }
    Ok(n)
}

// ─── activity ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ActivityEntry {
    /// Committer date (`YYYY-MM-DD`).
    pub date: String,
    /// Abbreviated commit SHA.
    pub sha: String,
    pub author: String,
    pub summary: String,
    /// Issue slugs touched by this commit, derived from the file paths
    /// (`issues/<...>/<slug>/item.md`).
    pub slugs: Vec<String>,
}

/// Recent commits that touched issue files. Walks
/// `git log [--since=...] --name-only -- issues/` once and groups the
/// affected `item.md` paths back to slugs.
pub fn activity(
    repo_root: &Path,
    since_days: Option<i64>,
    limit: Option<usize>,
) -> Result<Vec<ActivityEntry>> {
    let mut cmd = Command::new("git");
    cmd.args(["log", "--format=%x01%H%x09%cs%x09%an%x09%s", "--name-only"]);
    if let Some(d) = since_days {
        cmd.arg(format!("--since={d}.days.ago"));
    }
    cmd.arg("--").arg("issues/");
    cmd.current_dir(repo_root);
    let out = cmd.output().with_context(|| "running `git log`")?;
    if !out.status.success() {
        bail!(
            "`git log` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();

    let mut entries: Vec<ActivityEntry> = Vec::new();
    let mut current: Option<ActivityEntry> = None;
    let mut current_slugs: BTreeSet<String> = BTreeSet::new();
    for line in text.lines() {
        if let Some(header) = line.strip_prefix('\x01') {
            if let Some(mut e) = current.take() {
                e.slugs = current_slugs.iter().cloned().collect();
                if !e.slugs.is_empty() {
                    entries.push(e);
                }
            }
            current_slugs.clear();
            let mut parts = header.splitn(4, '\t');
            let hash = parts.next().unwrap_or("").to_string();
            let date = parts.next().unwrap_or("").to_string();
            let author = parts.next().unwrap_or("").to_string();
            let summary = parts.next().unwrap_or("").to_string();
            current = Some(ActivityEntry {
                date,
                sha: hash.chars().take(12).collect(),
                author,
                summary,
                slugs: Vec::new(),
            });
        } else if !line.is_empty() {
            if let Some(slug) = slug_from_path(line) {
                current_slugs.insert(slug);
            }
        }
    }
    if let Some(mut e) = current.take() {
        e.slugs = current_slugs.iter().cloned().collect();
        if !e.slugs.is_empty() {
            entries.push(e);
        }
    }
    if let Some(n) = limit {
        entries.truncate(n);
    }
    Ok(entries)
}

/// Extract the slug from an `issues/.../<slug>/item.md` path. Returns
/// `None` for any other file (attachments, README, etc.).
fn slug_from_path(path: &str) -> Option<String> {
    let p = Path::new(path);
    if p.file_name()?.to_str()? != "item.md" {
        return None;
    }
    let parent = p.parent()?;
    let slug = parent.file_name()?.to_str()?.to_string();
    if !crate::slug::is_valid(&slug) {
        return None;
    }
    Some(slug)
}

// ─── timeline ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TimelineEvent {
    pub date: String,
    pub sha: String,
    pub author: String,
    pub summary: String,
    /// `None` on the creation commit, `Some(prev)` thereafter.
    pub prev_status: Option<String>,
    pub status: String,
}

/// Reconstruct status transitions for one issue from its git history.
/// Uses `git log --follow -p` on the issue's `item.md` so rename moves
/// (open/ → closed/, archive moves) don't break continuity.
///
/// History rewrites (rebase, squash) reshape what shows up here;
/// frontmatter timestamps are authoritative when the two disagree.
pub fn timeline(repo_root: &Path, slug: &str) -> Result<Vec<TimelineEvent>> {
    // Validate slug shape and resolve any prefix so the pathspec is
    // unambiguous. We don't actually use the located path — instead
    // we let git find every layout the file has lived under via a
    // glob pathspec. `--follow` only works with a single path and
    // misses pure-rename commits, which is exactly when the layout
    // moved (open/ → flat → archive); the glob covers them.
    let slug = crate::repo::resolve_slug_input(repo_root, slug)?;
    if !crate::slug::is_valid(&slug) {
        bail!("invalid slug: {slug:?}");
    }
    let pathspec = format!(":(glob)issues/**/{slug}/item.md");

    let mut cmd = Command::new("git");
    cmd.args([
        "log",
        "--reverse",
        "--format=%x01%H%x09%cs%x09%an%x09%s",
        "-p",
        "--unified=0",
        "--no-color",
        "--",
    ]);
    cmd.arg(&pathspec).current_dir(repo_root);

    let out = cmd.output().with_context(|| "running `git log`")?;
    if !out.status.success() {
        bail!(
            "`git log` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();

    let mut events: Vec<TimelineEvent> = Vec::new();
    let mut header: Option<(String, String, String, String)> = None;
    let mut added: Option<String> = None;
    let mut removed: Option<String> = None;

    let flush = |header: &mut Option<(String, String, String, String)>,
                 added: &mut Option<String>,
                 removed: &mut Option<String>,
                 events: &mut Vec<TimelineEvent>| {
        if let Some((sha, date, author, summary)) = header.take() {
            let new_status = added.take();
            let old_status = removed.take();
            if let Some(new_s) = new_status {
                // Only record commits that actually changed status (or
                // that created the file). The creation commit has no
                // `-status:` line and yields prev_status = None.
                if old_status.as_deref() != Some(new_s.as_str()) {
                    events.push(TimelineEvent {
                        date,
                        sha: sha.chars().take(12).collect(),
                        author,
                        summary,
                        prev_status: old_status,
                        status: new_s,
                    });
                }
            }
        }
    };

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('\x01') {
            flush(&mut header, &mut added, &mut removed, &mut events);
            let mut parts = rest.splitn(4, '\t');
            header = Some((
                parts.next().unwrap_or("").to_string(),
                parts.next().unwrap_or("").to_string(),
                parts.next().unwrap_or("").to_string(),
                parts.next().unwrap_or("").to_string(),
            ));
        } else if line.starts_with("+++") || line.starts_with("---") {
            // file header markers — not real adds/removes
        } else if let Some(v) = line.strip_prefix("+status:") {
            added = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("-status:") {
            removed = Some(v.trim().to_string());
        }
    }
    flush(&mut header, &mut added, &mut removed, &mut events);

    Ok(events)
}

// ─── changelog ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ChangelogEntry {
    pub slug: String,
    pub title: String,
    pub issue_type: String,
    pub labels: Vec<String>,
    pub status: String,
    pub commits: Vec<ChangelogCommit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangelogCommit {
    pub sha: String,
    pub summary: String,
    /// `true` when the commit's `Fixes-Issue:` trailer pinned the issue
    /// (vs. a `Refs-Issue:` mention).
    pub fixes: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangelogReport {
    pub range: String,
    /// Slugs whose closed-date or trailer placed them in the range,
    /// grouped by issue `type`. Order within a group is by closed date
    /// then slug.
    pub groups: BTreeMap<String, Vec<ChangelogEntry>>,
    /// Commits in range with one or more `Refs-Issue` / `Fixes-Issue`
    /// trailers, but whose slugs didn't resolve to an issue file (e.g.
    /// issue deleted). Surfaced so the report isn't silently lossy.
    pub orphan_commits: Vec<ChangelogCommit>,
}

/// Walk `git log <range>` for `Refs-Issue:` / `Fixes-Issue:` trailers,
/// group by referenced issue, and produce a structured changelog.
pub fn changelog(repo_root: &Path, range: &str, issues: &[Issue]) -> Result<ChangelogReport> {
    let commits = git_trailers::parse_log(repo_root, range)?;
    let by_slug: BTreeMap<&str, &Issue> = issues.iter().map(|i| (i.slug.as_str(), i)).collect();

    let mut per_issue: BTreeMap<String, ChangelogEntry> = BTreeMap::new();
    let mut orphans: Vec<ChangelogCommit> = Vec::new();
    for c in commits {
        if !c.has_any_trailer() {
            continue;
        }
        let short = ChangelogCommit {
            sha: c.hash.chars().take(12).collect(),
            summary: c.summary.clone(),
            fixes: !c.fixes_issue.is_empty(),
        };
        let mut matched_any = false;
        for slug in c.fixes_issue.iter().chain(c.refs_issue.iter()) {
            let Some(issue) = by_slug.get(slug.as_str()) else {
                continue;
            };
            matched_any = true;
            let entry = per_issue
                .entry(slug.clone())
                .or_insert_with(|| ChangelogEntry {
                    slug: slug.clone(),
                    title: issue.title.clone(),
                    issue_type: issue.issue_type.clone(),
                    labels: issue.labels.clone().unwrap_or_default(),
                    status: issue.status.clone(),
                    commits: Vec::new(),
                });
            let fixes_here = c.fixes_issue.iter().any(|s| s == slug);
            let mut cc = short.clone();
            cc.fixes = fixes_here;
            if !entry.commits.iter().any(|x| x.sha == cc.sha) {
                entry.commits.push(cc);
            }
        }
        if !matched_any {
            orphans.push(short);
        }
    }

    let mut groups: BTreeMap<String, Vec<ChangelogEntry>> = BTreeMap::new();
    for (_, entry) in per_issue {
        groups
            .entry(entry.issue_type.clone())
            .or_default()
            .push(entry);
    }
    for v in groups.values_mut() {
        v.sort_by(|a, b| a.slug.cmp(&b.slug));
    }

    Ok(ChangelogReport {
        range: range.to_string(),
        groups,
        orphan_commits: orphans,
    })
}

/// Render a [`ChangelogReport`] as markdown. Heading shape:
///
/// ```text
/// ## Changelog <range>
///
/// ### feature
/// - **slug** — title (status) — abc1234 short, def5678 short
/// ```
pub fn render_changelog_markdown(report: &ChangelogReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("## Changelog {}\n\n", report.range));
    if report.groups.is_empty() {
        out.push_str("_No issues referenced by commits in this range._\n");
    }
    for (issue_type, entries) in &report.groups {
        out.push_str(&format!("### {issue_type}\n\n"));
        for e in entries {
            let commit_blurb = e
                .commits
                .iter()
                .map(|c| {
                    if c.fixes {
                        format!("{} (fixes)", c.sha)
                    } else {
                        c.sha.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "- **{}** — {} _({})_ — {}\n",
                e.slug, e.title, e.status, commit_blurb
            ));
        }
        out.push('\n');
    }
    if !report.orphan_commits.is_empty() {
        out.push_str("### Orphan commits\n\n");
        out.push_str("_Trailers reference an unknown slug (issue may have been deleted)._\n\n");
        for c in &report.orphan_commits {
            out.push_str(&format!("- {} — {}\n", c.sha, c.summary));
        }
    }
    out
}

// ─── metrics ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct MetricsReport {
    pub since_days: Option<i64>,
    /// Number of issues closed in the window (or total when no window).
    pub throughput: usize,
    /// Median / p90 cycle time in whole days, computed across issues
    /// closed in the window. `None` when the sample is empty.
    pub cycle_time_days: Option<CycleTimeStats>,
    /// Per-assignee open-issue count (open folder + effective assignee).
    pub workload_by_assignee: BTreeMap<String, usize>,
    /// Per-assignee throughput in the window.
    pub closed_by_assignee: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CycleTimeStats {
    pub sample: usize,
    pub median: i64,
    pub p90: i64,
    pub mean: f64,
}

/// Compute lightweight metrics from the in-repo issue set. Cycle time
/// is derived from frontmatter `created:` / `closed:` — the authoritative
/// fields when git history has been rewritten.
/// Convenience: same as [`metrics`] but uses today's local date as
/// the reference point.
pub fn metrics_today(issues: &[Issue], since_days: Option<i64>) -> MetricsReport {
    metrics(issues, since_days, chrono::Local::now().date_naive())
}

pub fn metrics(issues: &[Issue], since_days: Option<i64>, today: NaiveDate) -> MetricsReport {
    let window_start = since_days.map(|d| today - chrono::Duration::days(d));

    let mut cycle_samples: Vec<i64> = Vec::new();
    let mut closed_by_assignee: BTreeMap<String, usize> = BTreeMap::new();
    let mut throughput = 0usize;

    for i in issues {
        let Some(closed_str) = i.closed.as_deref() else {
            continue;
        };
        let Some(closed) = parse_date(closed_str) else {
            continue;
        };
        if let Some(ws) = window_start {
            if closed < ws {
                continue;
            }
        }
        throughput += 1;
        let assignee = i.effective_assignee();
        let key = if assignee.is_empty() {
            "(none)".to_string()
        } else {
            assignee.to_string()
        };
        *closed_by_assignee.entry(key).or_default() += 1;
        if let Some(created) = i.created.as_deref().and_then(parse_date) {
            let days = (closed - created).num_days();
            if days >= 0 {
                cycle_samples.push(days);
            }
        }
    }

    let mut workload_by_assignee: BTreeMap<String, usize> = BTreeMap::new();
    for i in issues {
        if i.folder != "open" {
            continue;
        }
        let a = i.effective_assignee();
        let key = if a.is_empty() {
            "(none)".to_string()
        } else {
            a.to_string()
        };
        *workload_by_assignee.entry(key).or_default() += 1;
    }

    let cycle_time_days = if cycle_samples.is_empty() {
        None
    } else {
        cycle_samples.sort();
        let n = cycle_samples.len();
        let median = cycle_samples[n / 2];
        let p90_idx = ((n as f64) * 0.9).ceil() as usize - 1;
        let p90 = cycle_samples[p90_idx.min(n - 1)];
        let sum: i64 = cycle_samples.iter().sum();
        let mean = (sum as f64) / (n as f64);
        Some(CycleTimeStats {
            sample: n,
            median,
            p90,
            mean,
        })
    };

    MetricsReport {
        since_days,
        throughput,
        cycle_time_days,
        workload_by_assignee,
        closed_by_assignee,
    }
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.get(0..10).unwrap_or(s), "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_issue(slug: &str, status: &str, created: &str, closed: Option<&str>) -> Issue {
        Issue {
            slug: slug.to_string(),
            folder: if crate::issue_fields::is_closing_status(status) {
                "closed".into()
            } else {
                "open".into()
            },
            created: Some(created.to_string()),
            status: status.to_string(),
            updated: None,
            priority: "normal".to_string(),
            issue_type: "feature".to_string(),
            reporter: None,
            assignee: Some("alice".to_string()),
            owner: None,
            epic: None,
            related: None,
            labels: None,
            closed: closed.map(|s| s.to_string()),
            closed_by: None,
            lane: None,
            collision: None,
            lane_seq: None,
            commits: None,
            title: format!("title for {slug}"),
            body: String::new(),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn parse_since_accepts_d_suffix_and_bare_int() {
        assert_eq!(parse_since_days("7").unwrap(), 7);
        assert_eq!(parse_since_days("30d").unwrap(), 30);
        assert!(parse_since_days("").is_err());
        assert!(parse_since_days("-1").is_err());
        assert!(parse_since_days("abc").is_err());
    }

    #[test]
    fn slug_from_path_only_matches_item_md() {
        assert_eq!(
            slug_from_path("issues/open/foo-bar-baz/item.md"),
            Some("foo-bar-baz".to_string())
        );
        assert_eq!(slug_from_path("issues/open/foo-bar-baz/notes.md"), None);
        assert_eq!(slug_from_path("README.md"), None);
        // archive layout: issues/archive/2026-05/<slug>/item.md
        assert_eq!(
            slug_from_path("issues/archive/2026-05/qux-quux-corge/item.md"),
            Some("qux-quux-corge".to_string())
        );
    }

    #[test]
    fn metrics_window_filters_and_assignee_rollup() {
        let issues = vec![
            mk_issue("a-old-fox", "fixed", "2026-01-01", Some("2026-01-10")),
            mk_issue("b-mid-owl", "fixed", "2026-05-10", Some("2026-05-20")),
            mk_issue("c-new-elk", "open", "2026-05-25", None),
        ];
        let today = NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();
        let m = metrics(&issues, Some(30), today);
        assert_eq!(m.throughput, 1, "only b-mid-owl is within 30 days");
        assert_eq!(m.closed_by_assignee.get("alice").copied(), Some(1));
        assert_eq!(m.workload_by_assignee.get("alice").copied(), Some(1));
        let cs = m.cycle_time_days.unwrap();
        assert_eq!(cs.sample, 1);
        assert_eq!(cs.median, 10);
    }

    #[test]
    fn metrics_handles_empty_window() {
        let issues = vec![mk_issue("a-old-fox", "open", "2026-05-25", None)];
        let today = NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();
        let m = metrics(&issues, Some(30), today);
        assert_eq!(m.throughput, 0);
        assert!(m.cycle_time_days.is_none());
    }

    #[test]
    fn changelog_groups_by_type_and_attributes_fixes() {
        // No git here — call the structuring path directly via a
        // synthetic commit list would need a refactor; the surface
        // covered by integration is exercised by the CLI tests
        // (`tests/cli_report.rs`).
        let issues = [mk_issue(
            "a-fox-jr",
            "fixed",
            "2026-05-01",
            Some("2026-05-05"),
        )];
        // We can at least round-trip the rendering of an empty report.
        let report = ChangelogReport {
            range: "v1..v2".to_string(),
            groups: {
                let mut g = BTreeMap::new();
                g.insert(
                    "feature".to_string(),
                    vec![ChangelogEntry {
                        slug: issues[0].slug.clone(),
                        title: issues[0].title.clone(),
                        issue_type: issues[0].issue_type.clone(),
                        labels: vec![],
                        status: issues[0].status.clone(),
                        commits: vec![ChangelogCommit {
                            sha: "abcdef012345".into(),
                            summary: "fix the thing".into(),
                            fixes: true,
                        }],
                    }],
                );
                g
            },
            orphan_commits: vec![],
        };
        let md = render_changelog_markdown(&report);
        assert!(md.contains("## Changelog v1..v2"));
        assert!(md.contains("### feature"));
        assert!(md.contains("**a-fox-jr**"));
        assert!(md.contains("(fixes)"));
    }

    #[test]
    fn changelog_picks_up_close_stamped_trailer() {
        // End-to-end (requirement b): a commit stamped by
        // `git_trailers::stamp_fixes_trailer` (what `issuectl close
        // --stamp` runs) is attributed to its issue by `changelog`.
        fn git(dir: &std::path::Path, args: &[&str]) {
            let st = Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .unwrap();
            assert!(st.success(), "git {args:?} failed");
        }
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git(root, &["init", "-q", "-b", "main"]);
        git(root, &["config", "--local", "user.email", "t@example.com"]);
        git(root, &["config", "--local", "user.name", "t"]);
        // Non-empty commit: `--amend` (what the stamp runs) refuses an
        // empty tree.
        std::fs::write(root.join("fix.txt"), "the fix").unwrap();
        git(root, &["add", "fix.txt"]);
        std::fs::write(root.join(".msg"), "feat: land the fix\n\nbody\n").unwrap();
        git(root, &["commit", "-q", "-F", ".msg"]);

        // Stamp the landing commit exactly as `close --stamp` does.
        let outcome = git_trailers::stamp_fixes_trailer(root, "a-fox-jr").unwrap();
        assert!(
            matches!(outcome, git_trailers::StampOutcome::Stamped { .. }),
            "{outcome:?}"
        );
        // Rewrite is message-only; the file we committed is untouched.
        assert!(root.join("fix.txt").exists());

        let issues = [mk_issue(
            "a-fox-jr",
            "fixed",
            "2026-05-01",
            Some("2026-05-05"),
        )];
        let report = changelog(root, "", &issues).unwrap();
        let feature = report.groups.get("feature").expect("feature group");
        let entry = feature.iter().find(|e| e.slug == "a-fox-jr").unwrap();
        assert_eq!(entry.commits.len(), 1);
        assert!(entry.commits[0].fixes, "commit should be marked as a fix");
        assert!(report.orphan_commits.is_empty());
    }
}
