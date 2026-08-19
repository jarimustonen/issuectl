use super::*;

/// `issuectl close` semantics: read current type/status, reject if
/// already closing, default the status from the issue type, then apply
/// a status PATCH — all under a single flock so the type read cannot
/// race a concurrent mutation that flips it (M4).
///
/// `status_override` mirrors `--status`. When `None`, the default is
/// `fixed` for `type: bug`, `done` otherwise.
#[allow(clippy::too_many_arguments)]
pub fn close_issue(
    root: &Path,
    slug: &str,
    status_override: Option<String>,
    closed_by: Option<String>,
    comment: Option<String>,
    commits: Vec<CommitSpec>,
    expected_version: Option<String>,
) -> Result<UpdateOutcome, MutateError> {
    close_issue_via(
        root,
        slug,
        status_override,
        closed_by,
        comment,
        commits,
        expected_version,
        &SystemClock,
    )
}

/// Clock-injected variant of [`close_issue`].
#[allow(clippy::too_many_arguments)]
pub fn close_issue_via(
    root: &Path,
    slug: &str,
    status_override: Option<String>,
    closed_by: Option<String>,
    comment: Option<String>,
    commits: Vec<CommitSpec>,
    expected_version: Option<String>,
    clock: &dyn Clock,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }
    // `--as` is optional on `close` (unlike `note`, where it is
    // required), but when present it must satisfy the same author
    // grammar `note` uses so the closer attribution is a well-formed,
    // hash-stable token in the same vocabulary. Recorded as the
    // `closed_by:` frontmatter field alongside the auto-stamped
    // `closed:` date — see the status branch in `update_issue_under_lock`.
    // Normalize the closer attribution through the shared author seam:
    // a single leading `@` is stripped (so `close --as "@alice"` stores
    // `alice`, matching how the sigil is *shown* in headings) and the
    // remainder must satisfy the same grammar `note` uses. The
    // normalized token feeds both the `closed_by:` slot and the
    // `--comment` resolution note below, so `close --note --as` is
    // attributed consistently.
    let closed_by = match closed_by {
        Some(author) => Some(
            crate::body_sections::normalize_author(&author)
                .map_err(|e| MutateError::Validation(e.to_string()))?,
        ),
        None => None,
    };

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    // `update_issue_under_lock` runs `ensure_default_written` and the
    // legacy → flat migration only after every validation step has
    // passed. We therefore locate read-only here so a status-precondition
    // failure (already-closing issue) leaves no repo side effects.
    let item_path = locate_for_dry_run(root, slug)?;
    let item = write::read_item(&item_path).map_err(MutateError::Io)?;
    let schema =
        crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let current_status = item
        .frontmatter
        .get(serde_yaml::Value::String("status".into()))
        .and_then(|v| v.as_str())
        .unwrap_or("open")
        .to_string();
    if crate::schema::is_closing(&schema, &current_status) {
        return Err(MutateError::Validation(format!(
            "issue {slug} already has a closing status ({current_status}); use `update` to change status"
        )));
    }
    let issue_type = item
        .frontmatter
        .get(serde_yaml::Value::String("type".into()))
        .and_then(|v| v.as_str())
        .unwrap_or("bug")
        .to_string();
    // Default-status selection: bug → `fixed`, anything else → `done`.
    // The brief explicitly keeps built-in defaults — projects that
    // want a custom closing status as the `close` default must pass
    // it via `--status`. Schema validation under-lock then rejects
    // values the project's `status` enum disallows.
    let resolved_status = status_override.unwrap_or_else(|| {
        if issue_type == "bug" {
            "fixed".to_string()
        } else {
            "done".to_string()
        }
    });
    // `close` must land on a *closing* status. Schema validation under
    // lock only checks that the value is in the `status` enum, not that
    // it closes the issue — so without this guard `close --status open`
    // (or a schema that reclassifies `fixed` as active) would run the
    // reopen branch, leaving the issue active. Combined with `--as` that
    // produced an active issue carrying `closed_by`. Reject early.
    if !crate::schema::is_closing(&schema, &resolved_status) {
        return Err(MutateError::Validation(format!(
            "close status {resolved_status:?} is not a closing status; \
             use `update --status` to move an issue between active states"
        )));
    }

    // `close --comment/--note` records the closing rationale as a
    // timestamped block in a `## Resolution` section, appended via the
    // same body-op path (and same flock) as the status flip so the note
    // and the close land atomically — all-or-nothing. The block is
    // attributed to the closer (`--as`) when given; an anonymous close
    // records it under a stable `issuectl` sentinel so the managed
    // `### <ts> · @<author>` block shape stays well-formed. `validate()`
    // rejects an empty/whitespace comment via `validate_message`.
    let resolution_op = comment.map(|message| {
        let author = closed_by.clone().unwrap_or_else(|| "issuectl".to_string());
        BodyOp::AppendNote(AppendNoteOp {
            author,
            message,
            section: NoteSection::Resolution,
        })
    });

    let req = UpdateIssueRequest {
        expected_version,
        status: Patch::Set(resolved_status),
        // Closer attribution rides the same under-lock write as the
        // status flip via the first-class `closed_by` slot (NOT a custom
        // field): it is validated in `UpdateIssueRequest::validate`,
        // stamped alongside `closed:` in the status branch, surfaces in
        // `show --json` via the typed `Issue::closed_by` field, and is
        // folded into the version hash. Reopening clears it in lockstep
        // with `closed:`.
        closed_by: match closed_by {
            Some(author) => Patch::Set(author),
            None => Patch::Unspecified,
        },
        add_commits: commits,
        body_ops: resolution_op.into_iter().collect(),
        ..Default::default()
    };
    // _lock drops at end-of-scope after the locked update path returns.
    // We call the under-lock helper directly so we don't double-acquire
    // (fs2 advisory flock is per-fd; nested `WriteLock::acquire` would
    // deadlock on Linux).
    let mut req_normalized = req;
    let normalized_add_related = crate::refs::normalize_related_refs(&req_normalized.add_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let normalized_remove_related =
        crate::refs::normalize_related_refs(&req_normalized.remove_related)
            .map_err(|e| MutateError::Validation(e.to_string()))?;
    req_normalized.add_related = normalized_add_related;
    req_normalized.remove_related = normalized_remove_related;
    req_normalized.validate()?;
    let rules = load_validated_rules(root, &schema)?;
    update_issue_under_lock(
        root,
        slug,
        item_path,
        req_normalized,
        &schema,
        &rules,
        clock,
    )
}

