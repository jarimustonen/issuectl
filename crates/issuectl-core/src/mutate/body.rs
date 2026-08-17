use super::*;

/// PUT-style replacement of an issue's body markdown. Same lock and
/// optimistic-concurrency contract as `update_issue`, but only the body
/// (and `updated:`) change. Status/folder are untouched, so this never
/// causes a directory rename.
pub fn update_body(
    root: &Path,
    slug: &str,
    expected_version: Option<String>,
    body: String,
    dry_run: bool,
) -> Result<UpdateOutcome, MutateError> {
    update_body_via(root, slug, expected_version, body, dry_run, &SystemClock)
}

/// Clock-injected variant of [`update_body`].
pub fn update_body_via(
    root: &Path,
    slug: &str,
    expected_version: Option<String>,
    body: String,
    dry_run: bool,
    clock: &dyn Clock,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    // Locate read-only regardless of dry_run so the legacy → flat
    // migration and `.schema.yaml` bootstrap fire only after every
    // validation step has passed (parity with `update_issue`).
    let item_path = locate_for_dry_run(root, slug)?;
    let schema =
        crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;

    let parsed = crate::parser::parse_item_md_with_warnings(&item_path, slug, "open");
    if !parsed.warnings.is_empty() {
        return Err(MutateError::Corrupt {
            warnings: parsed.warnings,
        });
    }
    let mut prev_issue = parsed.issue;
    prev_issue.folder = folder_for_status(&schema, &prev_issue.status).to_string();
    let current_version = canonical_hash(&prev_issue);

    if let Some(ref expected) = expected_version {
        if expected != &current_version {
            return Err(MutateError::VersionMismatch {
                current: prev_issue,
                version: current_version,
            });
        }
    }

    // Authoring-time advisory: warn when the replacement body carries a
    // reserved-legacy section heading (`## Notes`) so the collision
    // surfaces now rather than at commit time via the doctor pre-commit
    // hook. Non-fatal — the write still proceeds (the author may be
    // migrating). Computed before `body` is moved into `item.body`.
    let warnings = crate::body_sections::reserved_section_warnings(&body);

    let mut item = write::read_item(&item_path).map_err(MutateError::Io)?;
    let before_serialized =
        if dry_run {
            // Raw on-disk bytes — not `serialize_item(&item)`. The
            // canonicalised re-serialization would mask formatting
            // changes (dropped YAML comments, key reordering, scalar-
            // style shifts) that the real write would also apply,
            // making the dry-run preview lie about disk impact (round-2 #2).
            Some(fs::read_to_string(&item_path).map_err(|e| {
                MutateError::Io(anyhow!("cannot read {}: {e}", item_path.display()))
            })?)
        } else {
            None
        };
    // Clients send a plain markdown body. Preserve the read_item
    // convention of one leading newline so the on-disk layout stays
    // `---\n<fm>\n---\n\n<body>` rather than `---<body>` — without
    // this, every web save would collapse the blank separator line
    // and parse_item still works but readers see a slightly different
    // file each round-trip.
    item.body = if body.starts_with('\n') {
        body
    } else {
        format!("\n{body}")
    };
    write::set_string(&mut item.frontmatter, "updated", &clock.today_string());

    // Schema validation: body-set doesn't change frontmatter shape but
    // the schema may have tightened since the last write. Refusing here
    // matches the `update_issue` contract.
    let violations = crate::schema::validate(&schema, &item.frontmatter);
    // Body-replace never writes status/closed, so an empty `written`
    // set keeps the lenient RequiredWhen handling.
    if let Some(msg) = hard_schema_failure(&violations, &std::collections::BTreeSet::new()) {
        return Err(MutateError::SchemaViolation(msg));
    }

    // Transition rules apply on the body-replace path too. Without
    // this, a client that PATCHed status=done with checked AC could
    // `update_body` afterwards to wipe / uncheck them, leaving the
    // issue in a state that violates the rule it just satisfied.
    // Status doesn't change here, so only `requires_*` checks matter
    // (graph rules are skipped by the prev==new guard).
    let rules = load_validated_rules(root, &schema)?;
    let projected = projected_issue_for_rules(slug, &item, &item_path, &schema)?;
    let rule_violations =
        crate::transitions::evaluate_transition(&rules, &projected, &prev_issue.status);
    if !rule_violations.is_empty() {
        return Err(MutateError::TransitionViolation(rule_violations.join("; ")));
    }

    if dry_run {
        let pending = write::serialize_item(&item).map_err(MutateError::Io)?;
        let new_issue = parse_serialized(&pending, slug, &schema);
        let new_version = canonical_hash(&new_issue);
        // Body verbs never change status, so an archived issue stays
        // archived — predict its current dir, not the active root.
        let post_closing = crate::schema::is_closing(&schema, &new_issue.status);
        return Ok(UpdateOutcome {
            issue: new_issue,
            version: new_version,
            issue_dir: predicted_issue_dir(root, slug, &item_path, post_closing),
            moved_to_closed: false,
            moved_to_open: false,
            pending_serialized: Some(pending),
            before_serialized,
            warnings,
        });
    }

    // Side effects deferred from the top of the function so a failed
    // validation above leaves no `.schema.yaml` bootstrap and no
    // legacy → flat migration on disk.
    crate::schema::ensure_default_written(root).map_err(MutateError::Io)?;
    let final_path = migrate_to_flat_if_legacy(root, slug, &item_path)?;
    write_item_atomic(&final_path, &item).map_err(MutateError::Io)?;

    let after = crate::parser::parse_item_md_with_warnings(&final_path, slug, "open");
    let mut new_issue = after.issue;
    new_issue.folder = folder_for_status(&schema, &new_issue.status).to_string();
    let new_version = canonical_hash(&new_issue);

    Ok(UpdateOutcome {
        issue: new_issue,
        version: new_version,
        issue_dir: final_path
            .parent()
            .expect("item.md has a parent")
            .to_path_buf(),
        moved_to_closed: false,
        moved_to_open: false,
        pending_serialized: None,
        before_serialized: None,
        warnings,
    })
}

