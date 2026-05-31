//! Copy user-supplied files into an issue's `attachments/` directory.
//! Materialises `issues/<slug>/attachments/` on demand
//! (`new_issue::ensure_issue_subdir`) so empty dirs aren't committed
//! eagerly, then copies each source under its basename. Collisions are
//! resolved with a numeric suffix (`name-1.ext`, `name-2.ext`, …) so a
//! batch attach does not bail halfway; the `renamed: true` flag in the
//! per-file outcome lets callers surface the rename.
//!
//! Lives next to other mutation verbs because it acquires the same
//! repo-wide `WriteLock` — keeps the attach-vs-rename race that would
//! otherwise corrupt a half-copied attachment off the table.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{MutateError, WriteLock};
use crate::mutate::new_issue::{ensure_issue_subdir, ATTACHMENTS_DIRNAME};
use crate::repo;

/// Per-file result of an attach run. `path` is repo-relative
/// (`issues/<slug>/attachments/<name>`) for parity with the rest of the
/// `--json` surface. `renamed` is true when the destination basename
/// differs from the source basename because of a collision.
#[derive(Debug, Clone, Serialize)]
pub struct AttachedFile {
    pub source: PathBuf,
    pub path: PathBuf,
    pub name: String,
    pub original_name: String,
    pub renamed: bool,
    pub bytes: u64,
}

/// Outcome of one `attach` invocation.
#[derive(Debug, Clone, Serialize)]
pub struct AttachReport {
    pub slug: String,
    /// Issue directory (the parent of `item.md`), repo-relative.
    pub dir: PathBuf,
    pub attached: Vec<AttachedFile>,
}

/// Copy `sources` into `<repo_root>/issues/<slug>/attachments/`,
/// creating the directory if needed. Validation errors (missing source,
/// source-is-a-directory, slug-not-found, basename not UTF-8) surface as
/// `MutateError::Validation`; filesystem errors as `MutateError::Io`.
pub fn attach_files(
    repo_root: &Path,
    slug: &str,
    sources: &[PathBuf],
) -> Result<AttachReport, MutateError> {
    if sources.is_empty() {
        return Err(MutateError::Validation(
            "no files to attach".to_string(),
        ));
    }
    let _lock = WriteLock::acquire(repo_root).map_err(MutateError::Io)?;

    let (_folder, item_path) = repo::locate_issue(repo_root, slug)
        .map_err(|e| MutateError::Validation(format!("{e:#}")))?;
    let issue_dir = item_path
        .parent()
        .ok_or_else(|| MutateError::Validation(format!("item.md for {slug} has no parent")))?
        .to_path_buf();

    // Pre-flight every source so we either attach all of them or none —
    // an early validation failure (missing path, basename collisions
    // against itself) shouldn't leave a half-finished attachments dir.
    let mut planned: Vec<(PathBuf, String)> = Vec::with_capacity(sources.len());
    for src in sources {
        let meta = std::fs::symlink_metadata(src).map_err(|e| {
            MutateError::Validation(format!("cannot stat {}: {e}", src.display()))
        })?;
        if meta.is_dir() {
            return Err(MutateError::Validation(format!(
                "{} is a directory; attach takes files only",
                src.display()
            )));
        }
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| {
                MutateError::Validation(format!(
                    "{}: cannot derive a UTF-8 filename",
                    src.display()
                ))
            })?
            .to_string();
        if name.is_empty() {
            return Err(MutateError::Validation(format!(
                "{}: empty basename",
                src.display()
            )));
        }
        planned.push((src.clone(), name));
    }

    let attachments_dir =
        ensure_issue_subdir(&issue_dir, ATTACHMENTS_DIRNAME).map_err(MutateError::Io)?;

    let mut taken: Vec<String> = Vec::with_capacity(planned.len());
    let mut attached = Vec::with_capacity(planned.len());
    for (src, original_name) in planned {
        let final_name = pick_unique_name(&attachments_dir, &original_name, &taken);
        let dest = attachments_dir.join(&final_name);
        let bytes = std::fs::copy(&src, &dest).map_err(|e| {
            MutateError::Io(anyhow::anyhow!(
                "cannot copy {} -> {}: {e}",
                src.display(),
                dest.display()
            ))
        })?;
        let rel_dir = issue_dir.strip_prefix(repo_root).unwrap_or(&issue_dir);
        let rel_path = rel_dir.join(ATTACHMENTS_DIRNAME).join(&final_name);
        attached.push(AttachedFile {
            source: src,
            path: rel_path,
            renamed: final_name != original_name,
            name: final_name.clone(),
            original_name,
            bytes,
        });
        taken.push(final_name);
    }

    let rel_dir = issue_dir
        .strip_prefix(repo_root)
        .unwrap_or(&issue_dir)
        .to_path_buf();
    Ok(AttachReport {
        slug: slug.to_string(),
        dir: rel_dir,
        attached,
    })
}

