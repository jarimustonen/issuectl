//! `issuectl sync-commits` — walk git history, parse `Refs-Issue:` /
//! `Fixes-Issue:` trailers, and append the matching commits onto each
//! issue's `commits[]` array. Idempotent: re-running on the same range
//! is a no-op because `write::add_commit` skips entries whose hash is
//! already present.
//!
//! Branch-name fallback: if a commit has no trailer and the current
//! branch name resolves to a known slug (exact match, `prefix/<slug>`,
//! or `prefix-<slug>`), the commit is implicitly attributed to that
//! slug.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;

use crate::git_trailers::{self, CommitInfo};
use crate::mutate::{self, CommitSpec, Patch, UpdateIssueRequest};
use crate::repo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionKind {
    /// Explicit `Refs-Issue:` trailer.
    Refs,
    /// Explicit `Fixes-Issue:` trailer.
    Fixes,
    /// Implicit attribution from branch name.
    Branch,
}

#[derive(Debug, Clone)]
pub struct PlannedAdd {
    pub slug: String,
    pub hash: String,
    pub summary: String,
    pub kind: AttributionKind,
    /// True when the issue's `commits[]` already contains this hash;
    /// the mutation layer would skip the add idempotently.
    pub already_present: bool,
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub planned: Vec<PlannedAdd>,
    /// Slugs mentioned in `Fixes-Issue:` trailers — caller prints a
    /// hint suggesting `issuectl close`. Not auto-transitioned.
    pub fixes_hints: BTreeSet<String>,
    /// Slugs that appeared in trailers but don't exist in the repo.
    pub unknown_slugs: BTreeSet<String>,
    /// Slug → number of commits actually appended (excluding
    /// already-present idempotent skips). Empty on dry-run.
    pub applied: BTreeMap<String, usize>,
    pub dry_run: bool,
    /// The git range that was walked (for human output).
    pub range: String,
    /// Branch name used for branch-name fallback, if any.
    pub branch: Option<String>,
    /// Issue-load warnings surfaced from `repo::load_issue_summaries`
    /// (malformed frontmatter, ambiguous layout, etc.). Bubbled up so
    /// callers can warn — silently dropping them lets sync run
    /// against a partial picture of the repo.
    pub load_warnings: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct SyncOptions {
    /// `<since>..<until>` or any other `git log` range expression. When
    /// `None`, defaults to `<merge-base of HEAD and main/master>..HEAD`.
    pub range: Option<String>,
    /// Disable branch-name fallback (commits with no trailer stay
    /// unattributed instead of inheriting the current branch's slug).
    pub no_branch_fallback: bool,
    /// Print the plan and skip writes.
    pub dry_run: bool,
}

pub fn run(repo_root: &Path, opts: SyncOptions) -> Result<SyncReport> {
    let range = match opts.range.clone() {
        Some(r) => r,
        None => match git_trailers::default_range(repo_root)? {
            Some(r) => r,
            // No merge-base with main/master — walk all of HEAD. This
            // happens on a fresh repo or one where the conventional
            // base branches are absent. Documented in `--help`.
            None => "HEAD".to_string(),
        },
    };

    let commits = git_trailers::parse_log(repo_root, &range)?;

    let (summaries, warnings) = repo::load_issue_summaries(repo_root);
    let known_slugs: BTreeSet<String> = summaries.iter().map(|s| s.slug.clone()).collect();
    let load_warnings: Vec<String> = warnings.into_iter().map(|w| w.message).collect();
    let mut commit_index: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for s in &summaries {
        let m = commit_index.entry(s.slug.clone()).or_default();
        if let Some(cs) = &s.commits {
            for c in cs {
                m.insert(c.hash.clone(), c.summary.clone());
            }
        }
    }

    let branch = if opts.no_branch_fallback {
        None
    } else {
        git_trailers::current_branch(repo_root)?
    };
    let branch_slug = branch
        .as_deref()
        .and_then(|b| git_trailers::branch_slug(b, known_slugs.iter().map(|s| s.as_str())));

    let mut planned: Vec<PlannedAdd> = Vec::new();
    let mut fixes_hints: BTreeSet<String> = BTreeSet::new();
    let mut unknown_slugs: BTreeSet<String> = BTreeSet::new();

    for c in &commits {
        // "Attributed" only counts trailer slugs that actually
        // resolve to a known issue. A trailer pointing at an
        // unknown slug (typo, deleted issue) shouldn't suppress the
        // branch-name fallback — otherwise the commit silently
        // drops onto the floor when the user clearly intended to
        // attribute it.
        let mut attributed_to_known = false;
        for slug in &c.refs_issue {
            let known = known_slugs.contains(slug);
            if known {
                attributed_to_known = true;
            }
            push_planned(
                &mut planned,
                &known_slugs,
                &commit_index,
                &mut unknown_slugs,
                slug,
                c,
                AttributionKind::Refs,
            );
        }
        for slug in &c.fixes_issue {
            let known = known_slugs.contains(slug);
            if known {
                attributed_to_known = true;
                fixes_hints.insert(slug.clone());
            }
            push_planned(
                &mut planned,
                &known_slugs,
                &commit_index,
                &mut unknown_slugs,
                slug,
                c,
                AttributionKind::Fixes,
            );
        }
        if !attributed_to_known {
            if let Some(slug) = &branch_slug {
                push_planned(
                    &mut planned,
                    &known_slugs,
                    &commit_index,
                    &mut unknown_slugs,
                    slug,
                    c,
                    AttributionKind::Branch,
                );
            }
        }
    }

    let mut report = SyncReport {
        planned: planned.clone(),
        fixes_hints,
        unknown_slugs,
        applied: BTreeMap::new(),
        dry_run: opts.dry_run,
        range,
        branch: branch_slug,
        load_warnings,
    };

    if opts.dry_run {
        return Ok(report);
    }

    // Group new (non-already-present) adds per slug, then issue one
    // mutate::update_issue per slug. The mutation layer's add_commit
    // is idempotent regardless, but pre-filtering means the planned
    // count matches the post-state count without re-reading.
    let mut by_slug: BTreeMap<String, Vec<CommitSpec>> = BTreeMap::new();
    for p in &planned {
        if p.already_present {
            continue;
        }
        by_slug.entry(p.slug.clone()).or_default().push(CommitSpec {
            hash: p.hash.clone(),
            summary: p.summary.clone(),
        });
    }

    for (slug, specs) in by_slug {
        let count = specs.len();
        let req = UpdateIssueRequest {
            add_commits: specs,
            // Sync is a background reconciliation: no
            // optimistic-version token needed (same as plain CLI
            // `--add-commit`).
            expected_version: None,
            status: Patch::Unspecified,
            ..Default::default()
        };
        mutate::update_issue(repo_root, &slug, req, &crate::repo_config::UncachedConfig)
            .map_err(|e| anyhow::anyhow!("updating @{slug}: {e}"))?;
        report.applied.insert(slug, count);
    }

    Ok(report)
}

fn push_planned(
    planned: &mut Vec<PlannedAdd>,
    known: &BTreeSet<String>,
    commit_index: &BTreeMap<String, BTreeMap<String, String>>,
    unknown: &mut BTreeSet<String>,
    slug: &str,
    c: &CommitInfo,
    kind: AttributionKind,
) {
    if !known.contains(slug) {
        unknown.insert(slug.to_string());
        return;
    }
    let already_present = commit_index
        .get(slug)
        .map(|m| {
            m.keys()
                .any(|existing| crate::write::hashes_match(existing, &c.hash))
        })
        .unwrap_or(false);
    // Suppress in-plan duplicates (same slug + same hash from
    // multiple Refs-Issue trailers, or Refs+branch fallback chained).
    if planned
        .iter()
        .any(|p| p.slug == slug && crate::write::hashes_match(&p.hash, &c.hash))
    {
        return;
    }
    planned.push(PlannedAdd {
        slug: slug.to_string(),
        hash: c.hash.clone(),
        summary: c.summary.clone(),
        kind,
        already_present,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?} failed");
    }

    fn fresh_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-q", "-b", "main"]);
        for (k, v) in [("user.email", "t@example.com"), ("user.name", "t")] {
            git(tmp.path(), &["config", "--local", k, v]);
        }
        tmp
    }

