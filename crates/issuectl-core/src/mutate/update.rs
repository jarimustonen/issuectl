use super::*;

pub fn update_issue(
    root: &Path,
    slug: &str,
    req: UpdateIssueRequest,
) -> Result<UpdateOutcome, MutateError> {
    update_issue_via(root, slug, req, &SystemClock)
}

/// Clock-injected variant of [`update_issue`].
pub fn update_issue_via(
    root: &Path,
    slug: &str,
    req: UpdateIssueRequest,
    clock: &dyn Clock,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }

    // Normalize related-ref shapes BEFORE validate() so a typo'd ref
    // like `add_related: ["123"]` + `remove_related: ["#123"]`
    // (which both normalize to `#123`) is caught by the overlap check.
    let normalized_add_related = crate::refs::normalize_related_refs(&req.add_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let normalized_remove_related = crate::refs::normalize_related_refs(&req.remove_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let normalized_add_blocked_by = crate::refs::normalize_related_refs(&req.add_blocked_by)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let normalized_remove_blocked_by = crate::refs::normalize_related_refs(&req.remove_blocked_by)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    // Reject self-blockers up front. Doctor flags them too, but the
    // mutation API is the authoring surface — failing here keeps
    // `issuectl depend add foo --blocked-by foo` from producing a
    // file the next `doctor` run will immediately complain about.
    if normalized_add_blocked_by
        .iter()
        .any(|s| s.trim_start_matches('@') == slug)
    {
        return Err(MutateError::Validation(format!(
            "issue {slug:?} cannot block itself (blocked_by must reference a different slug)"
        )));
    }
    let mut req_normalized = req;
    req_normalized.add_related = normalized_add_related.clone();
    req_normalized.remove_related = normalized_remove_related.clone();
    req_normalized.add_blocked_by = normalized_add_blocked_by.clone();
    req_normalized.remove_blocked_by = normalized_remove_blocked_by.clone();
    let req = req_normalized;

    req.validate()?;

    // M13: an empty PATCH (all `Unspecified`, no list/commit changes)
    // is a no-op — return the current state without touching the file.
    // Without this short-circuit, an "empty" call would still bump
    // `updated:` and (surprisingly) trigger an in-line legacy→flat
    // migration. Read-only locate + parse, no write, no publish.
    // Dry-run noop also short-circuits — the diff would be empty.
    if req.is_noop() {
        let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
        let located = repo::locate_issue_full(root, slug).map_err(|_| MutateError::NotFound)?;
        let parsed = crate::parser::parse_item_md_with_warnings(&located.item_path, slug, "open");
        if !parsed.warnings.is_empty() {
            return Err(MutateError::Corrupt {
                warnings: parsed.warnings,
            });
        }
        let mut issue = parsed.issue;
        let schema =
            crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
        issue.folder = folder_for_status(&schema, &issue.status).to_string();
        let version = canonical_hash(&issue);
        if let Some(ref expected) = req.expected_version {
            if expected != &version {
                return Err(MutateError::VersionMismatch {
                    current: issue,
                    version,
                });
            }
        }
        return Ok(UpdateOutcome {
            issue_dir: located
                .item_path
                .parent()
                .expect("item.md has parent")
                .to_path_buf(),
            issue,
            version,
            moved_to_closed: false,
            moved_to_open: false,
            pending_serialized: None,
            before_serialized: None,
            warnings: Vec::new(),
        });
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    // Locate without migrating, regardless of dry_run. The legacy →
    // flat directory rename and the default-`.schema.yaml` *bootstrap*
    // used to run *before* `update_issue_under_lock`, which meant a
    // body op (or any other validation failure) could roll the issue's
    // content back while leaving `.schema.yaml` newly created and the
    // legacy directory permanently moved — directly contradicting the
    // documented "all-or-nothing under one flock" contract. We now
    // defer both side effects until validation has passed
    // (`update_issue_under_lock` runs them just before the atomic
    // write). Schema *load* and transition-rules load still happen
    // here so that a malformed config fails fast before any work is
    // attempted; those two paths produce typed errors
    // (`SchemaConfig` / `TransitionConfig`) and never write to disk.
    let item_path = locate_for_dry_run(root, slug)?;
    let schema =
        crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let rules = load_validated_rules(root, &schema)?;
    update_issue_under_lock(root, slug, item_path, req, &schema, &rules, clock)
}

/// Load `.issuectl/transitions.yaml` and cross-validate every status
/// it mentions against the schema's `status` enum. Typo'd status
/// names silently fail open (rule no-ops or denies everything), so
/// failing fast here gives the operator a precise pointer. All write
/// paths route through this helper.
pub(crate) fn load_validated_rules(
    root: &Path,
    schema: &crate::schema::Schema,
) -> Result<crate::transitions::TransitionRules, MutateError> {
    let rules = crate::transitions::load(root)
        .map_err(|e| MutateError::TransitionConfig(format!("{e:#}")))?;
    let universe = crate::schema::status_universe(schema);
    crate::transitions::validate_status_refs(&rules, &universe)
        .map_err(|e| MutateError::TransitionConfig(format!("{e:#}")))?;
    Ok(rules)
}

/// Project the in-flight `ItemFile` into the canonical `Issue` shape
/// the rules engine consumes. Serializes the item to the same byte
/// layout `write_item_atomic` would produce, then runs it through
/// `parser::parse_item_md_text_with_warnings`. This guarantees the
/// post-mutation projection cannot drift from the canonical reader
/// (the alternative — a hand-rolled subset parser — silently diverges
/// every time `parser` learns to normalise a new field). Cost is one
/// serialize + one parse per validated mutation.
pub(crate) fn projected_issue_for_rules(
    slug: &str,
    item: &write::ItemFile,
    item_path: &Path,
    schema: &crate::schema::Schema,
) -> Result<Issue, MutateError> {
    let text = write::serialize_item(item).map_err(MutateError::Io)?;
    let parsed = crate::parser::parse_item_md_text_with_warnings(&text, slug, "open", item_path);
    if !parsed.warnings.is_empty() {
        return Err(MutateError::Corrupt {
            warnings: parsed.warnings,
        });
    }
    let mut issue = parsed.issue;
    issue.folder = folder_for_status(schema, &issue.status).to_string();
    Ok(issue)
}

/// Body of `update_issue` that runs with the flock already held. Used
/// by `close_issue` to read+decide+mutate atomically without
/// double-acquiring the lock (which deadlocks on Linux because fs2's
/// advisory lock is per-fd).
///
/// `root` is threaded in (rather than derived from `item_path`) so the
/// dry-run branch can predict the *flat* `issue_dir` even when the
/// issue currently lives at a legacy path — a real write would migrate
/// it to flat layout, and the JSON envelope's `final_dir` must agree
/// (round-2 #3).
pub(crate) fn update_issue_under_lock(
    root: &Path,
    slug: &str,
    item_path: PathBuf,
    req: UpdateIssueRequest,
    schema: &crate::schema::Schema,
    rules: &crate::transitions::TransitionRules,
    clock: &dyn Clock,
) -> Result<UpdateOutcome, MutateError> {
    let folder = "open"; // placeholder; folder is derived from status post-write

    // 2) read + parse + hash. Refuse to mutate a corrupt file —
    // overwriting parser fallback defaults would silently destroy the
    // user's real (but malformed) on-disk content (§8.6 / M7).
    let parsed = crate::parser::parse_item_md_with_warnings(&item_path, slug, folder);
    if !parsed.warnings.is_empty() {
        return Err(MutateError::Corrupt {
            warnings: parsed.warnings,
        });
    }
    let mut current_issue = parsed.issue;
    current_issue.folder = folder_for_status(schema, &current_issue.status).to_string();
    let current_version = canonical_hash(&current_issue);
    let prev_status = current_issue.status.clone();
    let prev_type = current_issue.issue_type.clone();

    // 3) optimistic concurrency
    if let Some(ref expected) = req.expected_version {
        if expected != &current_version {
            return Err(MutateError::VersionMismatch {
                current: current_issue,
                version: current_version,
            });
        }
    }

    // 4) load the YAML mapping for in-place edits
    let mut item = write::read_item(&item_path).map_err(MutateError::Io)?;
    // Capture the canonicalised pre-mutation bytes before any in-memory
    // edit. Done under the held flock so the dry-run diff is against
    // the same state the mutation planned against — a concurrent
    // writer can't slip a different "before" into the diff.
    let before_serialized =
        if req.dry_run {
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
    // Snapshot once so a mutation crossing midnight keeps all derived
    // provenance fields internally consistent.
    let today = clock.today_string();
    let mut moved_to_closed = false;
    let mut moved_to_open = false;

    // Whole-body replacement (`set_body`). Applied first — before the
    // reopen-notes append, the `body_ops` loop, and the type-change
    // required-section check below — so each of those operates on and
    // validates the *replacement* body, not the stale one. Mirrors
    // `update_body`: preserve one leading newline so the on-disk layout
    // stays `---\n<fm>\n---\n\n<body>`, and surface the same non-fatal
    // reserved-legacy-heading warning (merged into the returned
    // `warnings` alongside the DoD advisories below).
    let mut body_warnings: Vec<String> = Vec::new();
    let mut type_warnings: Vec<String> = Vec::new();
    if let Some(body) = &req.set_body {
        let explicit_title = matches!(req.title, Patch::Set(_));
        let (body, mut title_warnings) = super::body::reconcile_replacement_title(
            slug,
            body.clone(),
            &current_issue.title,
            explicit_title,
        );
        body_warnings.append(&mut title_warnings);
        body_warnings.extend(crate::body_sections::reserved_section_warnings(&body));
        item.body = body;
    }

    // Title is body-backed, not frontmatter-backed. Apply it after a
    // whole-body replacement so an explicit `--title` remains authoritative
    // when both flags are supplied in one selective update. Strip the
    // ItemFile framing newline before a headingless repair, then restore the
    // canonical framing after all body/title edits.
    if let Patch::Set(title) = &req.title {
        item.body =
            crate::body_sections::set_title_heading(item.body.trim_start_matches('\n'), title);
    }
    if req.set_body.is_some() || matches!(req.title, Patch::Set(_)) {
        item.body = crate::body_sections::canonicalise_body_leading(&item.body);
    }

    // Frontmatter keys this mutation actually writes. Threaded into
    // `hard_schema_failure` so a `RequiredWhen` produced by this very
    // write (e.g. clearing `closed:` on a closing-status issue) is
    // rejected, while a pre-existing inconsistency on an untouched field
    // stays exempt (doctor heals those). NOTE: any new frontmatter write
    // added below must record its key here, or a violation it introduces
    // will be silently dropped.
    let mut written: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // Status change is a pure frontmatter PATCH; no directory rename
    // (post-flat-layout). The `moved_to_*` booleans now track the
    // active↔closing transition for messaging parity with the old API.
    if let Patch::Set(s) = &req.status {
        write::set_string(&mut item.frontmatter, "status", s);
        // The status branch always (re)evaluates the two close-lifecycle
        // fields — `closed:` and `closed_by:` — stamping, backfilling, or
        // removing each, so all three keys count as written.
        written.insert("status".into());
        written.insert("closed".into());
        written.insert("closed_by".into());
        let prev_closing = crate::schema::is_closing(schema, &prev_status);
        let new_closing = crate::schema::is_closing(schema, s);
        if new_closing {
            // Only set `closed:` on the active→closing edge, OR backfill
            // if the field is missing on a closing→closing transition
            // against an issue that pre-dates the auto-stamping.
            // Closing→closing (e.g. fixed→wontfix) MUST preserve the
            // historical close date — overwriting it would silently
            // destroy provenance.
            let has_closed = item
                .frontmatter
                .contains_key(serde_yaml::Value::String("closed".into()));
            if !prev_closing || !has_closed {
                write::set_string(&mut item.frontmatter, "closed", &today);
            }
            // `closed_by:` tracks `closed:`. An explicit attribution
            // (`close --as`, or a PATCH populating the slot) is written /
            // re-attributed. Without one, the active→closing edge scrubs
            // any stray value so an anonymous close never inherits a
            // stale closer, while a closing→closing re-status preserves
            // the recorded closer for the same provenance reason as the
            // `closed:` date above.
            match &req.closed_by {
                Patch::Set(author) => write::set_string(&mut item.frontmatter, "closed_by", author),
                Patch::Clear => write::remove_key(&mut item.frontmatter, "closed_by"),
                Patch::Unspecified => {
                    if !prev_closing {
                        write::remove_key(&mut item.frontmatter, "closed_by");
                    }
                }
            }
            if !prev_closing {
                moved_to_closed = true;
            }
        } else {
            write::remove_key(&mut item.frontmatter, "closed");
            // Closer attribution is close-time provenance; on reopen (or
            // any active status) it is stale, so drop it in lockstep with
            // `closed:`. Because `closed_by` is a reserved key, the only
            // writers are this lifecycle branch and the validated
            // request slot — so clearing here on the shared active edge
            // is authoritative and can't be re-added by a later
            // custom-field patch in the same call.
            write::remove_key(&mut item.frontmatter, "closed_by");
            if prev_closing {
                moved_to_open = true;
            }
        }
    } else if let Patch::Clear = &req.status {
        return Err(MutateError::Validation(
            "status cannot be cleared (issues always have a status)".into(),
        ));
    }

    if let Patch::Set(t) = &req.issue_type {
        write::set_string(&mut item.frontmatter, "type", t);
        written.insert("type".into());
    }
    for (key, patch) in [
        ("priority", &req.priority),
        ("reporter", &req.reporter),
        ("assignee", &req.assignee),
        ("owner", &req.owner),
        ("epic", &req.epic),
        ("lane", &req.lane),
    ] {
        apply_string_patch(&mut item, key, patch);
        if !matches!(patch, Patch::Unspecified) {
            written.insert(key.into());
        }
    }

    // lane_seq: numeric scalar patch (mirrors `lane`, but writes a YAML
    // integer rather than a string).
    match &req.lane_seq {
        Patch::Unspecified => {}
        Patch::Clear => {
            write::remove_key(&mut item.frontmatter, "lane_seq");
            written.insert("lane_seq".into());
        }
        Patch::Set(n) => {
            write::set_i64(&mut item.frontmatter, "lane_seq", *n);
            written.insert("lane_seq".into());
        }
    }

    if !req.add_labels.is_empty() || !req.remove_labels.is_empty() {
        written.insert("labels".into());
    }
    for label in &req.add_labels {
        write::add_to_string_list(&mut item.frontmatter, "labels", label)
            .map_err(MutateError::Io)?;
    }
    for label in &req.remove_labels {
        write::remove_from_string_list(&mut item.frontmatter, "labels", label)
            .map_err(MutateError::Io)?;
    }

    // related refs were normalized before validate(), so use them as-is.
    if !req.add_related.is_empty() || !req.remove_related.is_empty() {
        written.insert("related".into());
    }
    for r in &req.add_related {
        write::add_to_string_list(&mut item.frontmatter, "related", r).map_err(MutateError::Io)?;
    }
    for r in &req.remove_related {
        write::remove_from_string_list(&mut item.frontmatter, "related", r)
            .map_err(MutateError::Io)?;
    }

    // blocked_by: same shape contract as `related`. Normalization
    // already ran in `update_issue` so the list elements are bare
    // slugs by the time we get here.
    if !req.add_blocked_by.is_empty() || !req.remove_blocked_by.is_empty() {
        written.insert("blocked_by".into());
    }
    for r in &req.add_blocked_by {
        write::add_to_string_list(&mut item.frontmatter, "blocked_by", r)
            .map_err(MutateError::Io)?;
    }
    for r in &req.remove_blocked_by {
        write::remove_from_string_list(&mut item.frontmatter, "blocked_by", r)
            .map_err(MutateError::Io)?;
    }

    // collision: free-form hot-file tokens, same list mechanics as labels
    // (no ref normalization — they are file/family identifiers).
    if !req.add_collision.is_empty() || !req.remove_collision.is_empty() {
        written.insert("collision".into());
    }
    for c in &req.add_collision {
        write::add_to_string_list(&mut item.frontmatter, "collision", c)
            .map_err(MutateError::Io)?;
    }
    for c in &req.remove_collision {
        write::remove_from_string_list(&mut item.frontmatter, "collision", c)
            .map_err(MutateError::Io)?;
    }

    for spec in &req.add_commits {
        if spec.hash.is_empty() || spec.summary.is_empty() {
            return Err(MutateError::Validation(
                "commit hash and summary must be non-empty".into(),
            ));
        }
        write::add_commit(&mut item.frontmatter, &spec.hash, &spec.summary)
            .map_err(MutateError::Io)?;
    }

    // Custom-field patches. Reserved-key / shape checks already ran in
    // `validate()`; here we just translate the ternary onto the YAML
    // mapping. `Unspecified` shouldn't appear (BTreeMap entries imply
    // the caller mentioned the key) but is handled defensively.
    for (key, patch) in &req.custom_fields {
        match patch {
            Patch::Unspecified => {}
            Patch::Clear => {
                write::remove_key(&mut item.frontmatter, key);
                written.insert(key.clone());
            }
            Patch::Set(v) => {
                write::set_string(&mut item.frontmatter, key, v);
                written.insert(key.clone());
            }
        }
    }

    write::set_string(&mut item.frontmatter, "updated", &today);

    // Reopen flow: when transitioning closing → active, append a
    // `## Reopen Notes — <date>` section so the rationale isn't
    // implicit. One section per transition (multiple reopens stack).
    if moved_to_open {
        let trimmed_body = item.body.trim_start_matches('\n');
        let with_section = crate::body_sections::append_reopen_notes(trimmed_body, &today);
        item.body = crate::body_sections::canonicalise_body_leading(&with_section);
    }

    // Type-change rules. Only fire when `--type` is set AND the new
    // value actually differs from the current type — same-value sets
    // are a true no-op so idempotent JSON clients don't trip the
    // checks below.
    if let Patch::Set(new_type) = &req.issue_type {
        if new_type != &prev_type {
            // C4: forbid combining a close→open reopen with `--type`.
            // Both are body-mutating in different ways; bundling them
            // makes the resulting document harder to reason about and
            // is a rare combination in practice. Splitting into two
            // calls is the path forward.
            if moved_to_open {
                return Err(MutateError::Validation(
                    "cannot change --type while reopening (close→open) in the same call; \
                     run --status open first, then --type as a separate call"
                        .into(),
                ));
            }
            // D1: epic↔non-epic frontmatter invariants. A lone reporter
            // maps unambiguously to an epic owner, so migrate it and report
            // the semantic change. Assignees and conflicting owners remain
            // caller-actionable errors with an exact CLI remedy.
            if let Some(warning) = reconcile_type_invariants(new_type, &mut item.frontmatter, slug)?
            {
                written.insert("reporter".into());
                written.insert("owner".into());
                type_warnings.push(warning);
            }
            // C2: option 2 — reject when the new type's required body
            // sections aren't already present. Empty stubs would pass
            // `doctor` while the content is semantically blank, which
            // is worse than a clear error message (especially for AI
            // agents whose retry loop is well-defined here: edit body,
            // resubmit). Schema is fence-aware via
            // `body_sections::all_h2_sections`.
            let missing = crate::schema::missing_body_sections(
                schema,
                new_type,
                item.body.trim_start_matches('\n'),
            );
            if !missing.is_empty() {
                return Err(MutateError::SchemaViolation(format!(
                    "type {new_type:?} requires body sections that are missing: {}; \
                     add the section headings to the body first, then re-run --type",
                    missing
                        .iter()
                        .map(|s| format!("## {s}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
    }

    // Body ops apply in vector order so a patch's narrative reads top-
    // to-bottom: the user can toggle a checkbox and append a note that
    // refers to the toggle, all under the single flock above. Schema
    // and transition validation below run on the post-body state, so a
    // failing op rolls back the entire transaction (the in-memory
    // `item` is dropped without writing).
    for (i, op) in req.body_ops.iter().enumerate() {
        apply_body_op(&mut item, i, op, clock)?;
    }

    // 4b) schema validation against the post-mutation frontmatter. The
    //     built-in clap parsers already guard known enums; this layer
    //     enforces user-declared required fields and custom enums
    //     (e.g. a constrained `labels` enum). Schema is loaded once by
    //     the caller and threaded in so we don't re-read the file on
    //     each mutation.
    let violations = crate::schema::validate(schema, &item.frontmatter);
    if let Some(msg) = hard_schema_failure(&violations, &written) {
        return Err(MutateError::SchemaViolation(msg));
    }
    // Belt-and-braces status check. `schema::validate` only flags
    // out-of-enum values when `fields.status.enum` is declared — but
    // the schema's whole-spec replacement semantics let a user redeclare
    // `fields.status` without `enum:`, which would otherwise let any
    // string land here and silently default-classify as Active.
    // `status_universe()` falls back to the built-in `all_statuses()`
    // list in that no-enum case, so a typo can't sneak past.
    if let Some(status) = item
        .frontmatter
        .get(serde_yaml::Value::String("status".into()))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        let universe = crate::schema::status_universe(schema);
        if !universe.contains(status) {
            let mut allowed: Vec<&str> = universe.iter().map(String::as_str).collect();
            allowed.sort();
            return Err(MutateError::SchemaViolation(format!(
                "status {status:?} is not in the allowed set [{}]",
                allowed.join(", ")
            )));
        }
    }

    // 4c) declarative transition-rule check. Coordinates with the
    //     mutation verbs in the same module by sharing a single hook
    //     surface — both write paths apply rules through the
    //     post-mutation `Issue` projection. Rules are loaded once by
    //     the caller (same pattern as `schema`).
    let projected = projected_issue_for_rules(slug, &item, &item_path, schema)?;

    // Intrinsic intake invariants (OD-9 A) — always on, independent of
    // `.issuectl/transitions.yaml`. Routed through here so the generic
    // `set status` / `update --status` path enforces the same type ×
    // status and reception-state rules as the first-class intake verbs;
    // neither is a bypass. Gated on an actual status/type change so a
    // no-op re-assert or an unrelated field PATCH against legacy data is
    // never retroactively rejected.
    if projected.status != prev_status || projected.issue_type != prev_type {
        let intrinsic = intake::intrinsic_transition_violations(
            &prev_status,
            &prev_type,
            &projected.status,
            &projected.issue_type,
        );
        if !intrinsic.is_empty() {
            return Err(MutateError::TransitionViolation(intrinsic.join("; ")));
        }
    }

    let mut rule_violations =
        crate::transitions::evaluate_transition(rules, &projected, &prev_status);
    let (dod_warnings, dod_errors) =
        crate::transitions::evaluate_dod(schema, &projected, &prev_status);
    rule_violations.extend(dod_errors);
    if !rule_violations.is_empty() {
        return Err(MutateError::TransitionViolation(rule_violations.join("; ")));
    }
    // Fold the body-replacement advisories in with the DoD warnings so
    // every return path below surfaces both to the caller in one list.
    let mut dod_warnings = dod_warnings;
    dod_warnings.append(&mut body_warnings);
    dod_warnings.append(&mut type_warnings);

    // Post-mutation closing classification drives both the dry-run dir
    // prediction and the real unarchive decision below — an archived
    // issue left non-closing gets lifted back to the active root.
    let post_closing = crate::schema::is_closing(schema, &projected.status);

    // 5) Either dry-run (compute serialized bytes, skip write/publish)
    //    or atomic write. The only directory move is unarchiving (an
    //    archived issue reopened to a non-closing status); flat-layout
    //    status changes keep `item_path` as the canonical location.
    if req.dry_run {
        let pending = write::serialize_item(&item).map_err(MutateError::Io)?;
        let new_issue = parse_serialized(&pending, slug, schema);
        let new_version = canonical_hash(&new_issue);
        return Ok(UpdateOutcome {
            issue: new_issue,
            version: new_version,
            issue_dir: predicted_issue_dir(root, slug, &item_path, post_closing),
            moved_to_closed,
            moved_to_open,
            pending_serialized: Some(pending),
            before_serialized,
            warnings: dod_warnings,
        });
    }
    // 5b) Side effects deferred from `update_issue` so they only fire
    //     after every validation step above has passed. Schema
    //     bootstrap and the legacy → flat directory migration would
    //     otherwise leak past a rolled-back transaction (failed body
    //     op, schema violation, transition rejection): the on-disk
    //     `item.md` would be unchanged but `.schema.yaml` would be
    //     newly created and the legacy directory permanently moved.
    crate::schema::ensure_default_written(root).map_err(MutateError::Io)?;
    // An archived issue left in a non-closing status must be lifted out of
    // cold storage: otherwise the frontmatter write below lands on the
    // `issues/archive/YYYY/MM/<slug>/` path and the issue reads as active
    // in `list`/`show` while physically still living in the archive. This
    // is the inverse of the `archive` move and runs under the same flock.
    let item_path = unarchive_if_active(root, slug, item_path, post_closing)?;
    let final_path = migrate_to_flat_if_legacy(root, slug, &item_path)?;
    write_item_atomic(&final_path, &item).map_err(MutateError::Io)?;

    // 6) recompute canonical hash from final on-disk content
    let after = crate::parser::parse_item_md_with_warnings(&final_path, slug, "open");
    let mut new_issue = after.issue;
    new_issue.folder = folder_for_status(schema, &new_issue.status).to_string();
    let new_version = canonical_hash(&new_issue);

    Ok(UpdateOutcome {
        issue: new_issue,
        version: new_version,
        issue_dir: final_path
            .parent()
            .expect("written file has a parent")
            .to_path_buf(),
        moved_to_closed,
        moved_to_open,
        pending_serialized: None,
        before_serialized: None,
        warnings: dod_warnings,
    })
}

/// Re-parse already-serialized `item.md` bytes into a domain `Issue`.
/// Dry-run paths serialize the post-mutation `ItemFile` to compute
/// `pending_serialized` for the diff; passing those same bytes back
/// here avoids serializing a second time.
pub(crate) fn parse_serialized(
    serialized: &str,
    slug: &str,
    schema: &crate::schema::Schema,
) -> Issue {
    let parsed = crate::parser::parse_item_md_text_with_warnings(
        serialized,
        slug,
        "open",
        Path::new("<dry-run>"),
    );
    let mut issue = parsed.issue;
    issue.folder = folder_for_status(schema, &issue.status).to_string();
    issue
}