/// Append a timestamped block to the issue's `## Comments` section
/// (creating it if missing). Same flock + optimistic-version contract
/// as `update_issue`. Body-only mutation: `status`, `closed`, etc.
/// are untouched, so this never causes a status transition or
/// directory rename.
pub fn note_issue(
    root: &Path,
    slug: &str,
    author: &str,
    message: &str,
    section: &str,
    expected_version: Option<String>,
    dry_run: bool,
) -> Result<UpdateOutcome, MutateError> {
    note_issue_via(
        root,
        slug,
        author,
        message,
        section,
        expected_version,
        dry_run,
        &SystemClock,
    )
}

/// Clock-injected variant of [`note_issue`].
#[allow(clippy::too_many_arguments)]
pub fn note_issue_via(
    root: &Path,
    slug: &str,
    author: &str,
    message: &str,
    section: &str,
    expected_version: Option<String>,
    dry_run: bool,
    clock: &dyn Clock,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }
    // Strip a single leading `@` and validate through the shared author
    // seam, then attribute the note to the normalized token so
    // `note --as "@alice"` records `alice` (matching the `@alice` we render).
    let author = crate::body_sections::normalize_author(author)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let author = author.as_str();
    crate::body_sections::validate_message(message)
        .map_err(|e| MutateError::Validation(e.to_string()))?;

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    // Locate read-only regardless of `dry_run`. Migration / schema
    // bootstrap deferred to just before atomic write so that any
    // validation failure below leaves no repo side effects.
    let item_path = locate_for_dry_run(root, slug)?;
    let schema =
        crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;

    let parsed = crate::parser::parse_item_md_with_warnings(&item_path, slug, "open");
    if !parsed.warnings.is_empty() {
        return Err(MutateError::Corrupt {
            warnings: parsed.warnings,
        });
    }
    let mut prev_issue = parsed.issue;
    prev_issue.folder = folder_for_status(&schema, &prev_issue.status).to_string();
    let current_version = canonical_hash(&prev_issue);
    if let Some(ref expected) = expected_version {
        if expected != &current_version {
            return Err(MutateError::VersionMismatch {
                current: prev_issue,
                version: current_version,
            });
        }
    }

    let mut item = write::read_item(&item_path).map_err(MutateError::Io)?;
    let before_serialized =
        if dry_run {
            // Raw on-disk bytes — not `serialize_item(&item)`. The
            // canonicalised re-serialization would mask formatting
            // changes (dropped YAML comments, key reordering, scalar-
            // style shifts) that the real write would also apply,
            // making the dry-run preview lie about disk impact (round-2 #2).
            Some(fs::read_to_string(&item_path).map_err(|e| {
                MutateError::Io(anyhow!("cannot read {}: {e}", item_path.display()))
            })?)
        } else {
            None
        };
    let block = crate::body_sections::render_note_block(
        &crate::body_sections::now_iso_via(clock),
        author,
        message,
    )
    .map_err(|e| MutateError::Validation(e.to_string()))?;
    let trimmed_body = item.body.trim_start_matches('\n');
    let appended = crate::body_sections::append_block(trimmed_body, section, &block);
    // Canonicalise leading-newline shape so `serialize_item` always
    // produces `---\n\n<body>` rather than leaving a legacy
    // no-blank-line file in a state `fmt` would still want to change.
    item.body = crate::body_sections::canonicalise_body_leading(&appended);
    write::set_string(&mut item.frontmatter, "updated", &clock.today_string());

    // Schema validation runs on every write surface for parity with
    // `update_body` / `update_issue` — without this, a tightened
    // schema could block `body set` while letting `note` keep
    // mutating the same invalid issue (review finding #6).
    validate_against_schema(root, &item.frontmatter)?;

    // Transition rules also evaluated for parity with `update_body`,
    // BUT — by design — violations are surfaced as warnings rather
    // than hard errors. `cmd_note` is a body-only verb agents reach
    // for to record intent (decisions, agent runs, comments); blocking
    // the write would force them to back out and replay through the
    // unified `apply` envelope just to log "I noticed AC#2 is
    // unticked." We let the write through, leave the issue in a
    // (potentially) rule-violating state, and tell the caller. The
    // unified PATCH path (`update_issue_under_lock`) keeps the strict
    // rejection — body_ops there compose with frontmatter mutations,
    // so the caller has the tools to fix the violation in the same
    // transaction.
    let warnings = transition_warnings(root, slug, &item, &item_path, &prev_issue.status);

    if dry_run {
        let pending = write::serialize_item(&item).map_err(MutateError::Io)?;
        let new_issue = parse_serialized(&pending, slug, &schema);
        let new_version = canonical_hash(&new_issue);
        // Body verbs never change status, so an archived issue stays
        // archived — predict its current dir, not the active root.
        let post_closing = crate::schema::is_closing(&schema, &new_issue.status);
        return Ok(UpdateOutcome {
            issue: new_issue,
            version: new_version,
            issue_dir: predicted_issue_dir(root, slug, &item_path, post_closing),
            moved_to_closed: false,
            moved_to_open: false,
            pending_serialized: Some(pending),
            before_serialized,
            warnings,
        });
    }

    crate::schema::ensure_default_written(root).map_err(MutateError::Io)?;
    let final_path = migrate_to_flat_if_legacy(root, slug, &item_path)?;
    write_item_atomic(&final_path, &item).map_err(MutateError::Io)?;

    let after = crate::parser::parse_item_md_with_warnings(&final_path, slug, "open");
    let mut new_issue = after.issue;
    new_issue.folder = folder_for_status(&schema, &new_issue.status).to_string();
    let new_version = canonical_hash(&new_issue);

    Ok(UpdateOutcome {
        issue: new_issue,
        version: new_version,
        issue_dir: final_path
            .parent()
            .expect("item.md has a parent")
            .to_path_buf(),
        moved_to_closed: false,
        moved_to_open: false,
        pending_serialized: None,
        before_serialized: None,
        warnings,
    })
}

