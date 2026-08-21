use super::*;

#[allow(dead_code)] // retained as the SystemClock convenience for inline tests.
pub(crate) fn apply(
    repo_root: &Path,
    actions: DoctorActions,
    lock: &crate::mutate::WriteLock,
) -> Result<ApplyOutcome> {
    apply_via(repo_root, actions, lock, &crate::clock::SystemClock)
}

/// Clock-injected apply phase used by [`run_via`].
pub(crate) fn apply_via(
    repo_root: &Path,
    mut actions: DoctorActions,
    lock: &crate::mutate::WriteLock,
    clock: &dyn crate::clock::Clock,
) -> Result<ApplyOutcome> {
    let mut outcome = ApplyOutcome::default();

    // Schema bootstrap runs UNCONDITIONALLY, before the preflight
    // refusal. The read-only `doctor` output advertises that
    // `.schema.yaml` will be auto-created on first `--fix`; gating
    // bootstrap on an empty preflight blocker list breaks that
    // promise for repos with any other violation present. The
    // operation is idempotent (`ensure_default_written` returns
    // `false` when the file exists) and writes a known-good template
    // — there is no failure mode where running it makes the repo
    // worse than before. (issue: @unreasonably-attractive-star)
    let issues_dir = repo_root.join("issues");
    fs::create_dir_all(&issues_dir)
        .with_context(|| format!("cannot create {}", issues_dir.display()))?;
    outcome.schema_bootstrapped = schema::ensure_default_written(repo_root)?;

    // Preflight: refuse to mutate when a layout-fatal blocker is
    // present (see `BlockerScope::ApplyPreflight` for the narrowed
    // list). Schema-shape findings deliberately do NOT block the
    // apply pipeline — the user can fix layout first and address
    // schema violations against the post-migration state. We DO NOT
    // `bail!` — the blockers go into the outcome so `--json --fix`
    // callers receive structured output instead of an anyhow-
    // formatted stderr blob (the AGENTS.md "always `--json` when
    // scripting" promise).
    if !actions.preflight_blockers.is_empty() {
        // Schema bootstrap above may have written `.schema.yaml`
        // before this preflight refusal — that's intentional and the
        // documented behaviour (issue: @unreasonably-attractive-star).
        // The Preflight invariant in `stop_with_blockers` accepts
        // `schema_bootstrapped` as the one allowed pre-preflight
        // write; no state masking is needed.
        outcome.stop_with_blockers(
            StopPhase::Preflight,
            std::mem::take(&mut actions.preflight_blockers),
        );
        return Ok(outcome);
    }

    // Orphan tempfile cleanup runs FIRST so paths recorded by scan()
    // are still valid: directory migration would invalidate them.
    apply_orphan_tempfiles(&mut actions, &mut outcome)?;

    // Alias coercion (legacy status/type values → canonical) runs
    // BEFORE status/folder reconciliation: reconciliation classifies a
    // status via the lifecycle layering, and a legacy value like
    // `resolved` resolves to the lenient `Active` default until it is
    // coerced to its canonical (`fixed`, closing) form. Coercing first
    // means reconciliation reasons about canonical statuses, not the
    // pre-migration ones. Both run before the flat-layout migration so
    // the rewrites land at the legacy path scan() recorded. Schema is
    // loaded post-bootstrap so a repo with no prior `.schema.yaml`
    // still gets the built-in alias table.
    let apply_schema = schema::load(repo_root)?;
    apply_alias_coercions(&mut actions, &mut outcome, &apply_schema, clock)?;
    apply_deferred_label_removal(&mut actions, &mut outcome);
    if outcome.apply_error.is_some() {
        return Ok(outcome);
    }

    // Status/folder reconciliation runs BEFORE the flat-layout
    // migration so the rewrites land at the legacy path that scan()
    // recorded; the subsequent migration moves the corrected file.
    apply_status_reconciliation(&mut actions, &mut outcome, clock)?;

    // Notes → Comments migration is independent of layout migration:
    // it touches body markdown of flat-layout dirs only, never moves
    // files. Run it FIRST so layout-conflict bail-outs don't block
    // unrelated body fixes (round-2 finding O18).
    rename_notes_to_comments(repo_root, &mut actions, &mut outcome)?;

    regenerate_agents_md(repo_root, &actions, &mut outcome)?;
    rewrite_legacy_issues_agents_md(repo_root, &actions, &mut outcome)?;

    // Flat-layout migration: any issue still under
    // `issues/{open,closed}/<slug>/` moves up to `issues/<slug>/`. The
    // pre-acquired write lock in `run` covers this — `execute_migrate_layout_plan`
    // is the lock-free body and must not re-acquire.
    let mut legacy_dirs = std::mem::take(&mut actions.legacy_dirs);
    if let Some(plan) = actions.flat_layout_plan.take() {
        if !plan.moves().is_empty() {
            // `ExecuteOutcome` carries partial progress on mid-loop
            // failure so the user-facing summary can still render
            // "moved A, B before failing on C".
            let exec_outcome = execute_migrate_layout_plan(plan, lock);
            outcome.flat_layout_migrated = exec_outcome.migrated;
            // Prune empty `issues/{open,closed}` parent dirs as soon
            // as the moves land — every code path below this point
            // can early-return (post-migration blocker bail, empty
            // `legacy_dirs`, or successful NN-rename), and the prune
            // is best-effort idempotent so calling it once here is
            // simpler than gating it at every exit.
            crate::migrate_layout::prune_empty_legacy_parents(&repo_root.join("issues"));
            if let Some(err) = exec_outcome.error {
                // Forward-progress only: surface the failure cause on
                // the structured outcome and bail. Returning `Err` here
                // would propagate past `render_text` / `render_json` and
                // strand the partial `flat_layout_migrated` (already on
                // disk) inside an anyhow text blob on stderr — invisible
                // to `--json` consumers.
                outcome.apply_error = Some(format!("{err:#}"));
                return Ok(outcome);
            }
            // Re-scan so the NN-rename pass operates on fresh
            // `old_path`s and picks up frontmatter-only legacy issues
            // that just moved into the flat layout.
            let fresh = scan_via(repo_root, clock)?;
            // Re-check `apply_blockers` (the layout-fatal subset)
            // against the fresh scan before the NN-rename phase.
            // Phase 5 can surface a layout-fatal condition that was
            // hidden by the pre-migration layout —
            // `populate_notes_migration` walks only flat-folder dirs,
            // so a `## Notes` / `## Comments` ambiguity in a body
            // that was still under `issues/{open,closed}/` is
            // invisible to the initial scan, and the planner's own
            // `flat_layout_conflicts` could surface only on the
            // post-move state in unusual layouts. NN-rename builds
            // `number_to_slug` against `legacy_dirs` and rewrites
            // refs + renames dirs based on it; running that pass
            // over a layout-unhealthy repo can rewrite refs to the
            // wrong target or have `fs::rename` overwrite a sibling.
            // We use `apply_blockers` (not the broader
            // `critical_blockers`) so newly-surfaced schema
            // violations don't strand the partial layout migration —
            // schema fixes are forward work the user does after the
            // layout is in place. Forward-progress only: rolling
            // back N partial renames is itself a multi-step
            // operation that can fail mid-rollback.
            let post_blockers = apply_blockers(&fresh);
            if !post_blockers.is_empty() {
                outcome.stop_with_blockers(StopPhase::PostApply, post_blockers);
                return Ok(outcome);
            }
            // Re-run the Notes → Comments rename against the
            // post-migration state. `populate_notes_migration` walks
            // only `folder == "flat"` dirs, so any issue still under
            // `issues/{open,closed}/<slug>/` whose body has `## Notes`
            // is invisible to the pre-migration scan. After phase 5
            // lifts it to `issues/<slug>/`, the rename is applicable —
            // running it here closes the one-shot `--fix` contract so
            // users don't have to invoke `doctor --fix` twice. Safe to
            // call twice in the same apply: `rename_notes_to_comments`
            // appends to `outcome.notes_renamed`, and the first call
            // already drained `actions.notes_to_rename`.
            actions.notes_to_rename = fresh.notes_to_rename;
            // Post-flat-layout dirs may now expose `## Notes`/`## Comments`
            // ambiguity that was invisible while still under `issues/{open,closed}/`.
            // Surface them via the same outcome field so they don't
            // silently disappear (issue: @doctor-fix-noop).
            actions.notes_conflicts = fresh.notes_conflicts;
            rename_notes_to_comments(repo_root, &mut actions, &mut outcome)?;
            legacy_dirs = fresh.legacy_dirs;
        }
    }

    if legacy_dirs.is_empty() {
        apply_inbox_migration(repo_root, &mut actions, &mut outcome, lock);
        return Ok(outcome);
    }

    // Build maps for reference rewriting.
    let mut number_to_slug: BTreeMap<u32, String> = BTreeMap::new();
    let mut dir_to_slug: BTreeMap<String, String> = BTreeMap::new();
    for m in &legacy_dirs {
        let _prev = number_to_slug.insert(m.old_number, m.new_slug.clone());
        // Duplicate legacy numbers are flagged via build_ambiguous below;
        // rewrites for those numbers will be skipped.
        dir_to_slug.insert(m.old_dir_name.clone(), m.new_slug.clone());
    }

    let ambiguous_numbers = build_ambiguous(&legacy_dirs);

    // Single-phase atomic rename: old dirname (`<NN>-<slug>`) and new
    // slug (`<intensifier-adj-noun>`) cannot collide, so the temp-suffix
    // shuffle that the previous version did is unnecessary — and worse,
    // an interruption mid-shuffle would leave `*.issuectl-doctor-<pid>`
    // dirs that no subsequent doctor run could recognize.
    for m in &legacy_dirs {
        if m.new_path.exists() {
            bail!("target slug dir already exists: {}", m.new_path.display());
        }
        fs::rename(&m.old_path, &m.new_path).with_context(|| {
            format!(
                "cannot rename {} to {}",
                m.old_path.display(),
                m.new_path.display()
            )
        })?;
    }

    for m in &legacy_dirs {
        let item_path = m.new_path.join("item.md");
        rewrite_item_frontmatter(&item_path, &m.new_slug, &number_to_slug, &ambiguous_numbers)?;
    }

    // Body-ref rewrites are scoped to `issues/` by default. Documentation
    // outside the issue tree (CHANGELOG, README, design docs) commonly
    // contains literal `#NN` strings that are not issue references, and
    // rewriting them silently is data loss. Users who want a wider sweep
    // can run grep + a one-time replace themselves.
    let issues_path = repo_root.join("issues");
    let scopes = vec![issues_path];
    let files_rewritten =
        rewrite_markdown_in_scopes(&scopes, &number_to_slug, &dir_to_slug, &ambiguous_numbers)?;
    outcome.files_rewritten = files_rewritten;
    outcome.legacy_dirs_migrated = legacy_dirs;

    // Prune empty `issues/{open,closed}` parent dirs again — covers
    // the numbered-legacy-only repo path where the flat-layout
    // planner had no moves and the earlier in-pipeline prune did not
    // run. Idempotent and best-effort.
    crate::migrate_layout::prune_empty_legacy_parents(&repo_root.join("issues"));

    apply_inbox_migration(repo_root, &mut actions, &mut outcome, lock);
    Ok(outcome)
}

