//! Plan-then-execute migration of legacy `issues/{open,closed}/<slug>/`
//! directories into the flat `issues/<slug>/` layout.
//!
//! Two invariants this module enforces by construction:
//!
//! 1. The plan is opaque and move-only. Fields are private; `Clone` is
//!    not derived, so the executor's consume-by-value signature
//!    actually enforces single execution.
//! 2. Execution requires `&WriteLock` *and* the lock's repo root must
//!    match the plan's repo root. Combined, these prove "this caller
//!    holds the write lock for the repo this plan operates on" — the
//!    weaker "some lock exists" guarantee was a false-capability
//!    claim and is no longer accepted.
//!
//! Legacy `<NN>-<slug>` numbered directories (e.g. `12-old-bug`) are
//! out of scope for the flat-layout migration — they are doctor's
//! numbered-legacy migration's responsibility. The planner skips them
//! both at plan time (so they don't generate spurious conflicts) and
//! by construction (so flat-layout execution can't pre-empt the
//! numbered migration).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::mutate::WriteLock;
use crate::parser;
use crate::slug;

#[derive(Debug, Clone)]
pub struct MigrateMove {
    pub slug: String,
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug, Clone)]
pub struct MigrateConflict {
    pub slug: String,
    pub detail: String,
}

/// A single planned rename. Fields are private so external code cannot
/// fabricate one with paths outside `root/issues/`. Constructable only
/// by `plan_migrate_layout`.
#[derive(Debug, Clone)]
pub struct PlannedMove {
    slug: String,
    from: PathBuf,
    to: PathBuf,
}

impl PlannedMove {
    pub fn slug(&self) -> &str {
        &self.slug
    }
    pub fn from(&self) -> &Path {
        &self.from
    }
    pub fn to(&self) -> &Path {
        &self.to
    }
}

/// Opaque, move-only plan. Carries the originating `root` so the
/// executor can re-validate that every `from`/`to` lives under
/// `root/issues/` and matches the lock's root.
///
/// Two-pass plan-then-execute: discover everything → classify into
/// `moves` and `conflicts` → if any conflict exists, the `moves` list
/// is empty so callers cannot accidentally execute a partial migration
/// (C6, M7).
///
/// Skips legacy directory entries whose names don't pass `slug::is_valid`
/// — `issues/open/scratchwork` (or any non-kebab name) is reported as a
/// `MigrateConflict` rather than silently migrated to `issues/scratchwork`
/// (M6).
#[derive(Debug)]
pub struct MigrateLayoutPlan {
    root: PathBuf,
    moves: Vec<PlannedMove>,
    conflicts: Vec<MigrateConflict>,
}

impl MigrateLayoutPlan {
    /// The repo root this plan was computed against.
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn moves(&self) -> &[PlannedMove] {
        &self.moves
    }
    pub fn conflicts(&self) -> &[MigrateConflict] {
        &self.conflicts
    }
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty() && self.conflicts.is_empty()
    }
}

