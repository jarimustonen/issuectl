//! Cold-storage archival of closed issues. Moves
//! `issues/<slug>/` → `issues/archive/YYYY/MM/<slug>/` to keep the
//! active tree small. The move is a directory rename only — issue
//! content is preserved byte-for-byte (no `updated` bump, no re-serialize).
//! Archived issues stay fully readable: `repo::load_issues` /
//! `locate_issue` walk the archive root, so `show`, `list`, and queries
//! all still find them.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::clock::{Clock, SystemClock};
use serde::Serialize;

use super::{MutateError, WriteLock};
use crate::repo;

/// Default `--older-than` window in days when the flag is omitted.
pub const DEFAULT_ARCHIVE_DAYS: i64 = 90;

/// One issue moved (or, under `--dry-run`, that *would* move) into the
/// archive.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveMove {
    pub slug: String,
    pub from: PathBuf,
    pub to: PathBuf,
    /// Closing date used to bucket the archive path (`YYYY-MM-DD`).
    pub dated: String,
}

/// A closed issue considered but not archived, with the reason.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveSkip {
    pub slug: String,
    pub reason: String,
}

/// Outcome of an `archive` run.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveReport {
    pub older_than_days: i64,
    pub dry_run: bool,
    pub archived: Vec<ArchiveMove>,
    pub skipped: Vec<ArchiveSkip>,
}

/// Archive every closed issue whose closing date is at least
/// `older_than_days` in the past. Holds the repo write lock for the whole
/// batch so the set is consistent. `dry_run` reports the planned moves
/// without touching disk.
pub fn archive_closed(
    repo_root: &Path,
    older_than_days: i64,
    dry_run: bool,
) -> Result<ArchiveReport, MutateError> {
    archive_closed_via(repo_root, older_than_days, dry_run, &SystemClock)
}

/// Clock-injected variant of [`archive_closed`].
pub fn archive_closed_via(
    repo_root: &Path,
    older_than_days: i64,
    dry_run: bool,
    clock: &dyn Clock,
) -> Result<ArchiveReport, MutateError> {
    let _lock = WriteLock::acquire(repo_root).map_err(MutateError::Io)?;
    let schema =
        crate::schema::load(repo_root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let today = clock.today();
    // One archive-tree walk shared across every per-slug layout resolve
    // below, keeping the batch O(N) rather than O(N·archive).
    let archive_root = repo_root.join("issues").join(repo::ARCHIVE_DIR);
    let index = repo::archive_index(repo_root);

    let mut archived = Vec::new();
    let mut skipped = Vec::new();

    // Re-load under the lock so concurrent writers can't shift the set.
    for issue in repo::load_issues(repo_root) {
        if !crate::schema::is_closing(&schema, &issue.status) {
            continue;
        }
        let Some(dated) = issue
            .closed
            .as_deref()
            .or(issue.updated.as_deref())
            .and_then(crate::stale::parse_date)
        else {
            skipped.push(ArchiveSkip {
                slug: issue.slug.clone(),
                reason: "no parseable closed/updated date — cannot judge age".to_string(),
            });
            continue;
        };
        if (today - dated).num_days() < older_than_days {
            continue;
        }
        match plan_move(repo_root, &issue.slug, dated, &index, &archive_root) {
            Ok(None) => continue, // already in cold storage
            Ok(Some(mv)) => {
                if !dry_run {
                    if let Err(e) = perform_move(&mv) {
                        skipped.push(ArchiveSkip {
                            slug: issue.slug.clone(),
                            reason: format!("move failed: {e:#}"),
                        });
                        continue;
                    }
                }
                archived.push(mv);
            }
            Err(reason) => skipped.push(ArchiveSkip {
                slug: issue.slug.clone(),
                reason,
            }),
        }
    }

    archived.sort_by(|a, b| a.slug.cmp(&b.slug));
    skipped.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(ArchiveReport {
        older_than_days,
        dry_run,
        archived,
        skipped,
    })
}

/// Compute the source/destination dirs for a slug. `Ok(None)` means the
/// issue is already in cold storage (nothing to do). `Err(reason)` flags
/// a state that blocks a clean move (ambiguous, missing, destination
/// collision). Reuses the prebuilt archive index; does not touch disk.
fn plan_move(
    repo_root: &Path,
    slug: &str,
    dated: NaiveDate,
    index: &repo::ArchiveIndex,
    archive_root: &Path,
) -> Result<Option<ArchiveMove>, String> {
    let from = match repo::resolve_layout_in(repo_root, slug, index) {
        repo::LayoutState::Flat { item_path } => {
            let dir = item_path
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| "item.md has no parent".to_string())?;
            if dir.starts_with(archive_root) {
                return Ok(None); // already archived
            }
            dir
        }
        repo::LayoutState::Legacy { .. } => {
            return Err("issue is at a legacy path — run `issuectl doctor --fix` first".to_string())
        }
        repo::LayoutState::Inbox { .. } => {
            return Err(
                "issue is in the inbox — run `issuectl triage <slug>` to promote it first"
                    .to_string(),
            )
        }
        repo::LayoutState::Ambiguous { .. } => return Err("slug is ambiguous".to_string()),
        repo::LayoutState::Absent => return Err("issue vanished mid-run".to_string()),
        repo::LayoutState::Invalid { reason, .. } => return Err(reason),
    };
    let rel = repo::archive_relpath(slug, &dated.format("%Y-%m-%d").to_string());
    let to = repo_root.join("issues").join(rel);
    if to.exists() {
        return Err(format!(
            "destination already exists: {} — resolve manually",
            to.display()
        ));
    }
    Ok(Some(ArchiveMove {
        slug: slug.to_string(),
        from,
        to,
        dated: dated.format("%Y-%m-%d").to_string(),
    }))
}