/// Promote every stranded inbox draft through the same lock-aware mutation
/// body as the deprecated `triage` compatibility command. Earlier doctor
/// phases may rewrite these item files in place; moving them last keeps those
/// scan-time paths valid throughout the rest of the pipeline.
pub(crate) fn apply_inbox_migration(
    repo_root: &Path,
    actions: &mut DoctorActions,
    outcome: &mut ApplyOutcome,
    lock: &crate::mutate::WriteLock,
) {
    for (slug, _) in std::mem::take(&mut actions.inbox_drafts) {
        match crate::mutate::triage::triage_locked(repo_root, &slug, lock) {
            Ok(migrated) => outcome.inbox_drafts_migrated.push(migrated),
            Err(error) => {
                outcome.apply_error = Some(format!(
                    "failed to migrate deprecated inbox draft {slug}: {error}"
                ));
                return;
            }
        }
    }
    let inbox = repo_root.join("issues").join(crate::repo::INBOX_DIR);
    if inbox.is_dir()
        && fs::read_dir(&inbox)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false)
    {
        let _ = fs::remove_dir(inbox);
    }
}

/// Apply the Notes → Comments rename to every slug in
/// `actions.notes_to_rename`. Best-effort, sequential (per round-2
/// decision: `O17` is intentionally not preflight-bail). Conflicts
/// are populated by the upstream scan; this function does not
/// re-classify. Callable multiple times in one `apply` pass —
/// `mem::take` drains the input on each call and outcomes append to
/// `outcome.notes_renamed` / `outcome.notes_conflicts_at_apply`.
/// Regenerate the schema-derived block in `.issuectl/AGENTS.md` when
/// scan flagged drift. No-op if the file is absent (init is opt-in),
/// the block is already in sync, the file is malformed (refuse —
/// auto-collapse would destroy user content), or the schema/rules
/// failed to parse (would regenerate from defaults, overwriting real
/// policy). Doctor's run() already holds `mutate::WriteLock` for the
/// whole apply pass; this function does not re-acquire.
pub(crate) fn regenerate_agents_md(
    repo_root: &Path,
    actions: &DoctorActions,
    outcome: &mut ApplyOutcome,
) -> Result<()> {
    if !actions.regenerate_agents_md {
        return Ok(());
    }
    let path = agents::agents_path(repo_root);
    if !path.is_file() {
        return Ok(());
    }
    let original =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let schema = schema::load(repo_root)?;
    let rules = crate::transitions::load(repo_root)?;
    let new_text = agents::regenerate_managed(&original, &schema, &rules)?;
    if new_text != original {
        agents::atomic_write(&path, new_text.as_bytes())?;
        outcome.agents_md_regenerated = true;
    }
    Ok(())
}