/// Toggle a markdown checklist item in the issue body. Matches the
/// first body line containing `substring` whose stripped text starts
/// with `- [ ]` or `- [x]` (case-insensitive on the cross marker), and
/// flips its checkbox in place. Errors when zero or multiple lines
/// match. Same flock + optimistic-version contract as `update_body`.
pub fn toggle_checkbox(
    root: &Path,
    slug: &str,
    substring: &str,
    expected_version: Option<String>,
    dry_run: bool,
) -> Result<UpdateOutcome, MutateError> {
    toggle_checkbox_via(
        root,
        slug,
        substring,
        expected_version,
        dry_run,
        &SystemClock,
    )
}

/// Clock-injected variant of [`toggle_checkbox`].
pub fn toggle_checkbox_via(
    root: &Path,
    slug: &str,
    substring: &str,
    expected_version: Option<String>,
    dry_run: bool,
    clock: &dyn Clock,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }
    if substring.trim().is_empty() {
        return Err(MutateError::Validation(
            "task substring cannot be empty".into(),
        ));
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    // Locate read-only regardless of `dry_run`. Migration / schema
    // bootstrap deferred to just before atomic write.
    let item_path = locate_for_dry_run(root, slug)?;
    let schema =
        crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let parsed = crate::parser::parse_item_md_with_warnings(&item_path, slug, "open");
    if !parsed.warnings.is_empty() {
        return Err(MutateError::Corrupt {
            warnings: parsed.warnings,
        });
    }
    let mut prev_issue = parsed.issue;
    prev_issue.folder = folder_for_status(&schema, &prev_issue.status).to_string();
    let current_version = canonical_hash(&prev_issue);
    if let Some(ref expected) = expected_version {
        if expected != &current_version {
            return Err(MutateError::VersionMismatch {
                current: prev_issue,
                version: current_version,
            });
        }
    }

    let mut item = write::read_item(&item_path).map_err(MutateError::Io)?;
    let before_serialized =
        if dry_run {
            // Raw on-disk bytes — not `serialize_item(&item)`. The
            // canonicalised re-serialization would mask formatting
            // changes (dropped YAML comments, key reordering, scalar-
            // style shifts) that the real write would also apply,
            // making the dry-run preview lie about disk impact (round-2 #2).
            Some(fs::read_to_string(&item_path).map_err(|e| {
                MutateError::Io(anyhow!("cannot read {}: {e}", item_path.display()))
            })?)
        } else {
            None
        };
    let new_body = toggle_checkbox_in_body(&item.body, substring)?;
    item.body = new_body;
    write::set_string(&mut item.frontmatter, "updated", &clock.today_string());

    validate_against_schema(root, &item.frontmatter)?;

    // Transition rules: surface as warnings on this body-only verb,
    // matching `note_issue`. Toggling a checkbox cannot legitimately
    // be blocked by a transition rule (the verb doesn't change
    // status), but a `requires_*` rule that pins acceptance criteria
    // for an already-closing issue WILL fire here — and the user
    // probably wants to know without being blocked from making the
    // edit. The unified `body_ops` PATCH path keeps the strict
    // rejection.
    let warnings = transition_warnings(root, slug, &item, &item_path, &prev_issue.status);

    if dry_run {
        let pending = write::serialize_item(&item).map_err(MutateError::Io)?;
        let new_issue = parse_serialized(&pending, slug, &schema);
        let new_version = canonical_hash(&new_issue);
        // Body verbs never change status, so an archived issue stays
        // archived — predict its current dir, not the active root.
        let post_closing = crate::schema::is_closing(&schema, &new_issue.status);
        return Ok(UpdateOutcome {
            issue: new_issue,
            version: new_version,
            issue_dir: predicted_issue_dir(root, slug, &item_path, post_closing),
            moved_to_closed: false,
            moved_to_open: false,
            pending_serialized: Some(pending),
            before_serialized,
            warnings,
        });
    }

    crate::schema::ensure_default_written(root).map_err(MutateError::Io)?;
    let final_path = migrate_to_flat_if_legacy(root, slug, &item_path)?;
    write_item_atomic(&final_path, &item).map_err(MutateError::Io)?;
    let after = crate::parser::parse_item_md_with_warnings(&final_path, slug, "open");
    let mut new_issue = after.issue;
    new_issue.folder = folder_for_status(&schema, &new_issue.status).to_string();
    let new_version = canonical_hash(&new_issue);

    Ok(UpdateOutcome {
        issue: new_issue,
        version: new_version,
        issue_dir: final_path
            .parent()
            .expect("item.md has a parent")
            .to_path_buf(),
        moved_to_closed: false,
        moved_to_open: false,
        pending_serialized: None,
        before_serialized: None,
        warnings,
    })
}