/// Return `name` if it doesn't collide with an existing on-disk file or
/// with a name we've already taken in this batch; otherwise insert a
/// numeric suffix before the final extension (`foo.tar.gz` → `foo-1.tar.gz`).
fn pick_unique_name(dir: &Path, name: &str, taken: &[String]) -> String {
    let exists = |candidate: &str| -> bool {
        taken.iter().any(|t| t == candidate) || dir.join(candidate).exists()
    };
    if !exists(name) {
        return name.to_string();
    }
    let (stem, ext) = split_stem_ext(name);
    for n in 1..10_000 {
        let candidate = if ext.is_empty() {
            format!("{stem}-{n}")
        } else {
            format!("{stem}-{n}.{ext}")
        };
        if !exists(&candidate) {
            return candidate;
        }
    }
    // Pathological case: ~10k collisions. Fall back to a longer suffix
    // built from the nanosecond clock so we never loop forever.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    if ext.is_empty() {
        format!("{stem}-{nanos}")
    } else {
        format!("{stem}-{nanos}.{ext}")
    }
}

/// Split a basename into `(stem, ext)`. The extension is the suffix
/// after the *first* dot (so `archive.tar.gz` → `("archive","tar.gz")`)
/// — keeps multi-part extensions intact across renames. A leading dot
/// is preserved on the stem (`.envrc` → `(".envrc","")`).
fn split_stem_ext(name: &str) -> (&str, &str) {
    let bytes = name.as_bytes();
    // Find first '.' that is NOT at position 0.
    for (i, b) in bytes.iter().enumerate().skip(1) {
        if *b == b'.' {
            return (&name[..i], &name[i + 1..]);
        }
    }
    (name, "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn repo_with_issue(slug: &str) -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            format!("---\nstatus: open\n---\n\n# {slug}\n"),
        )
        .unwrap();
        tmp
    }

    fn write_src(dir: &Path, name: &str, body: &[u8]) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn copies_files_into_attachments_creating_the_dir() {
        let tmp = repo_with_issue("calm-quiet-otter");
        let src_dir = tempfile::tempdir().unwrap();
        let a = write_src(src_dir.path(), "shot.png", b"PNGDATA");
        let b = write_src(src_dir.path(), "log.txt", b"hello");

        let report = attach_files(tmp.path(), "calm-quiet-otter", &[a.clone(), b.clone()]).unwrap();

        assert_eq!(report.attached.len(), 2);
        let att_dir = tmp.path().join("issues/calm-quiet-otter/attachments");
        assert!(att_dir.is_dir());
        assert_eq!(fs::read(att_dir.join("shot.png")).unwrap(), b"PNGDATA");
        assert_eq!(fs::read(att_dir.join("log.txt")).unwrap(), b"hello");
        assert!(report.attached.iter().all(|f| !f.renamed));
        assert_eq!(
            report.dir,
            PathBuf::from("issues/calm-quiet-otter")
        );
        assert_eq!(report.attached[0].bytes, b"PNGDATA".len() as u64);
        assert_eq!(
            report.attached[0].path,
            PathBuf::from("issues/calm-quiet-otter/attachments/shot.png")
        );
    }

    #[test]
    fn collision_with_existing_file_renames_with_numeric_suffix() {
        let tmp = repo_with_issue("calm-quiet-otter");
        let att_dir = tmp.path().join("issues/calm-quiet-otter/attachments");
        fs::create_dir_all(&att_dir).unwrap();
        fs::write(att_dir.join("shot.png"), b"OLD").unwrap();

        let src_dir = tempfile::tempdir().unwrap();
        let src = write_src(src_dir.path(), "shot.png", b"NEW");

        let report = attach_files(tmp.path(), "calm-quiet-otter", &[src]).unwrap();
        assert_eq!(report.attached.len(), 1);
        assert!(report.attached[0].renamed);
        assert_eq!(report.attached[0].name, "shot-1.png");
        assert_eq!(fs::read(att_dir.join("shot.png")).unwrap(), b"OLD");
        assert_eq!(fs::read(att_dir.join("shot-1.png")).unwrap(), b"NEW");
    }

    #[test]
    fn collision_within_a_single_batch_renames_per_file() {
        let tmp = repo_with_issue("calm-quiet-otter");
        let src_dir1 = tempfile::tempdir().unwrap();
        let src_dir2 = tempfile::tempdir().unwrap();
        let a = write_src(src_dir1.path(), "log.txt", b"A");
        let b = write_src(src_dir2.path(), "log.txt", b"B");

        let report = attach_files(tmp.path(), "calm-quiet-otter", &[a, b]).unwrap();
        assert_eq!(report.attached[0].name, "log.txt");
        assert!(!report.attached[0].renamed);
        assert_eq!(report.attached[1].name, "log-1.txt");
        assert!(report.attached[1].renamed);
        let att_dir = tmp.path().join("issues/calm-quiet-otter/attachments");
        assert_eq!(fs::read(att_dir.join("log.txt")).unwrap(), b"A");
        assert_eq!(fs::read(att_dir.join("log-1.txt")).unwrap(), b"B");
    }

    #[test]
    fn unknown_slug_is_a_validation_error() {
        let tmp = repo_with_issue("calm-quiet-otter");
        let src_dir = tempfile::tempdir().unwrap();
        let s = write_src(src_dir.path(), "x.txt", b"x");
        let err = attach_files(tmp.path(), "no-such-slug", &[s]).unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "{err:?}");
    }

    #[test]
    fn directory_source_is_rejected() {
        let tmp = repo_with_issue("calm-quiet-otter");
        let src_dir = tempfile::tempdir().unwrap();
        let nested = src_dir.path().join("folder");
        fs::create_dir(&nested).unwrap();
        let err = attach_files(tmp.path(), "calm-quiet-otter", &[nested]).unwrap_err();
        match err {
            MutateError::Validation(m) => assert!(m.contains("directory"), "{m}"),
            other => panic!("{other:?}"),
        }
        // Nothing was attached so the attachments dir wasn't materialised.
        assert!(!tmp
            .path()
            .join("issues/calm-quiet-otter/attachments")
            .exists());
    }

    #[test]
    fn missing_source_is_a_validation_error() {
        let tmp = repo_with_issue("calm-quiet-otter");
        let err =
            attach_files(tmp.path(), "calm-quiet-otter", &[PathBuf::from("/no/such/file")])
                .unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)));
    }

    #[test]
    fn empty_sources_is_a_validation_error() {
        let tmp = repo_with_issue("calm-quiet-otter");
        assert!(matches!(
            attach_files(tmp.path(), "calm-quiet-otter", &[]),
            Err(MutateError::Validation(_))
        ));
    }

    #[test]
    fn multi_part_extension_is_preserved_across_rename() {
        let tmp = repo_with_issue("calm-quiet-otter");
        let att_dir = tmp.path().join("issues/calm-quiet-otter/attachments");
        fs::create_dir_all(&att_dir).unwrap();
        fs::write(att_dir.join("dump.tar.gz"), b"OLD").unwrap();
        let src_dir = tempfile::tempdir().unwrap();
        let src = write_src(src_dir.path(), "dump.tar.gz", b"NEW");
        let report = attach_files(tmp.path(), "calm-quiet-otter", &[src]).unwrap();
        assert_eq!(report.attached[0].name, "dump-1.tar.gz");
    }

    #[test]
    fn dotfile_basename_keeps_its_leading_dot() {
        assert_eq!(split_stem_ext(".envrc"), (".envrc", ""));
        assert_eq!(split_stem_ext("foo.txt"), ("foo", "txt"));
        assert_eq!(split_stem_ext("a.tar.gz"), ("a", "tar.gz"));
        assert_eq!(split_stem_ext("README"), ("README", ""));
    }
}