/// Heuristic for "this is the pre-v0.5.0 `issues/AGENTS.md` scaffold,
/// not user-authored content." Any one of the markers is enough — they
/// all point at concepts (numbered layout, `open/`/`closed/` subdirs,
/// sequential numbering) that no current template would produce.
pub(crate) fn is_legacy_issues_agents(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "## Issue Numbering",
        "├── open/",
        "└── open/",
        "NN-short-title",
        "moved from `open/` to `closed/`",
    ];
    MARKERS.iter().any(|m| text.contains(m))
}

pub(crate) fn rewrite_legacy_issues_agents_md(
    repo_root: &Path,
    actions: &DoctorActions,
    outcome: &mut ApplyOutcome,
) -> Result<()> {
    if !actions.rewrite_issues_agents_md {
        return Ok(());
    }
    let path = repo_root.join("issues").join("AGENTS.md");
    if !path.is_file() {
        return Ok(());
    }
    fs::write(&path, crate::skill::ISSUES_AGENTS_TEMPLATE)
        .with_context(|| format!("cannot write {}", path.display()))?;
    outcome.issues_agents_md_rewritten = true;
    Ok(())
}

pub(crate) fn rename_notes_to_comments(
    repo_root: &Path,
    actions: &mut DoctorActions,
    outcome: &mut ApplyOutcome,
) -> Result<()> {
    let issues = repo_root.join("issues");
    // Surface scan-time `## Notes`/`## Comments` conflicts via the same
    // outcome field as TOCTOU-race skips. Manual merge is required; the
    // apply pipeline used to bail the whole pass on these (issue:
    // @doctor-fix-noop). Drain so a second call (post-flat-layout
    // rescan) only adds newly-discovered conflicts.
    for slug in std::mem::take(&mut actions.notes_conflicts) {
        if !outcome.notes_conflicts_at_apply.contains(&slug) {
            outcome.notes_conflicts_at_apply.push(slug);
        }
    }
    let planned = std::mem::take(&mut actions.notes_to_rename);
    for slug in planned {
        let item_path = issues.join(&slug).join("item.md");
        if !item_path.is_file() {
            continue;
        }
        let original = fs::read_to_string(&item_path)
            .with_context(|| format!("cannot read {}", item_path.display()))?;
        let (rewritten, has_conflict) = migrate_notes_heading(&original);
        if has_conflict {
            // Conflict surfaced during apply (file changed between
            // scan and apply — manual edit). Record it explicitly:
            // the post-apply re-scan will pick up the conflict only
            // if both headings are still present, but we need a
            // reliable signal even on the no-write path so JSON
            // consumers see that planned work was skipped.
            outcome.notes_conflicts_at_apply.push(slug);
            continue;
        }
        if rewritten != original {
            fs::write(&item_path, rewritten)
                .with_context(|| format!("cannot write {}", item_path.display()))?;
            outcome.notes_renamed.push(slug);
        }
    }
    Ok(())
}