/// Evaluate transition rules against the post-mutation projection and
/// return any violations as warning strings. Used by body-only verbs
/// (`note_issue`, `toggle_checkbox`) that surface rule mismatches as
/// warnings rather than hard errors. `prev_status` is the on-disk
/// status before the mutation; for body-only verbs that's the same as
/// the post-mutation status, so only `requires_*` rules can fire.
///
/// On schema / transitions config load failure we *surface* the error
/// as a warning rather than swallow it. The body verbs predate the
/// rules engine and shouldn't refuse the write because the operator
/// broke `transitions.yaml`, but they also shouldn't go silent on it —
/// without a warning, agents iterating with `note` / `check` against a
/// broken config would never know the rules engine is dead, which is a
/// trust violation. The unified PATCH path keeps the strict
/// `MutateError::TransitionConfig` rejection.
pub(crate) fn transition_warnings(
    root: &Path,
    slug: &str,
    item: &write::ItemFile,
    item_path: &Path,
    prev_status: &str,
) -> Vec<String> {
    let schema = match crate::schema::load(root) {
        Ok(s) => s,
        Err(e) => {
            return vec![format!(
                "rules engine: schema load failed, transition checks skipped: {e:#}"
            )]
        }
    };
    let rules = match crate::transitions::load(root) {
        Ok(r) => r,
        Err(e) => {
            return vec![format!(
                "rules engine: transitions config load failed, transition checks skipped: {e:#}"
            )]
        }
    };
    let universe = crate::schema::status_universe(&schema);
    if let Err(e) = crate::transitions::validate_status_refs(&rules, &universe) {
        return vec![format!(
            "rules engine: transitions reference unknown statuses, transition checks skipped: {e:#}"
        )];
    }
    let projected = match projected_issue_for_rules(slug, item, item_path, &schema) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    crate::transitions::evaluate_transition(&rules, &projected, prev_status)
}

