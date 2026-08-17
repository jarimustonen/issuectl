use super::*;

/// Run schema validation against a post-mutation frontmatter mapping.
/// Centralised so every body- and frontmatter-mutation entry point
/// (`update_issue_under_lock`, `update_body`, `note_issue`,
/// `toggle_checkbox`) enforces the same contract — schema runs once
/// per write, immediately before atomic write or dry-run return.
/// Join schema violations into a hard-fail message, conditionally
/// dropping `RequiredWhen` violations. A `required_when` constraint
/// (today: closing status implies `closed:`) is a lifecycle-consistency
/// rule that `doctor` owns and heals. When a mutation leaves a field's
/// `required_when` condition unsatisfied *without having touched either
/// that field or the `status` that drives the condition*, the violation
/// is a pre-existing inconsistency the user didn't introduce — blocking
/// an unrelated edit (e.g. a checkbox toggle on an already-`done` issue)
/// would be surprising, so it's dropped.
///
/// But when the mutation *did* write the field or `status`, the
/// violation is something this very write produced — e.g. explicitly
/// clearing `closed:` on a closing-status issue (`set closed ""`). That
/// must be rejected, not silently healed later, so the `RequiredWhen` is
/// kept. `written` is the set of frontmatter keys this mutation wrote;
/// body-only paths pass an empty set and so keep the lenient behaviour.
/// Returns `None` when nothing remains to fail on.
pub(crate) fn hard_schema_failure(
    violations: &[crate::schema::ViolationKind],
    written: &std::collections::BTreeSet<String>,
) -> Option<String> {
    // A `RequiredWhen` condition is gated solely on the issue's status
    // class (`schema::RequiredWhen` only carries `status_class`), so
    // `status` is the condition driver for *every* such violation. If
    // the format ever grows non-status drivers, this check must learn
    // the per-violation driver (see `ViolationKind::RequiredWhen`)
    // instead of assuming `status`.
    let status_written = written.contains("status");
    let msgs: Vec<String> = violations
        .iter()
        .filter(|v| match v {
            crate::schema::ViolationKind::RequiredWhen { field, .. } => {
                // Keep (enforce) only when this mutation touched the
                // required field itself or the status that triggers it.
                status_written || written.contains(field)
            }
            _ => true,
        })
        .map(|v| v.message())
        .collect();
    (!msgs.is_empty()).then(|| msgs.join("; "))
}

/// Body-only schema gate. Callers here never mutate frontmatter, so the
/// `written` set passed to `hard_schema_failure` is always empty and
/// `RequiredWhen` violations stay lenient. A future frontmatter-mutating
/// caller must NOT route through this helper — it would silently drop a
/// `RequiredWhen` it introduced; use `hard_schema_failure` with a real
/// `written` set instead (as `update_issue_under_lock` does).
pub(crate) fn validate_against_schema(
    root: &Path,
    frontmatter: &serde_yaml::Mapping,
) -> Result<(), MutateError> {
    let schema =
        crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let violations = crate::schema::validate(&schema, frontmatter);
    // Body-only mutation: status/closed are never written here, so the
    // empty `written` set preserves the lenient RequiredWhen behaviour.
    if let Some(msg) = hard_schema_failure(&violations, &std::collections::BTreeSet::new()) {
        return Err(MutateError::SchemaViolation(msg));
    }
    Ok(())
}