/// Remove the retired lifecycle label without touching the identically named
/// intake status. Re-check the on-disk list through the canonical write helper
/// so a stale scan cannot remove any unrelated label.
pub(crate) fn apply_deferred_label_removal(
    actions: &mut DoctorActions,
    outcome: &mut ApplyOutcome,
) {
    for (slug, item_path) in std::mem::take(&mut actions.deferred_labels) {
        let result = (|| -> Result<bool> {
            if !item_path.is_file() {
                return Ok(false);
            }
            let mut item = write::read_item(&item_path)?;
            let labels_key = serde_yaml::Value::String("labels".into());
            let still_present = item
                .frontmatter
                .get(&labels_key)
                .and_then(|value| value.as_sequence())
                .is_some_and(|labels| {
                    labels
                        .iter()
                        .any(|label| label.as_str() == Some("deferred"))
                });
            if !still_present {
                return Ok(false);
            }
            write::remove_from_string_list(&mut item.frontmatter, "labels", "deferred")?;
            write::write_item(&item_path, &item)?;
            Ok(true)
        })();
        match result {
            Ok(true) => outcome.deferred_labels_removed.push(slug),
            Ok(false) => {}
            Err(error) => {
                outcome.apply_error = Some(format!(
                    "failed to remove retired `deferred` label from {slug}: {error:#}"
                ));
                return;
            }
        }
    }
}

pub(crate) fn apply_orphan_tempfiles(
    actions: &mut DoctorActions,
    outcome: &mut ApplyOutcome,
) -> Result<()> {
    let planned = std::mem::take(&mut actions.orphan_tempfiles);
    let mut removed = Vec::new();
    for path in planned {
        match fs::remove_file(&path) {
            Ok(_) => removed.push(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(anyhow::Error::from(e))
                    .with_context(|| format!("cannot remove {}", path.display()));
            }
        }
    }
    outcome.orphan_tempfiles_removed = removed;
    Ok(())
}

/// Best-effort closed-date for an issue being coerced/reconciled into a
/// closing status without an explicit `closed:` date. Stamping
/// `write::today()` is lossy for an issue that was actually closed long
/// ago, so we prefer, in order: the author date of the last git commit
/// that touched `item.md`, the file's mtime, then today(). All steps are
/// best-effort — any failure (not a git repo, untracked file, unreadable
/// metadata) falls through to the next source.
#[allow(dead_code)] // retained as the SystemClock convenience for inline tests.
pub(crate) fn derive_closed_date(item_path: &Path) -> String {
    derive_closed_date_via(item_path, &crate::clock::SystemClock)
}

/// Clock-injected variant of [`derive_closed_date`].
pub(crate) fn derive_closed_date_via(item_path: &Path, clock: &dyn crate::clock::Clock) -> String {
    git_last_commit_date(item_path)
        .or_else(|| file_mtime_date(item_path))
        .unwrap_or_else(|| clock.today_string())
}

/// Author date (`%aI`, strict ISO 8601) of the last commit that touched
/// `item_path`, converted to the machine's local timezone and projected
/// to `YYYY-MM-DD`. Converting to local — rather than slicing the raw
/// committer-TZ date — keeps this consistent with `write::today()` and
/// `file_mtime_date`, which both use local time, so the three fallback
/// tiers never disagree by a day. `--follow` lets it find the history of
/// a file that was renamed (e.g. an earlier flat-layout move that was
/// already committed). `None` when git is unavailable, the path is not
/// in a git repo, or the file is untracked (empty output).
pub(crate) fn git_last_commit_date(item_path: &Path) -> Option<String> {
    let dir = item_path.parent()?;
    let name = item_path.file_name()?;
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["log", "-1", "--follow", "--format=%aI", "--"])
        .arg(name)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Parsing the full RFC 3339 line validates it end-to-end (a
    // malformed `%aI` never lands in frontmatter) and carries the
    // offset, so the local-time conversion below is correct.
    let dt = chrono::DateTime::parse_from_rfc3339(stdout.trim()).ok()?;
    Some(
        dt.with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string(),
    )
}