/// Read-only plan: discover what would move and what conflicts. No renames.
/// Safe to call without the write lock — callers wanting consistency
/// should re-run after acquiring the lock, or pass the resulting plan
/// straight to [`execute_migrate_layout_plan`] which re-validates paths.
pub fn plan_migrate_layout(root: &Path) -> Result<MigrateLayoutPlan> {
    // Canonicalise the root once, here. Storing the canonical form on
    // the plan means `check_lock_matches_plan` is a pure comparison
    // (no I/O at execute time, no asymmetric canonicalize-now-vs-later
    // window), and any symlink resolution differences between plan
    // and execute disappear from the trust boundary.
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("cannot canonicalize repo root {}", root.display()))?;
    let issues = canonical_root.join("issues");

    let mut by_slug: BTreeMap<String, Vec<(PathBuf, &'static str)>> = BTreeMap::new();
    let mut conflicts = Vec::new();
    for legacy in ["open", "closed"] {
        let legacy_dir = issues.join(legacy);
        let rd = match fs::read_dir(&legacy_dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("cannot read legacy issue directory {}", legacy_dir.display())
                });
            }
        };
        for entry in rd {
            let entry = entry.with_context(|| {
                format!("cannot read entry under {}", legacy_dir.display())
            })?;
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // Numbered-legacy `<NN>-<slug>` directories are doctor's
            // responsibility *only when the suffix isn't itself a
            // valid slug*. Skipping every `<digits>-<rest>` would
            // silently steal user-named slugs like `2-factor-auth` or
            // `100-things-to-fix` (digit-leading kebab is permitted by
            // `slug::is_valid`) into the numbered-legacy pipeline,
            // which would auto-generate a new canonical slug and
            // destroy the user's intended name. The intent is: only
            // names that fail `slug::is_valid` AND parse as numbered
            // (e.g. `12-Old_Bug`) are genuinely numbered-legacy here.
            if !slug::is_valid(&name) && parser::parse_legacy_dir(&name).is_some() {
                continue;
            }
            if !slug::is_valid(&name) {
                // M6: don't silently migrate non-slug-shaped names
                // (and not numbered-legacy, handled above).
                conflicts.push(MigrateConflict {
                    slug: name.clone(),
                    detail: format!(
                        "{} is not a valid slug shape — rename or move out of issues/{} before migrating",
                        entry.path().display(),
                        legacy
                    ),
                });
                continue;
            }
            by_slug
                .entry(name)
                .or_default()
                .push((entry.path(), legacy));
        }
    }

    // Pass 2: classify. flat-exists OR multiple-legacy → conflict.
    let mut moves: Vec<PlannedMove> = Vec::new();
    for (slug, locations) in by_slug {
        let dest = issues.join(&slug);
        // Use `symlink_metadata` so a broken symlink at `dest` is
        // detected as a conflict (Path::exists() returns false for
        // those and would let the executor `fs::rename` over them).
        if fs::symlink_metadata(&dest).is_ok() {
            conflicts.push(MigrateConflict {
                slug: slug.clone(),
                detail: format!(
                    "both flat ({}) and legacy ({}) exist",
                    dest.display(),
                    locations
                        .iter()
                        .map(|(p, _)| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
            continue;
        }
        if locations.len() > 1 {
            conflicts.push(MigrateConflict {
                slug: slug.clone(),
                detail: format!(
                    "slug exists in both legacy folders ({})",
                    locations
                        .iter()
                        .map(|(p, _)| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" and ")
                ),
            });
            continue;
        }
        let (src, _) = locations.into_iter().next().unwrap();
        moves.push(PlannedMove {
            slug,
            from: src,
            to: dest,
        });
    }

    // C6: all-or-nothing. If any conflict exists, the plan carries no
    // moves so callers don't accidentally execute a partial migration.
    if !conflicts.is_empty() {
        return Ok(MigrateLayoutPlan {
            root: canonical_root.clone(),
            moves: Vec::new(),
            conflicts,
        });
    }

    Ok(MigrateLayoutPlan {
        root: canonical_root,
        moves,
        conflicts,
    })
}

/// Outcome of a migration execution. On the error path, `migrated`
/// still carries the renames that completed before the failure so
/// callers can report partial progress instead of silently dropping
/// it. Callers must pattern-match the struct (or destructure the
/// fields) — there is intentionally no `into_result()` collapse,
/// because that would discard the structured `migrated` list and
/// defeat the partial-state guarantee.
pub struct ExecuteOutcome {
    pub migrated: Vec<MigrateMove>,
    pub error: Option<anyhow::Error>,
}

/// Execute a previously-planned migration. The `&WriteLock` parameter
/// is the type-system contract that the caller holds the repo write
/// lock; it is checked against `plan.root()` so a lock acquired for a
/// *different* repo cannot satisfy it.
///
/// Re-validates that every `from`/`to` is under `plan.root/issues/`
/// using uniform canonicalisation — both endpoints are canonicalised
/// via the same code path, and any canonicalisation failure is a hard
/// error (no silent fallback to lexical comparison).
///
/// Returns an [`ExecuteOutcome`] so the caller can render partial
/// progress on a mid-loop failure. Use `outcome.into_result()` when
/// "succeeded with N moves OR failed without losing what already
/// completed" is enough.
pub fn execute_migrate_layout_plan(
    plan: MigrateLayoutPlan,
    lock: &WriteLock,
) -> ExecuteOutcome {
    if let Err(e) = check_lock_matches_plan(&plan, lock) {
        return ExecuteOutcome {
            migrated: Vec::new(),
            error: Some(e),
        };
    }

    if !plan.conflicts.is_empty() {
        return ExecuteOutcome {
            migrated: Vec::new(),
            error: Some(anyhow::anyhow!(
                "migration plan has {} unresolved conflict(s); refusing to execute",
                plan.conflicts.len()
            )),
        };
    }

    let issues = plan.root.join("issues");
    let issues_canonical = match fs::canonicalize(&issues) {
        Ok(p) => p,
        Err(e) => {
            return ExecuteOutcome {
                migrated: Vec::new(),
                error: Some(anyhow::Error::new(e).context(format!(
                    "cannot canonicalize issues root {}",
                    issues.display()
                ))),
            };
        }
    };

    let mut migrated = Vec::with_capacity(plan.moves.len());
    for mv in plan.moves {
        if let Err(e) = validate_under(&issues_canonical, &mv.from, "from")
            .and_then(|_| validate_under(&issues_canonical, &mv.to, "to"))
        {
            return ExecuteOutcome {
                migrated,
                error: Some(e),
            };
        }

        if let Err(e) = fs::rename(&mv.from, &mv.to).with_context(|| {
            format!("cannot rename {} → {}", mv.from.display(), mv.to.display())
        }) {
            return ExecuteOutcome {
                migrated,
                error: Some(e),
            };
        }
        migrated.push(MigrateMove {
            slug: mv.slug,
            from: mv.from,
            to: mv.to,
        });
    }

    // Cleanup of empty `issues/{open,closed}` parent dirs is the
    // caller's responsibility — `doctor::apply` invokes
    // `prune_empty_legacy_parents` after both flat-layout and
    // numbered-legacy migrations, which is the only spot that sees
    // both passes complete. Running it here too would be a duplicate
    // syscall and contradict the "one source of truth" placement.

    ExecuteOutcome {
        migrated,
        error: None,
    }
}

/// Best-effort prune of empty `issues/{open,closed}` parent dirs.
/// Idempotent and lock-free-safe to call from any post-migration
/// phase (flat-layout, numbered-legacy, future migrations) — the
/// callers already hold the repo write lock. Errors other than
/// "expected" outcomes (already gone, non-empty because user left a
/// stray file) are swallowed so partial cleanup state never converts
/// a successful migration into an error.
pub fn prune_empty_legacy_parents(issues_root: &Path) {
    for legacy in ["open", "closed"] {
        let p = issues_root.join(legacy);
        if !p.is_dir() {
            continue;
        }
        match fs::remove_dir(&p) {
            Ok(()) => {}
            Err(e) if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) => {}
            Err(_) => {
                // Permission denied / other I/O — leave the directory
                // in place; the next doctor run will retry.
            }
        }
    }
}

fn check_lock_matches_plan(plan: &MigrateLayoutPlan, lock: &WriteLock) -> Result<()> {
    // Both `plan.root` and `lock.canonical_root()` were canonicalised
    // at construction time, so this is a pure comparison.
    if plan.root() != lock.canonical_root() {
        bail!(
            "write lock is for {}, but plan was built for {}; refusing to execute",
            lock.canonical_root().display(),
            plan.root().display()
        );
    }
    Ok(())
}

/// Reject any planned path that does not start with `issues_root`.
/// Resolves symlinks via the path's parent (which always exists for
/// the planned `from` and `to` of a rename), then re-joins the file
/// name. Canonicalisation failure is a hard error — the validation
/// must not silently degrade to lexical comparison, since
/// `issues_root` is canonical and an asymmetric mix of canonical and
/// lexical paths produces unpredictable rejections (and, worse,
/// possible false acceptances).
fn validate_under(issues_root: &Path, path: &Path, label: &str) -> Result<()> {
    let canonical = canonicalize_via_parent(path).with_context(|| {
        format!(
            "cannot canonicalize migration plan {label} path {}; refusing to execute",
            path.display()
        )
    })?;
    if canonical.starts_with(issues_root) {
        Ok(())
    } else {
        bail!(
            "migration plan {label} path {} resolves to {}, outside {}; refusing to execute",
            path.display(),
            canonical.display(),
            issues_root.display()
        )
    }
}

fn canonicalize_via_parent(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("path has no parent: {}", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("path has no file name: {}", path.display()))?;
    let parent_canonical = fs::canonicalize(parent)
        .with_context(|| format!("cannot canonicalize {}", parent.display()))?;
    Ok(parent_canonical.join(file_name))
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

    fn write_legacy(root: &Path, folder: &str, slug: &str) {
        let dir = root.join("issues").join(folder).join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\nstatus: open\n---\n").unwrap();
    }

    #[test]
    fn plan_picks_up_legacy_dirs() {
        let tmp = fresh_repo();
        write_legacy(tmp.path(), "open", "alpha-bravo");
        write_legacy(tmp.path(), "closed", "charlie-delta");

        let plan = plan_migrate_layout(tmp.path()).unwrap();
        assert!(plan.conflicts().is_empty());
        let slugs: Vec<_> = plan.moves().iter().map(|m| m.slug().to_string()).collect();
        assert_eq!(slugs, vec!["alpha-bravo", "charlie-delta"]);
    }

    #[test]
    fn plan_flat_exists_is_conflict_and_clears_moves() {
        let tmp = fresh_repo();
        write_legacy(tmp.path(), "open", "echo-foxtrot");
        // Flat already exists too — conflict.
        fs::create_dir_all(tmp.path().join("issues/echo-foxtrot")).unwrap();

        let plan = plan_migrate_layout(tmp.path()).unwrap();
        assert!(plan.has_conflicts());
        assert_eq!(plan.moves().len(), 0, "C6: conflicts ⇒ no moves");
    }

    #[test]
    fn plan_invalid_slug_is_conflict() {
        let tmp = fresh_repo();
        let bad = tmp.path().join("issues/open/Not_A_Slug");
        fs::create_dir_all(&bad).unwrap();

        let plan = plan_migrate_layout(tmp.path()).unwrap();
        assert!(plan.has_conflicts());
        assert!(plan
            .conflicts()
            .iter()
            .any(|c| c.detail.contains("not a valid slug shape")));
    }

    #[test]
    fn plan_skips_only_invalid_slug_numbered_legacy_dirs() {
        // The flat-layout planner defers to doctor's numbered-legacy
        // migration ONLY for names that genuinely fail `slug::is_valid`
        // and match `parse_legacy_dir` (e.g. `42-Old_Mixed`). A
        // digit-leading name that *is* a valid slug — either a real
        // numbered-legacy dir like `12-old-bug` or a user-chosen slug
        // like `2-factor-auth` — must go through the normal
        // flat-layout migration so the user's intended name is
        // preserved (or, for legacy numbered dirs, the post-migration
        // rescan picks them up at the flat location).
        let tmp = fresh_repo();
        write_legacy(tmp.path(), "open", "12-old-bug"); // valid slug + numbered: flat
        write_legacy(tmp.path(), "open", "2-factor-auth"); // valid slug + numbered: flat
        write_legacy(tmp.path(), "open", "42-Old_Mixed"); // invalid slug + numbered: skip

        let plan = plan_migrate_layout(tmp.path()).unwrap();
        assert!(plan.conflicts().is_empty(), "no conflicts expected");
        let mut slugs: Vec<_> = plan.moves().iter().map(|m| m.slug().to_string()).collect();
        slugs.sort();
        assert_eq!(
            slugs,
            vec!["12-old-bug", "2-factor-auth"],
            "valid-slug numbered names must reach flat-layout migration; \
             only invalid-slug numbered names are deferred to doctor"
        );
    }

    #[test]
    fn execute_refuses_plan_with_conflicts() {
        let tmp = fresh_repo();
        write_legacy(tmp.path(), "open", "golf-hotel");
        fs::create_dir_all(tmp.path().join("issues/golf-hotel")).unwrap();
        let plan = plan_migrate_layout(tmp.path()).unwrap();
        assert!(plan.has_conflicts());

        let lock = WriteLock::acquire(tmp.path()).unwrap();
        let outcome = execute_migrate_layout_plan(plan, &lock);
        assert!(outcome.migrated.is_empty());
        let err = outcome.error.expect("expected error");
        assert!(err.to_string().contains("unresolved conflict"));
    }

    #[test]
    fn execute_renames_planned_moves() {
        let tmp = fresh_repo();
        write_legacy(tmp.path(), "open", "india-juliet");
        let plan = plan_migrate_layout(tmp.path()).unwrap();

        let lock = WriteLock::acquire(tmp.path()).unwrap();
        let outcome = execute_migrate_layout_plan(plan, &lock);
        assert!(outcome.error.is_none(), "expected success: {:?}", outcome.error);
        assert_eq!(outcome.migrated.len(), 1);
        assert!(tmp.path().join("issues/india-juliet/item.md").exists());
        // Legacy parent pruning is `doctor::apply`'s responsibility
        // (it runs after both flat-layout and numbered-legacy passes
        // have completed), not the executor's. Verify the explicit
        // helper instead.
        prune_empty_legacy_parents(&tmp.path().join("issues"));
        assert!(!tmp.path().join("issues/open").exists(), "legacy parent pruned");
    }

    #[test]
    fn execute_rejects_lock_for_different_repo() {
        let tmp_a = fresh_repo();
        let tmp_b = fresh_repo();
        write_legacy(tmp_a.path(), "open", "kilo-lima");

        let plan = plan_migrate_layout(tmp_a.path()).unwrap();
        let lock_b = WriteLock::acquire(tmp_b.path()).unwrap();

        let outcome = execute_migrate_layout_plan(plan, &lock_b);
        assert!(outcome.migrated.is_empty());
        let err = outcome.error.expect("expected error").to_string();
        assert!(
            err.contains("write lock is for") && err.contains("plan was built for"),
            "expected cross-repo error, got: {err}"
        );
        // Plan was not executed against repo A.
        assert!(tmp_a.path().join("issues/open/kilo-lima/item.md").exists());
    }

    #[test]
    fn execute_returns_partial_progress_on_mid_loop_failure() {
        // Force a mid-loop failure by removing the source of the second
        // planned move after planning. The first rename succeeds; the
        // second fails on `fs::rename`'s ENOENT. The outcome must carry
        // the first as migrated and the error as the cause.
        let tmp = fresh_repo();
        write_legacy(tmp.path(), "open", "mike-november");
        write_legacy(tmp.path(), "open", "oscar-papa");
        let plan = plan_migrate_layout(tmp.path()).unwrap();

        // Sabotage the second move's source.
        fs::remove_dir_all(tmp.path().join("issues/open/oscar-papa")).unwrap();

        let lock = WriteLock::acquire(tmp.path()).unwrap();
        let outcome = execute_migrate_layout_plan(plan, &lock);
        assert_eq!(
            outcome.migrated.len(),
            1,
            "first move must be reported as migrated"
        );
        assert_eq!(outcome.migrated[0].slug, "mike-november");
        assert!(outcome.error.is_some(), "second move must surface error");
    }
}