/// Apply a `Patch<String>` onto a frontmatter mapping. `Unspecified`
/// is a no-op; `Clear` removes the key; `Set(v)` sets the key.
///
/// Enforce epic↔non-epic invariants against post-patch frontmatter. A reporter
/// is the same role as an epic owner, so a lone reporter is migrated. This is
/// deliberately the only implicit conversion: an assignee or a conflicting
/// owner requires a caller decision and therefore gets an exact CLI command.
/// Only invoked on real type changes, preserving same-value idempotency.
pub(crate) fn reconcile_type_invariants(
    new_type: &str,
    fm: &mut serde_yaml::Mapping,
    slug: &str,
) -> Result<Option<String>, MutateError> {
    let get_nonempty = |key: &str| -> Option<String> {
        fm.get(serde_yaml::Value::String(key.into()))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    if new_type == "epic" {
        let reporter = get_nonempty("reporter");
        let owner = get_nonempty("owner");
        let assignee = get_nonempty("assignee");
        if assignee.is_some()
            || reporter
                .as_ref()
                .is_some_and(|r| owner.as_ref().is_some_and(|o| o != r))
        {
            let mut clears = Vec::new();
            if assignee.is_some() {
                clears.push("--no-assignee");
            }
            if reporter
                .as_ref()
                .is_some_and(|r| owner.as_ref().is_some_and(|o| o != r))
            {
                clears.push("--no-reporter");
            }
            return Err(MutateError::Validation(format!(
                "type {new_type:?} uses owner, not reporter/assignee; run `issuectl update {slug} {} --type epic`",
                clears.join(" ")
            )));
        }
        if let Some(reporter) = reporter {
            write::set_string(fm, "owner", &reporter);
            write::remove_key(fm, "reporter");
            return Ok(Some(format!(
                "@{slug}: migrated reporter {reporter:?} to owner while changing type to epic"
            )));
        }
    } else if get_nonempty("owner").is_some() {
        return Err(MutateError::Validation(format!(
            "type {new_type:?} does not use owner (only `epic` does); run `issuectl update {slug} --no-owner --type {new_type}`"
        )));
    }
    Ok(None)
}

pub(crate) fn apply_string_patch(item: &mut ItemFile, key: &str, p: &Patch<String>) {
    match p {
        Patch::Unspecified => {}
        Patch::Clear => write::remove_key(&mut item.frontmatter, key),
        Patch::Set(v) => write::set_string(&mut item.frontmatter, key, v),
    }
}

/// Where a real write to `slug` would land on disk after any layout
/// transition the write performs. Used by dry-run paths so the JSON
/// envelope's `final_dir` agrees with what a follow-up real write would
/// produce. Three cases:
///   - currently archived AND the post-mutation status is non-closing →
///     the real write unarchives it (see [`unarchive_if_active`]), so it
///     lands at the active flat root `issues/<slug>/`.
///   - currently archived AND staying closing → the real write leaves it
///     in cold storage, so the dir is its current archive path.
///   - active or legacy → the active flat root (legacy migrates to flat).
/// Without the archive cases, dry-run on an archived issue reported the
/// active root unconditionally while a non-reopening real write actually
/// lands back in the archive (and the inverse for reopens).
pub(crate) fn predicted_issue_dir(
    root: &Path,
    slug: &str,
    item_path: &Path,
    post_closing: bool,
) -> PathBuf {
    let archive_root = root.join("issues").join(repo::ARCHIVE_DIR);
    let in_archive = item_path
        .parent()
        .is_some_and(|p| p.starts_with(&archive_root));
    if in_archive && post_closing {
        // Stays in cold storage — report its current archive dir.
        return item_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.join("issues").join(slug));
    }
    root.join("issues").join(slug)
}

/// Locate the issue without migrating. Every mutation entry point
/// (CLI verbs and the unified PATCH path) now uses this read-only
/// locate; the legacy → flat directory rename and the default
/// `.schema.yaml` bootstrap are deferred until just before
/// `write_item_atomic` via `migrate_to_flat_if_legacy`. That guarantees
/// validation failures (schema, transition rules, body op match) leave
/// no repo side effects.
pub(crate) fn locate_for_dry_run(root: &Path, slug: &str) -> Result<PathBuf, MutateError> {
    use repo::LayoutState;
    match repo::resolve_layout(root, slug) {
        LayoutState::Flat { item_path }
        | LayoutState::Inbox { item_path }
        | LayoutState::Legacy { item_path, .. } => Ok(item_path),
        LayoutState::Ambiguous { paths } => Err(MutateError::AmbiguousSlug { paths }),
        LayoutState::Absent => Err(MutateError::NotFound),
        LayoutState::Invalid { reason, .. } => Err(MutateError::Io(anyhow!("{reason}"))),
    }
}

/// If the located item path is at a legacy `issues/{open,closed}/<slug>/`
/// directory, run the legacy → flat migration in-place and return the
/// new flat path. Otherwise return the path unchanged. Called after
/// validation has passed so the rename is part of the same atomic
/// success — never on a rolled-back transaction.
pub(crate) fn migrate_to_flat_if_legacy(
    root: &Path,
    slug: &str,
    item_path: &Path,
) -> Result<PathBuf, MutateError> {
    use repo::LayoutState;
    let needs_migration = matches!(repo::resolve_layout(root, slug), LayoutState::Legacy { .. });
    if !needs_migration {
        return Ok(item_path.to_path_buf());
    }
    repo::migrate_to_flat_inplace(root, slug).map_err(MutateError::Io)?;
    match repo::resolve_layout(root, slug) {
        LayoutState::Flat { item_path } | LayoutState::Inbox { item_path } => Ok(item_path),
        LayoutState::Absent => Err(MutateError::NotFound),
        LayoutState::Ambiguous { paths } => Err(MutateError::AmbiguousSlug { paths }),
        LayoutState::Invalid { reason, .. } => Err(MutateError::Io(anyhow!("{reason}"))),
        LayoutState::Legacy { .. } => Err(MutateError::Io(anyhow!(
            "post-migration state still classifies as legacy"
        ))),
    }
}