/// File mtime of `item_path` projected to a local-time `YYYY-MM-DD`.
pub(crate) fn file_mtime_date(item_path: &Path) -> Option<String> {
    let modified = fs::metadata(item_path).ok()?.modified().ok()?;
    let datetime: chrono::DateTime<chrono::Local> = modified.into();
    Some(datetime.format("%Y-%m-%d").to_string())
}

pub(crate) fn apply_status_reconciliation(
    actions: &mut DoctorActions,
    outcome: &mut ApplyOutcome,
    clock: &dyn crate::clock::Clock,
) -> Result<()> {
    let active_to_closed = std::mem::take(&mut actions.closed_with_active_status);
    let closing_to_open = std::mem::take(&mut actions.open_with_closing_status);
    for (slug, _old_status, item_path) in active_to_closed {
        let mut item = write::read_item(&item_path)?;
        write::set_string(&mut item.frontmatter, "status", "done");
        let has_closed = item
            .frontmatter
            .get(serde_yaml::Value::String("closed".into()))
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if !has_closed {
            write::set_string(
                &mut item.frontmatter,
                "closed",
                &derive_closed_date_via(&item_path, clock),
            );
        }
        write::write_item(&item_path, &item)?;
        outcome.status_reconciled.push(slug);
    }
    for (slug, _old_status, item_path) in closing_to_open {
        let mut item = write::read_item(&item_path)?;
        write::set_string(&mut item.frontmatter, "status", "open");
        write::remove_key(&mut item.frontmatter, "closed");
        write::write_item(&item_path, &item)?;
        outcome.status_reconciled.push(slug);
    }
    Ok(())
}

/// Rewrite legacy `status` / `type` values to their canonical form via
/// the schema alias tables. Re-reads the on-disk value and only
/// rewrites when it still equals the recorded `from`. This guard covers
/// both an external concurrent edit between scan and apply AND an
/// earlier in-process apply step that already changed the field, so a
/// stale coercion never clobbers a fresher value. When a coerced
/// status lands in a closing lifecycle class and no `closed:` date is
/// present, a `closed:` date is stamped — mirroring the status command
/// so the migrated issue doesn't immediately trip the `closed:`
/// required-when rule.
pub(crate) fn apply_alias_coercions(
    actions: &mut DoctorActions,
    outcome: &mut ApplyOutcome,
    schema: &schema::Schema,
    clock: &dyn crate::clock::Clock,
) -> Result<()> {
    let planned = std::mem::take(&mut actions.alias_coercions);
    // Group consecutive coercions that share an `item_path` so an issue
    // carrying BOTH a status and a type coercion is read once and written
    // once instead of read+written per field. `planned` is sorted by scan
    // (slug first), so a given issue's entries are already adjacent; the
    // run-length accumulator below relies on that for single-read grouping
    // but stays correct (just less optimal) if the order ever changes, and
    // preserves planned order so `alias_coercions_applied` is deterministic.
    // `(slug, field, from, to)` — one applied/planned coercion sans path.
    type Coercion = (String, String, String, String);
    let mut groups: Vec<(PathBuf, Vec<Coercion>)> = Vec::new();
    for (slug, field, from, to, item_path) in planned {
        match groups.last_mut() {
            Some((p, v)) if *p == item_path => v.push((slug, field, from, to)),
            _ => groups.push((item_path, vec![(slug, field, from, to)])),
        }
    }

    for (item_path, coercions) in groups {
        if !item_path.is_file() {
            continue;
        }
        let mut item = write::read_item(&item_path)?;
        let mut applied: Vec<Coercion> = Vec::new();
        let mut coerced_to_closing = false;
        for (slug, field, from, to) in coercions {
            // Re-read the field from the in-memory mapping and only
            // rewrite when it still equals the recorded `from`. Guards
            // against a stale plan (external edit between scan and apply)
            // clobbering a fresher value.
            let current = item
                .frontmatter
                .get(serde_yaml::Value::String(field.clone()))
                .and_then(|v| v.as_str());
            if current != Some(from.as_str()) {
                continue;
            }
            write::set_string(&mut item.frontmatter, &field, &to);
            if field == "status" && schema::is_closing(schema, &to) {
                coerced_to_closing = true;
            }
            applied.push((slug, field, from, to));
        }
        if coerced_to_closing {
            let has_closed = item
                .frontmatter
                .get(serde_yaml::Value::String("closed".into()))
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if !has_closed {
                write::set_string(
                    &mut item.frontmatter,
                    "closed",
                    &derive_closed_date_via(&item_path, clock),
                );
            }
        }
        if !applied.is_empty() {
            write::write_item(&item_path, &item)?;
            outcome.alias_coercions_applied.extend(applied);
        }
    }
    Ok(())
}

/// Classification of a file's `## Notes` / `## Comments` shape.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NotesScan {
    /// File has neither heading, or only `## Comments` — nothing to do.
    NoOp,
    /// File has exactly one `## Notes` and no `## Comments`. Safe to
    /// rewrite to `## Comments`.
    SafeRename,
    /// File has exactly one `## Notes` AND exactly one `## Comments`.
    /// The two are auto-merged: `## Notes`' entries fold into
    /// `## Comments` (document order preserved) and `## Notes` is
    /// dropped (issue @doctor-fix-merge-notes-comments).
    Merge,
    /// File has more than one `## Notes` (with or without
    /// `## Comments`), OR one `## Notes` alongside multiple
    /// `## Comments`. The merge target is ambiguous (round-2 finding
    /// G5/O5), so we skip and surface the slug for manual merge.
    Conflict,
}