/// Apply the *same* mutation to many issues under a single repo-wide
/// flock. Powers `issuectl bulk`.
///
/// `make_req(dry_run)` must return a fresh, content-identical request
/// each time it is called. Each write consumes its request, so callers
/// commonly clone one validated request and set its per-phase dry-run bit.
///
/// Semantics, in order:
/// 1. Acquire the repo-wide write lock **once** for the whole batch.
/// 2. Load schema + transition rules **once**.
/// 3. Phase 1 — validate and plan every target as an in-memory dry-run.
///    No file is written. Any validation failure aborts here with the
///    offending slug, so a bad value on the last target writes nothing.
/// 4. Phase 2 (skipped when `dry_run`) — write every target for real.
///
/// Holding one lock across both phases closes the time-of-check /
/// time-of-use window a per-call-locking loop would open: no concurrent
/// writer can slip between a target's validation and its write, and the
/// whole batch is serialized against other writers. This is the "one
/// commit" guarantee `bulk` advertises. The only residual non-atomicity
/// is a mid-phase-2 I/O error (disk full, EIO): earlier targets are
/// already on disk. That case returns an `Io` error naming how many
/// landed so the caller can surface the partial set.
pub fn bulk_update(
    root: &Path,
    slugs: &[String],
    mut make_req: impl FnMut(bool) -> UpdateIssueRequest,
    dry_run: bool,
) -> Result<Vec<UpdateOutcome>, MutateError> {
    for slug in slugs {
        if !crate::slug::is_valid(slug) {
            return Err(MutateError::Validation(format!(
                "invalid slug shape: {slug:?}"
            )));
        }
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    let schema =
        crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let rules = load_validated_rules(root, &schema)?;

    // Phase 1: validate + plan every target with a dry-run request, so
    // nothing is written until all targets are known-good. Dry-run mode
    // returns these planned outcomes directly (they carry the diff bytes).
    let mut planned = Vec::with_capacity(slugs.len());
    for slug in slugs {
        let req = prepare_bulk_req(make_req(true))?;
        let item_path = locate_for_dry_run(root, slug)?;
        let outcome =
            update_issue_under_lock(root, slug, item_path, req, &schema, &rules, &SystemClock)
                .map_err(|e| with_slug_context(slug, e))?;
        planned.push(outcome);
    }
    if dry_run {
        return Ok(planned);
    }

    // Phase 2: real writes. Every target already validated under this
    // same lock, so only I/O failures are expected from here on.
    let mut outcomes = Vec::with_capacity(slugs.len());
    for (i, slug) in slugs.iter().enumerate() {
        let req = prepare_bulk_req(make_req(false))?;
        let item_path = locate_for_dry_run(root, slug)?;
        match update_issue_under_lock(root, slug, item_path, req, &schema, &rules, &SystemClock) {
            Ok(o) => outcomes.push(o),
            Err(e) => {
                let written = slugs[..i]
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(MutateError::Io(anyhow!(
                    "bulk write failed on {slug} after writing {} issue(s) [{written}]: {}",
                    i,
                    e
                )));
            }
        }
    }
    Ok(outcomes)
}

/// Normalize related-ref shapes and run request validation — the part
/// of `update_issue` that runs before the lock. Shared by every
/// `bulk_update` target so a bulk write enforces the exact same
/// per-request contract as a single `update`.
pub(crate) fn prepare_bulk_req(req: UpdateIssueRequest) -> Result<UpdateIssueRequest, MutateError> {
    let add = crate::refs::normalize_related_refs(&req.add_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let remove = crate::refs::normalize_related_refs(&req.remove_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let add_bb = crate::refs::normalize_related_refs(&req.add_blocked_by)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let rem_bb = crate::refs::normalize_related_refs(&req.remove_blocked_by)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let mut req = req;
    req.add_related = add;
    req.remove_related = remove;
    req.add_blocked_by = add_bb;
    req.remove_blocked_by = rem_bb;
    req.validate()?;
    Ok(req)
}

/// Prefix a per-target error with its slug while preserving the error
/// variant (so the server/CLI keep their status mapping). Bulk writes
/// fail one slug at a time; naming it is the difference between an
/// actionable error and a mystery.
pub(crate) fn with_slug_context(slug: &str, e: MutateError) -> MutateError {
    use MutateError::*;
    match e {
        Validation(s) => Validation(format!("{slug}: {s}")),
        ConflictingIntent(s) => ConflictingIntent(format!("{slug}: {s}")),
        SchemaViolation(s) => SchemaViolation(format!("{slug}: {s}")),
        TransitionViolation(s) => TransitionViolation(format!("{slug}: {s}")),
        NotFound => Validation(format!("{slug}: issue not found")),
        other => other,
    }
}