/// Apply a single `BodyOp` against the in-flight `ItemFile`. Shared by
/// the `body_ops` vector in `UpdateIssueRequest` so the same primitives
/// `cmd_note` / `cmd_check` use also drive the transactional `apply`
/// path — keeping the rendering, fence handling, and error messages
/// identical across surfaces.
pub(crate) fn apply_body_op(
    item: &mut ItemFile,
    index: usize,
    op: &BodyOp,
    clock: &dyn Clock,
) -> Result<(), MutateError> {
    match op {
        BodyOp::SetCheckbox(set) => {
            let new_body = set_checkbox_in_body(&item.body, &set.match_substring, set.checked)
                .map_err(|e| prefix_body_op_error(index, e))?;
            item.body = new_body;
        }
        BodyOp::AppendNote(note) => {
            // `validate()` already checked author/message, but
            // `render_note_block` re-validates defensively. Mirror the
            // same `body_ops[i].<field>:` shape here so the under-lock
            // error path matches the pre-lock validation path
            // byte-for-byte (LLM review consensus #5).
            crate::body_sections::validate_author(&note.author)
                .map_err(|e| MutateError::Validation(format!("body_ops[{index}].author: {e}")))?;
            crate::body_sections::validate_message(&note.message)
                .map_err(|e| MutateError::Validation(format!("body_ops[{index}].message: {e}")))?;
            let block = crate::body_sections::render_note_block(
                &crate::body_sections::now_iso_via(clock),
                &note.author,
                &note.message,
            )
            .map_err(|e| MutateError::Validation(format!("body_ops[{index}]: {e}")))?;
            let trimmed = item.body.trim_start_matches('\n');
            let appended =
                crate::body_sections::append_block(trimmed, note.section.as_str(), &block);
            item.body = crate::body_sections::canonicalise_body_leading(&appended);
        }
    }
    Ok(())
}