/// Classify a single item.md text. Uses the same fence-aware scanner
/// as the body_sections writer so both agree on what counts as a
/// real heading.
pub(crate) fn classify_notes(text: &str) -> NotesScan {
    let lines: Vec<&str> = text.split('\n').collect();
    let notes = body_sections_scan(&lines, "Notes");
    let comments = body_sections_scan(&lines, "Comments");
    if notes == 0 {
        NotesScan::NoOp
    } else if notes == 1 && comments == 0 {
        NotesScan::SafeRename
    } else if notes == 1 && comments == 1 {
        NotesScan::Merge
    } else {
        NotesScan::Conflict
    }
}

pub(crate) fn body_sections_scan(lines: &[&str], name: &str) -> usize {
    use crate::body_sections::{closes_fence, opening_fence, Fence};
    let mut fence: Option<Fence> = None;
    let mut count = 0usize;
    for l in lines {
        match fence {
            Some(open) if closes_fence(l, open) => fence = None,
            Some(_) => {}
            None => {
                if let Some(o) = opening_fence(l) {
                    fence = Some(o);
                } else if l.strip_prefix("## ").map(|r| r.trim_end()) == Some(name) {
                    count += 1;
                }
            }
        }
    }
    count
}

/// Pure function: migrate `## Notes` toward `## Comments`. When only
/// `## Notes` exists it's renamed; when both exist (exactly one of
/// each) `## Notes`' entries are folded into `## Comments` in document
/// order and `## Notes` is dropped. Fence-aware so a `## Notes` line
/// inside a code block is preserved verbatim. Returns
/// `(new_text, conflict)` — `conflict=true` only for the genuinely
/// ambiguous shapes (multiple `## Notes`, or a `## Notes` alongside
/// multiple `## Comments`) which the caller skips and surfaces.
pub(crate) fn migrate_notes_heading(text: &str) -> (String, bool) {
    use crate::body_sections::{closes_fence, opening_fence, Fence};
    match classify_notes(text) {
        NotesScan::NoOp => return (text.to_string(), false),
        NotesScan::Conflict => return (text.to_string(), true),
        NotesScan::Merge => {
            return (
                crate::body_sections::merge_h2_section(text, "Notes", "Comments"),
                false,
            )
        }
        NotesScan::SafeRename => {}
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut fence: Option<Fence> = None;
    for l in &lines {
        match fence {
            Some(open) if closes_fence(l, open) => {
                fence = None;
                out.push((*l).to_string());
                continue;
            }
            Some(_) => {
                out.push((*l).to_string());
                continue;
            }
            None => {
                if let Some(o) = opening_fence(l) {
                    fence = Some(o);
                    out.push((*l).to_string());
                    continue;
                }
            }
        }
        if l.strip_prefix("## ").map(|r| r.trim_end()) == Some("Notes") {
            out.push("## Comments".to_string());
        } else {
            out.push((*l).to_string());
        }
    }
    (out.join("\n"), false)
}

pub(crate) fn build_ambiguous(migrations: &[LegacyMigration]) -> BTreeSet<u32> {
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for m in migrations {
        *counts.entry(m.old_number).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .filter_map(|(n, c)| if c > 1 { Some(n) } else { None })
        .collect()
}

pub(crate) fn rewrite_item_frontmatter(
    item_path: &Path,
    new_slug: &str,
    number_to_slug: &BTreeMap<u32, String>,
    ambiguous_numbers: &BTreeSet<u32>,
) -> Result<()> {
    let mut item = write::read_item(item_path)?;

    // Drop legacy `number`, write `slug`.
    write::remove_key(&mut item.frontmatter, "number");
    write::set_string(&mut item.frontmatter, "slug", new_slug);

    // Migrate `epic: NN` (numeric) → `epic: <new_slug>` (string) when unambiguous.
    let epic_key = serde_yaml::Value::String("epic".into());
    if let Some(val) = item.frontmatter.get(&epic_key).cloned() {
        let migrated = match val {
            serde_yaml::Value::Number(n) => n
                .as_u64()
                .and_then(|u| u32::try_from(u).ok())
                .filter(|n| !ambiguous_numbers.contains(n))
                .and_then(|n| number_to_slug.get(&n).cloned()),
            serde_yaml::Value::String(s) => {
                let bare = s.strip_prefix('@').unwrap_or(&s).to_string();
                if let Ok(n) = bare.parse::<u32>() {
                    if !ambiguous_numbers.contains(&n) {
                        number_to_slug.get(&n).cloned()
                    } else {
                        None
                    }
                } else {
                    Some(bare)
                }
            }
            _ => None,
        };
        if let Some(s) = migrated {
            write::set_string(&mut item.frontmatter, "epic", &s);
        }
    }

    // Migrate `related` / `blocked_by`: ["#NN", ...] → ["@<slug>", ...]
    // when unambiguous.
    for key in ["related", "blocked_by"] {
        let yaml_key = serde_yaml::Value::String(key.into());
        if let Some(serde_yaml::Value::Sequence(seq)) = item.frontmatter.get(&yaml_key).cloned() {
            let mut new_seq: Vec<serde_yaml::Value> = Vec::with_capacity(seq.len());
            for v in seq {
                let migrated = match v {
                    serde_yaml::Value::String(ref s) => {
                        if let Some(rest) = s.strip_prefix('#') {
                            if let Ok(n) = rest.parse::<u32>() {
                                if !ambiguous_numbers.contains(&n) {
                                    number_to_slug
                                        .get(&n)
                                        .map(|sl| format!("@{sl}"))
                                        .unwrap_or_else(|| s.clone())
                                } else {
                                    s.clone()
                                }
                            } else {
                                s.clone()
                            }
                        } else if s.starts_with('@') {
                            s.clone()
                        } else {
                            format!("@{s}")
                        }
                    }
                    _ => continue,
                };
                new_seq.push(serde_yaml::Value::String(migrated));
            }
            item.frontmatter
                .insert(yaml_key, serde_yaml::Value::Sequence(new_seq));
        }
    }

    write::write_item(item_path, &item)?;
    Ok(())
}

pub(crate) fn rewrite_markdown_in_scopes(
    scopes: &[PathBuf],
    number_to_slug: &BTreeMap<u32, String>,
    dir_to_slug: &BTreeMap<String, String>,
    ambiguous_numbers: &BTreeSet<u32>,
) -> Result<usize> {
    let mut changed = 0usize;
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    for scope in scopes {
        if !scope.exists() {
            continue;
        }
        let files = if scope.is_file() {
            vec![scope.clone()]
        } else {
            collect_markdown_files(scope)?
        };
        for path in files {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !seen.insert(canonical) {
                continue;
            }
            let original = fs::read_to_string(&path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            let rewritten = rewrite_text(&original, number_to_slug, dir_to_slug, ambiguous_numbers);
            if rewritten != original {
                fs::write(&path, rewritten)
                    .with_context(|| format!("cannot write {}", path.display()))?;
                changed += 1;
            }
        }
    }
    Ok(changed)
}

pub(crate) fn collect_markdown_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk_md(root, &mut out)?;
    out.sort();
    Ok(out)
}

pub(crate) fn walk_md(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type()?.is_dir() {
            if matches!(
                name.as_str(),
                ".git" | "target" | "node_modules" | ".cargo" | "dist" | "build"
            ) {
                continue;
            }
            walk_md(&path, out)?;
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("md"))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
    Ok(())
}

pub(crate) fn rewrite_text(
    text: &str,
    number_to_slug: &BTreeMap<u32, String>,
    dir_to_slug: &BTreeMap<String, String>,
    ambiguous_numbers: &BTreeSet<u32>,
) -> String {
    // 1. Markdown legacy heading `# E10. Title` → `# Title`.
    let heading_re = Regex::new(r"^(# )E?\d+\.\s+(.+)$").expect("valid heading");
    // 2. `#NN` body refs → `@<slug>` (best-effort, skip ambiguous).
    let ref_re = Regex::new(r"#(\d+)\b").expect("valid ref");
    // 3. Path components: `issues/{open,closed}/<NN>-<slug>/` → `issues/.../<new>/`.
    let dir_regexes: Vec<(Regex, String)> = dir_to_slug
        .iter()
        .map(|(old, new)| {
            let pat = format!(
                r"(^|[^A-Za-z0-9_-]){}($|[^A-Za-z0-9_-])",
                regex::escape(old)
            );
            (Regex::new(&pat).expect("valid dir regex"), new.clone())
        })
        .collect();
    // Skip-region awareness (fenced code blocks, inline code spans,
    // and link URLs) is delegated to the shared
    // `body_sections::rewrite_outside_code_and_urls` walker that
    // `refs::rewrite_body_refs` also uses — keeping the two callers
    // from drifting on which markdown constructs are off-limits.
    crate::body_sections::rewrite_outside_code_and_urls(
        text,
        crate::body_sections::RewriteSkips::code_only(),
        |seg| {
            // heading_re is line-anchored, but a prose segment that
            // starts at a line beginning (the common case for legacy
            // `# E10. Title` headings — which never contain inline code
            // or link URLs in the heading number/dot) still matches the
            // pattern. If the segment doesn't begin a line, `^` simply
            // doesn't fire and the segment passes through.
            let seg = heading_re.replace(seg, "$1$2");
            let seg = ref_re.replace_all(&seg, |caps: &Captures| {
                let n: u32 = match caps[1].parse() {
                    Ok(v) => v,
                    Err(_) => return caps[0].to_string(),
                };
                if ambiguous_numbers.contains(&n) {
                    return caps[0].to_string();
                }
                match number_to_slug.get(&n) {
                    Some(s) => format!("@{s}"),
                    None => caps[0].to_string(),
                }
            });
            let mut s = seg.into_owned();
            for (re, new) in &dir_regexes {
                s = re
                    .replace_all(&s, |caps: &Captures| {
                        format!("{}{}{}", &caps[1], new, &caps[2])
                    })
                    .to_string();
            }
            s
        },
    )
}

// ── Output rendering ────────────────────────────────────────────────────────

pub(crate) fn planned_moves(report: &DoctorFindings) -> &[PlannedMove] {
    report
        .flat_layout_plan
        .as_ref()
        .map(|p| p.moves())
        .unwrap_or(&[])
}

/// Threshold above which long warning lists collapse to a one-line
/// count in default rendering. `--verbose` always prints the full
/// list. The number itself is a UX dial — small enough that an
/// "almost-clean" repo still shows individual entries, large enough
/// that a real-world legacy repo's 100+ entries collapse cleanly.
pub(crate) const RENDER_FULL_LIST_LIMIT: usize = 10;

/// Render a list section to `out`, collapsing to a one-liner when
/// not `verbose` and the list exceeds `RENDER_FULL_LIST_LIMIT`
/// entries. Caller passes the `verb_phrase` used in the collapsed
/// line (e.g. "need layout migration"). Empty lists render nothing.
/// Writing through `&mut dyn fmt::Write` keeps the helper testable
/// against an in-memory buffer (issue:
/// `@ridiculously-outrageous-fold`).
pub(crate) fn render_section<T>(
    out: &mut dyn fmt::Write,
    title: &str,
    items: &[T],
    verbose: bool,
    verb_phrase: &str,
    fmt_item: impl Fn(&T) -> String,
) {
    if items.is_empty() {
        return;
    }
    if !verbose && items.len() > RENDER_FULL_LIST_LIMIT {
        let _ = writeln!(
            out,
            "{} {} (re-run with --verbose to list).",
            items.len(),
            verb_phrase
        );
        let _ = writeln!(out);
        return;
    }
    let _ = writeln!(out, "{title}");
    for it in items {
        let _ = writeln!(out, "  {}", fmt_item(it));
    }
    let _ = writeln!(out);
}

/// `render_section` adapter that prints to stdout.
pub(crate) fn print_section<T>(
    title: &str,
    items: &[T],
    verbose: bool,
    verb_phrase: &str,
    fmt_item: impl Fn(&T) -> String,
) {
    let mut buf = String::new();
    render_section(&mut buf, title, items, verbose, verb_phrase, fmt_item);
    print!("{buf}");
}

/// Count the individual unresolved findings rendered by a completed
/// `doctor --fix` pass. This deliberately counts entries, rather than
/// `critical_blockers()`' grouped diagnostic categories: one category can
/// contain many issues, and warning-only findings are still printed beside
/// critical ones for the user to resolve.
pub(crate) fn remaining_finding_count(report: &DoctorFindings) -> usize {
    report.legacy_dirs.len()
        + planned_moves(report).len()
        + report.flat_layout_conflicts.len()
        + report.invalid_slugs.len()
        + report.duplicate_slugs.len()
        + report.missing_item_md.len()
        + report.orphan_epic_refs.len()
        + report.parse_errors.len()
        + report.notes_to_rename.len()
        + report.notes_conflicts.len()
        + report.schema_violations.len()
        + report.alias_coercions.len()
        + usize::from(report.schema_parse_error.is_some())
        + report.broken_refs.len()
        + report.blocked_by_cycles.len()
        + report.blocked_by_self.len()
        + report.status_consistency.len()
        + report.timestamp_issues.len()
        + report.unknown_keys.len()
        + report.unknown_reviewers.len()
        + report.conflict_markers.len()
        + report.orphan_tempfiles.len()
        + report.symlinked_dirs.len()
        + report.both_open_and_closed.len()
        + report.closed_with_active_status.len()
        + report.open_with_closing_status.len()
        + report.transition_warnings.len()
        + report.missing_body_sections.len()
        + usize::from(report.agents_md_drift)
        + usize::from(report.agents_md_malformed.is_some())
        + usize::from(report.agents_md_check_skipped.is_some())
        + usize::from(report.agents_md_missing)
        + report.gitignored_paths.len()
        + usize::from(report.legacy_issues_agents_md)
        + report.large_binaries.len()
        + report.non_avif_images.len()
        + report.broken_attachment_refs.len()
        + report.deferred_labels.len()
}

pub(crate) fn fix_summary(report: &DoctorFindings, oc: &ApplyOutcome) -> String {
    let counts = format!(
        "{} legacy dir(s) migrated, {} flat-layout dir(s) migrated, {} markdown file(s) rewritten, {} `## Notes` rename(s), {} retired label(s) removed, {} AGENTS.md block(s) regenerated.",
        oc.legacy_dirs_migrated.len(),
        oc.flat_layout_migrated.len(),
        oc.files_rewritten,
        oc.notes_renamed.len(),
        oc.deferred_labels_removed.len(),
        if oc.agents_md_regenerated { 1 } else { 0 }
    );
    match (oc.stop_phase, oc.apply_error.is_some()) {
        (StopPhase::Preflight, _) => format!(
            "Refused — {} preflight blocker(s); no writes applied.",
            oc.blockers.len()
        ),
        (_, true) => format!("Aborted mid-pipeline. {counts}"),
        (StopPhase::PostApply, _) => format!(
            "Partial — {} post-apply blocker(s); partial writes retained. {counts}",
            oc.blockers.len()
        ),
        (StopPhase::Ok, _) if !oc.notes_conflicts_at_apply.is_empty() => format!(
            "Partial — auto-fixes ran where possible. {} issue(s) need manual attention (see above). {counts}",
            oc.notes_conflicts_at_apply.len()
        ),
        (StopPhase::Ok, _) => {
            if critical_blockers(report).is_empty() {
                format!("Applied. {counts}")
            } else {
                format!(
                    "Partial — {} unfixable finding(s) remain (see above). {counts}",
                    remaining_finding_count(report)
                )
            }
        }
    }
}