    fn seed_issue(root: &Path, slug: &str) {
        let dir = root.join("issues").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        let body = format!(
            "---\ncreated: 2026-01-01\nupdated: 2026-01-01\ntype: feature\nstatus: open\npriority: normal\nassignee: jari\n---\n\n# {slug}\n\nbody\n"
        );
        std::fs::write(dir.join("item.md"), body).unwrap();
    }

    fn commit(root: &Path, msg: &str) {
        // Record the message via -F to preserve newlines / trailers.
        let f = root.join(".msg");
        std::fs::write(&f, msg).unwrap();
        git(root, &["commit", "--allow-empty", "-q", "-F", ".msg"]);
        std::fs::remove_file(&f).ok();
    }

    #[test]
    fn sync_picks_up_refs_trailer_and_is_idempotent() {
        let tmp = fresh_repo();
        let root = tmp.path();
        seed_issue(root, "foo-bar-baz");
        git(root, &["add", "."]);
        commit(root, "initial\n\nseed\n");
        // Branch off so HEAD..main has commits with trailers.
        git(root, &["checkout", "-q", "-b", "work"]);
        commit(
            root,
            "feat: do thing\n\nlonger desc\n\nRefs-Issue: @foo-bar-baz\n",
        );
        commit(root, "fix: another\n\nFixes-Issue: foo-bar-baz\n");

        let report = run(
            root,
            SyncOptions {
                range: Some("main..HEAD".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.applied.get("foo-bar-baz").copied(), Some(2));
        assert!(report.fixes_hints.contains("foo-bar-baz"));
        // Idempotent re-run.
        let report2 = run(
            root,
            SyncOptions {
                range: Some("main..HEAD".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report2.applied.is_empty(), "expected no new adds");
    }

    #[test]
    fn dry_run_does_not_write() {
        let tmp = fresh_repo();
        let root = tmp.path();
        seed_issue(root, "foo-bar-baz");
        git(root, &["add", "."]);
        commit(root, "initial\n");
        git(root, &["checkout", "-q", "-b", "work"]);
        commit(root, "x\n\nRefs-Issue: @foo-bar-baz\n");

        let report = run(
            root,
            SyncOptions {
                range: Some("main..HEAD".into()),
                dry_run: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.planned.len(), 1);
        assert!(report.applied.is_empty());
        // Confirm no commits in item.md.
        let body = std::fs::read_to_string(root.join("issues/foo-bar-baz/item.md")).unwrap();
        assert!(!body.contains("commits:"), "got:\n{body}");
    }

    #[test]
    fn unknown_slug_in_trailer_is_skipped() {
        let tmp = fresh_repo();
        let root = tmp.path();
        seed_issue(root, "foo-bar-baz");
        git(root, &["add", "."]);
        commit(root, "initial\n");
        git(root, &["checkout", "-q", "-b", "work"]);
        commit(root, "x\n\nRefs-Issue: @no-such-issue\n");

        let report = run(
            root,
            SyncOptions {
                range: Some("main..HEAD".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report.applied.is_empty());
        assert!(report.unknown_slugs.contains("no-such-issue"));
    }

    #[test]
    fn branch_name_fallback_attributes_when_no_trailer() {
        let tmp = fresh_repo();
        let root = tmp.path();
        seed_issue(root, "foo-bar-baz");
        git(root, &["add", "."]);
        commit(root, "initial\n");
        git(root, &["checkout", "-q", "-b", "wt-foo-bar-baz"]);
        commit(root, "untagged work\n");

        let report = run(
            root,
            SyncOptions {
                range: Some("main..HEAD".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.applied.get("foo-bar-baz").copied(), Some(1));
    }

    #[test]
    fn branch_name_fallback_disabled_leaves_untagged_commits_alone() {
        let tmp = fresh_repo();
        let root = tmp.path();
        seed_issue(root, "foo-bar-baz");
        git(root, &["add", "."]);
        commit(root, "initial\n");
        git(root, &["checkout", "-q", "-b", "wt-foo-bar-baz"]);
        commit(root, "untagged work\n");

        let report = run(
            root,
            SyncOptions {
                range: Some("main..HEAD".into()),
                no_branch_fallback: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report.applied.is_empty());
    }
}