/// Attach `body_ops[{index}]:` context to *every* error variant a body
/// op might surface. The previous `match` only wrapped `Validation`
/// and let other variants pass through unprefixed — dead today (the
/// body primitives only return `Validation`), but a footgun the
/// moment one of them grows an Io / SchemaViolation path.
pub(crate) fn prefix_body_op_error(index: usize, err: MutateError) -> MutateError {
    match err {
        MutateError::Validation(s) => MutateError::Validation(format!("body_ops[{index}]: {s}")),
        MutateError::SchemaViolation(s) => {
            MutateError::SchemaViolation(format!("body_ops[{index}]: {s}"))
        }
        MutateError::TransitionViolation(s) => {
            MutateError::TransitionViolation(format!("body_ops[{index}]: {s}"))
        }
        MutateError::ConflictingIntent(s) => {
            MutateError::ConflictingIntent(format!("body_ops[{index}]: {s}"))
        }
        MutateError::Io(e) => {
            // `e.context(...)` preserves the anyhow `source()` chain so
            // downstream `{e:#}` rendering and `e.chain()` walking still
            // work; the previous `format!("{e:#}")` flattened the chain
            // into the inner message and threw away the source links.
            MutateError::Io(e.context(format!("body_ops[{index}]")))
        }
        // Variants we deliberately pass through unchanged. `NotFound`,
        // `VersionMismatch`, and `AmbiguousSlug` describe whole-document
        // state from before the body-op loop — the index doesn't help.
        // `Corrupt { warnings: Vec<String> }` does carry a payload, but
        // it's parser warnings about the on-disk file, not a single
        // body-op error; splicing the index in would mislead. Operator-
        // facing config errors (`SchemaConfig`, `TransitionConfig`) are
        // about the repo's configuration, not the request.
        other => other,
    }
}

/// Drive the unique checkbox line containing `substring` to the target
/// `checked` state. Idempotent: if the matched line is already in the
/// target state, returns the body unchanged so retried agent requests
/// don't flip the box back and forth. Errors when zero or multiple
/// lines match — same shape as `toggle_checkbox_in_body`.
pub(crate) fn set_checkbox_in_body(
    body: &str,
    substring: &str,
    checked: bool,
) -> Result<String, MutateError> {
    let lines: Vec<&str> = body.split('\n').collect();
    let mut matches: Vec<(usize, bool)> = Vec::new();
    crate::body_sections::for_each_line_outside_fences(body, |i, line| {
        if let Some(state) = checkbox_state(line) {
            if line.contains(substring) {
                matches.push((i, state));
            }
        }
    });
    match matches.as_slice() {
        [] => Err(MutateError::Validation(format!(
            "no checkbox line matched {substring:?}"
        ))),
        [(idx, current_state)] => {
            if *current_state == checked {
                return Ok(body.to_string());
            }
            let new_line = set_line_checkbox(lines[*idx], checked).ok_or_else(|| {
                MutateError::Validation(format!(
                    "internal: matched line {:?} is not a checkbox after match",
                    lines[*idx]
                ))
            })?;
            let mut out = Vec::with_capacity(lines.len());
            for (i, l) in lines.iter().enumerate() {
                if i == *idx {
                    out.push(new_line.clone());
                } else {
                    out.push((*l).to_string());
                }
            }
            Ok(out.join("\n"))
        }
        many => Err(MutateError::Validation(format!(
            "{} checkbox lines matched {substring:?}; refine to a unique substring",
            many.len()
        ))),
    }
}

/// Find a unique checkbox line containing `substring` and return the
/// body with that one line's `[ ]` / `[x]` toggled. Fence-aware:
/// checkbox lines inside fenced code blocks are skipped so example
/// task lists in documentation snippets don't get silently mutated.
/// The checkbox shape matched is `^\s*[-*+]\s+\[[ xX]\]\s` so common
/// GFM variants work, while non-checkbox brackets like `- [n]` are
/// rejected.
pub(crate) fn toggle_checkbox_in_body(body: &str, substring: &str) -> Result<String, MutateError> {
    let lines: Vec<&str> = body.split('\n').collect();
    let mut matches: Vec<usize> = Vec::new();
    // Fence-aware enumeration so `- [ ]` examples inside ```fenced```
    // code blocks aren't toggled. Routes through the borrowing
    // callback wrapper rather than `lines_outside_fences` so we don't
    // allocate one `String` per scanned line.
    crate::body_sections::for_each_line_outside_fences(body, |i, line| {
        if checkbox_state(line).is_some() && line.contains(substring) {
            matches.push(i);
        }
    });
    match matches.len() {
        0 => Err(MutateError::Validation(format!(
            "no checkbox line matched {substring:?}"
        ))),
        1 => {
            let idx = matches[0];
            let toggled = toggle_line_checkbox(lines[idx]).ok_or_else(|| {
                MutateError::Validation(format!(
                    "internal: matched line {:?} is not a checkbox after match",
                    lines[idx]
                ))
            })?;
            let mut out = Vec::with_capacity(lines.len());
            for (i, l) in lines.iter().enumerate() {
                if i == idx {
                    out.push(toggled.clone());
                } else {
                    out.push((*l).to_string());
                }
            }
            Ok(out.join("\n"))
        }
        n => Err(MutateError::Validation(format!(
            "{n} checkbox lines matched {substring:?}; refine to a unique substring"
        ))),
    }
}