/// Lift an issue out of cold storage when a mutation leaves it in an
/// active (non-closing) state. When `post_closing` is false and the
/// issue's `item.md` currently lives under `issues/archive/YYYY/MM/`,
/// rename the issue directory back to the active root (`issues/<slug>/`)
/// — the inverse of the `archive` move. Returns the new `item.md` path
/// so the subsequent write lands on the active copy. No-op for issues
/// that aren't archived or whose post-mutation status is still closing.
///
/// The trigger is "archived AND now active", not strictly "reopened
/// (closing→active)": that also heals an archived issue whose status was
/// dragged active out-of-band (manual edit / external git op) and then
/// touched by an unrelated PATCH — `resolve_layout` already reads such an
/// issue as active, so leaving it physically archived is the same bug.
///
/// Runs under the caller's held flock and only after validation passed,
/// so it shares the archive move's all-or-nothing guarantee. Refuses
/// (rather than clobbering) if an active directory for the slug already
/// exists — that collision is `Ambiguous` and would have failed the
/// read-time locate, but the check is kept as defence in depth.
///
/// Failure mode if the later `write_item_atomic` errors after this
/// rename: the dir is at the active root carrying its still-closing
/// pre-mutation `item.md`, i.e. a closed-but-unarchived issue — a
/// self-consistent state (closed issues live at the active root until
/// the next `archive` run), not the "active-but-archived" inconsistency
/// this fix targets. Re-running the mutation completes cleanly.
pub(crate) fn unarchive_if_active(
    root: &Path,
    slug: &str,
    item_path: PathBuf,
    post_closing: bool,
) -> Result<PathBuf, MutateError> {
    if post_closing {
        return Ok(item_path);
    }
    // `archive_root` and `cur_dir` are both derived by joining the same
    // `root` (`item_path` came from `resolve_layout(root, …)`), so the
    // `starts_with` prefix test is robust to whatever base `root` carries
    // (relative, symlinked) — both sides share it.
    let archive_root = root.join("issues").join(repo::ARCHIVE_DIR);
    let cur_dir = item_path.parent().ok_or_else(|| {
        MutateError::Io(anyhow!("item.md has no parent: {}", item_path.display()))
    })?;
    if !cur_dir.starts_with(&archive_root) {
        return Ok(item_path); // not archived — nothing to lift
    }
    let dest_dir = root.join("issues").join(slug);
    if dest_dir.exists() {
        return Err(MutateError::Io(anyhow!(
            "cannot unarchive {slug}: active destination already exists: {} — resolve manually",
            dest_dir.display()
        )));
    }
    fs::rename(cur_dir, &dest_dir).map_err(|e| {
        MutateError::Io(anyhow!(
            "cannot unarchive {slug}: rename {} -> {} failed: {e}",
            cur_dir.display(),
            dest_dir.display()
        ))
    })?;
    // Best-effort prune of the now-possibly-empty YYYY/MM (and YYYY)
    // buckets, symmetric with the `archive` move creating them.
    // `remove_dir` only removes empty dirs, so a bucket still holding
    // other archived issues is left untouched.
    prune_empty_archive_buckets(cur_dir, &archive_root);
    Ok(dest_dir.join("item.md"))
}

/// Remove the now-orphaned `YYYY/MM` (then `YYYY`) archive bucket dirs
/// after a slug dir was moved out, walking up but never past — or onto —
/// `archive_root`. Best-effort: any non-empty dir stops the walk.
pub(crate) fn prune_empty_archive_buckets(moved_dir: &Path, archive_root: &Path) {
    let mut cur = moved_dir.parent();
    while let Some(dir) = cur {
        if dir == archive_root || !dir.starts_with(archive_root) {
            break;
        }
        if fs::remove_dir(dir).is_err() {
            break; // non-empty or gone — stop pruning
        }
        cur = dir.parent();
    }
}

/// Atomic write: stage as `.issuectl-tmp-…`, fsync, persist into
/// place. On Unix, best-effort fsync the parent directory after
/// rename. The tempfile prefix is the signal the watcher uses to
/// filter our own writes (§5.1).
pub fn write_item_atomic(target: &Path, item: &ItemFile) -> Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| anyhow!("target has no parent: {}", target.display()))?;
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let serialized = write::serialize_item(item)?;
    let mut tf = tempfile::Builder::new()
        .prefix(".issuectl-tmp-")
        .tempfile_in(dir)
        .with_context(|| format!("cannot create tempfile in {}", dir.display()))?;
    use std::io::Write;
    tf.as_file_mut()
        .write_all(serialized.as_bytes())
        .with_context(|| format!("cannot write {}", target.display()))?;
    tf.as_file()
        .sync_all()
        .with_context(|| format!("cannot fsync {}", target.display()))?;
    tf.persist(target)
        .map_err(|e| anyhow!("cannot persist tempfile: {e}"))?;
    #[cfg(unix)]
    {
        if let Err(err) = fsync_dir(dir) {
            eprintln!(
                "issuectl[mutate]: fsync_dir({}) failed: {err}",
                dir.display()
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    let f = File::open(dir)?;
    f.sync_all()
}

// ── Create / close ──────────────────────────────────────────────────────
