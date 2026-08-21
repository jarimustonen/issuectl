//! Promote an inbox draft (`issues/inbox/<slug>/`) to the canonical
//! flat layout (`issues/<slug>/`). Reverse direction is out of scope —
//! once a draft is triaged, the active issue lives in the flat root.
//!
//! The move is filesystem-level (`fs::rename`) under the repo
//! `WriteLock` so a concurrent writer can't observe the half-moved
//! state. The frontmatter is left untouched; if the draft set
//! `status: open` (or any other active value), that survives the
//! promotion verbatim.

use std::path::{Path, PathBuf};

use super::{MutateError, WriteLock};
use crate::repo;

/// Outcome of a `triage` call.
#[derive(Debug, Clone)]
pub struct TriageOutcome {
    pub slug: String,
    /// Pre-move path (`issues/inbox/<slug>/`).
    pub from: PathBuf,
    /// Post-move path (`issues/<slug>/`).
    pub to: PathBuf,
}

/// Move `issues/inbox/<slug>/` → `issues/<slug>/`. Errors:
/// - `Validation` for missing slug / wrong layout (not in inbox).
/// - `Validation` if the target flat path already exists.
pub fn triage(repo_root: &Path, slug: &str) -> Result<TriageOutcome, MutateError> {
    let lock = WriteLock::acquire(repo_root).map_err(MutateError::Io)?;
    triage_locked(repo_root, slug, &lock)
}

/// Lock-aware body shared with `doctor --fix` while it holds the repository
/// write lock for its whole migration pipeline.
pub(crate) fn triage_locked(
    repo_root: &Path,
    slug: &str,
    _lock: &WriteLock,
) -> Result<TriageOutcome, MutateError> {
    let item_path = match repo::resolve_layout(repo_root, slug) {
        repo::LayoutState::Inbox { item_path } => item_path,
        repo::LayoutState::Absent => return Err(MutateError::NotFound),
        repo::LayoutState::Flat { .. } => {
            return Err(MutateError::Validation(format!(
                "{slug} is already in the flat layout; nothing to triage"
            )))
        }
        repo::LayoutState::Legacy { folder, .. } => {
            return Err(MutateError::Validation(format!(
                "{slug} is at legacy path issues/{folder}/{slug}/ — run `issuectl doctor --fix` instead"
            )))
        }
        repo::LayoutState::Ambiguous { paths } => {
            return Err(MutateError::AmbiguousSlug { paths })
        }
        repo::LayoutState::Invalid { reason, .. } => {
            return Err(MutateError::Io(anyhow::anyhow!("{reason}")))
        }
    };

    let from_dir = item_path
        .parent()
        .ok_or_else(|| MutateError::Validation(format!("item.md for {slug} has no parent")))?
        .to_path_buf();
    let to_dir = repo_root.join("issues").join(slug);
    if to_dir.exists() {
        return Err(MutateError::Validation(format!(
            "cannot triage {slug}: target {} already exists",
            to_dir.display()
        )));
    }
    if let Some(parent) = to_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            MutateError::Io(
                anyhow::Error::from(e).context(format!("cannot create {}", parent.display())),
            )
        })?;
    }
    std::fs::rename(&from_dir, &to_dir).map_err(|e| {
        MutateError::Io(anyhow::Error::from(e).context(format!(
            "cannot move {} -> {}",
            from_dir.display(),
            to_dir.display()
        )))
    })?;
    Ok(TriageOutcome {
        slug: slug.to_string(),
        from: from_dir,
        to: to_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fresh_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        tmp
    }

    fn seed_inbox(tmp: &TempDir, slug: &str) {
        let dir = tmp.path().join("issues").join("inbox").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            format!("---\nstatus: open\ntype: bug\n---\n\n# {slug}\n"),
        )
        .unwrap();
    }

    #[test]
    fn promotes_inbox_to_flat() {
        let tmp = fresh_repo();
        seed_inbox(&tmp, "calm-quiet-otter");
        let out = triage(tmp.path(), "calm-quiet-otter").unwrap();
        assert!(out.to.ends_with("issues/calm-quiet-otter"));
        assert!(tmp.path().join("issues/calm-quiet-otter/item.md").is_file());
        assert!(!tmp.path().join("issues/inbox/calm-quiet-otter").exists());
    }

    #[test]
    fn refuses_when_flat_and_inbox_both_exist() {
        // A slug present at both inbox AND flat surfaces as
        // `LayoutState::Ambiguous` — triage refuses to pick a side
        // rather than silently overwriting either copy.
        let tmp = fresh_repo();
        seed_inbox(&tmp, "calm-quiet-otter");
        let flat = tmp.path().join("issues/calm-quiet-otter");
        fs::create_dir_all(&flat).unwrap();
        fs::write(flat.join("item.md"), "---\nstatus: open\n---\n# x\n").unwrap();
        let err = triage(tmp.path(), "calm-quiet-otter").unwrap_err();
        assert!(matches!(err, MutateError::AmbiguousSlug { .. }), "{err:?}");
    }

    #[test]
    fn rejects_non_inbox_slugs() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/foo-bar");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\nstatus: open\n---\n# x\n").unwrap();
        let err = triage(tmp.path(), "foo-bar").unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)));
    }

    #[test]
    fn missing_slug_is_not_found() {
        let tmp = fresh_repo();
        let err = triage(tmp.path(), "no-such-slug").unwrap_err();
        assert!(matches!(err, MutateError::NotFound));
    }
}