/// `Some(true)` for `- [x]`, `Some(false)` for `- [ ]`, `None`
/// otherwise. Byte-based parsing so multibyte mark chars (e.g.
/// `[✓]`, `[é]`) return `None` rather than panicking on a
/// non-char-boundary slice.
pub(crate) fn checkbox_state(line: &str) -> Option<bool> {
    let bytes = line.as_bytes();
    // Skip leading ASCII whitespace. A non-ASCII leading char means
    // this can't be a GFM checkbox line — return None.
    let mut i = 0;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    let bullet = *bytes.get(i)?;
    if !matches!(bullet, b'-' | b'*' | b'+') {
        return None;
    }
    i += 1;
    if !matches!(bytes.get(i), Some(b' ' | b'\t')) {
        return None;
    }
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if bytes.get(i) != Some(&b'[') {
        return None;
    }
    let mark = *bytes.get(i + 1)?;
    if bytes.get(i + 2) != Some(&b']') {
        return None;
    }
    if !matches!(bytes.get(i + 3), Some(b' ' | b'\t')) {
        return None;
    }
    match mark {
        b' ' => Some(false),
        b'x' | b'X' => Some(true),
        _ => None,
    }
}

/// Toggle the `[ ]` / `[x]` mark on a known-checkbox line. Returns
/// `None` if the line doesn't match the byte-safe checkbox shape —
/// callers should have validated via `checkbox_state` first. Builds
/// the result from raw bytes to avoid the implicit "ASCII-only first
/// 4 chars" invariant the previous string-slice version assumed.
pub(crate) fn toggle_line_checkbox(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if !matches!(bytes.get(i), Some(b'-' | b'*' | b'+')) {
        return None;
    }
    i += 1;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if bytes.get(i) != Some(&b'[') {
        return None;
    }
    let mark_idx = i + 1;
    let mark = *bytes.get(mark_idx)?;
    if bytes.get(i + 2) != Some(&b']') {
        return None;
    }
    let new_mark = if mark == b' ' { b'x' } else { b' ' };
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[..mark_idx]);
    out.push(new_mark);
    out.extend_from_slice(&bytes[mark_idx + 1..]);
    // Safe: we replaced one ASCII byte with another at a position
    // already proven to be ASCII by the byte checks above; the rest
    // of the line (including any non-ASCII content after `]`) is
    // copied byte-for-byte and therefore preserves UTF-8 validity.
    String::from_utf8(out).ok()
}

/// Drive a checkbox line to the target `checked` state regardless of
/// its current state. Returns `None` when the line doesn't match the
/// byte-safe checkbox shape — callers should have validated via
/// `checkbox_state` first. Mirror of `toggle_line_checkbox` but with
/// an explicit target so `set_checkbox_in_body` can be idempotent.
pub(crate) fn set_line_checkbox(line: &str, checked: bool) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if !matches!(bytes.get(i), Some(b'-' | b'*' | b'+')) {
        return None;
    }
    i += 1;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if bytes.get(i) != Some(&b'[') {
        return None;
    }
    let mark_idx = i + 1;
    if bytes.get(mark_idx).is_none() || bytes.get(i + 2) != Some(&b']') {
        return None;
    }
    let new_mark = if checked { b'x' } else { b' ' };
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[..mark_idx]);
    out.push(new_mark);
    out.extend_from_slice(&bytes[mark_idx + 1..]);
    String::from_utf8(out).ok()
}