fn perform_move(mv: &ArchiveMove) -> std::io::Result<()> {
    if let Some(parent) = mv.to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&mv.from, &mv.to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::fs;
    use tempfile::TempDir;

    fn repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        tmp
    }

    fn seed(tmp: &TempDir, slug: &str, status: &str, closed: Option<&str>) {
        let dir = tmp.path().join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        let closed_line = closed.map(|c| format!("closed: {c}\n")).unwrap_or_default();
        fs::write(
            dir.join("item.md"),
            format!("---\nstatus: {status}\n{closed_line}---\n\n# {slug}\n"),
        )
        .unwrap();
    }

    #[test]
    fn archives_old_closed_issue() {
        let tmp = repo();
        seed(&tmp, "old-done-fox", "fixed", Some("2020-01-01"));
        let report = archive_closed_via(
            tmp.path(),
            90,
            false,
            &crate::clock::FixedClock::new(
                chrono::Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap(),
            ),
        )
        .unwrap();
        assert_eq!(report.archived.len(), 1);
        assert!(!tmp.path().join("issues/old-done-fox").exists());
        assert!(tmp
            .path()
            .join("issues/archive/2020/01/old-done-fox/item.md")
            .is_file());
    }

    #[test]
    fn skips_recently_closed_and_open_issues() {
        let tmp = repo();
        let recent = "2026-02-28".to_string();
        seed(&tmp, "fresh-done-owl", "fixed", Some(&recent));
        seed(&tmp, "active-open-elk", "open", None);
        let report = archive_closed_via(
            tmp.path(),
            90,
            false,
            &crate::clock::FixedClock::new(
                chrono::Utc
                    .with_ymd_and_hms(2026, 2, 28, 23, 59, 59)
                    .unwrap(),
            ),
        )
        .unwrap();
        assert!(report.archived.is_empty());
        assert!(tmp.path().join("issues/fresh-done-owl").exists());
        assert!(tmp.path().join("issues/active-open-elk").exists());
    }

    #[test]
    fn fixed_clock_buckets_month_boundary_from_closed_date() {
        let tmp = repo();
        seed(&tmp, "month-end-otter", "fixed", Some("2026-01-31"));
        let clock = crate::clock::FixedClock::new(
            chrono::Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap(),
        );
        let report = archive_closed_via(tmp.path(), 29, false, &clock).unwrap();
        assert_eq!(report.archived[0].dated, "2026-01-31");
        assert!(tmp
            .path()
            .join("issues/archive/2026/01/month-end-otter/item.md")
            .is_file());
    }

    #[test]
    fn dry_run_moves_nothing() {
        let tmp = repo();
        seed(&tmp, "old-done-fox", "fixed", Some("2020-01-01"));
        let report = archive_closed(tmp.path(), 90, true).unwrap();
        assert_eq!(report.archived.len(), 1);
        assert!(report.dry_run);
        assert!(tmp.path().join("issues/old-done-fox").exists());
        assert!(!tmp
            .path()
            .join("issues/archive/2020/01/old-done-fox")
            .exists());
    }

    #[test]
    fn already_archived_is_left_alone() {
        let tmp = repo();
        let dir = tmp.path().join("issues/archive/2020/01/old-done-fox");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\nstatus: fixed\nclosed: 2020-01-01\n---\n# x\n",
        )
        .unwrap();
        let report = archive_closed(tmp.path(), 90, false).unwrap();
        assert!(report.archived.is_empty());
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn missing_date_is_skipped_with_reason() {
        let tmp = repo();
        seed(&tmp, "dateless-done-newt", "fixed", None);
        let report = archive_closed(tmp.path(), 90, false).unwrap();
        assert!(report.archived.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].slug, "dateless-done-newt");
    }
}
