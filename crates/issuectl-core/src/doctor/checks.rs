use super::*;

#[allow(dead_code)] // retained as the SystemClock convenience for inline tests.
pub(crate) fn scan(repo_root: &Path) -> Result<DoctorFindings> {
    scan_via(repo_root, &crate::clock::SystemClock)
}

/// Clock-injected scan used by [`run_via`] and deterministic tests.
pub(crate) fn scan_via(
    repo_root: &Path,
    clock: &dyn crate::clock::Clock,
) -> Result<DoctorFindings> {
    let mut report = DoctorFindings::default();
    let scan = scan_issues(repo_root)?;

    populate_slug_and_legacy(&scan, repo_root, &mut report);
    populate_orphan_epic_refs(&scan, &mut report);

    report.schema_missing = !schema::schema_path(repo_root).is_file();
    let schema_value = match schema::load(repo_root) {
        Ok(s) => Some(s),
        Err(e) => {
            report.schema_parse_error = Some(e.to_string());
            None
        }
    };
    if let Some(s) = schema_value.as_ref() {
        // Coercion detection runs first so `populate_schema_violations`
        // can suppress the enum violation for any value the coercion
        // will rewrite (otherwise the user sees the same value flagged
        // both as a violation and as a pending fix).
        populate_alias_coercions(&scan, s, &mut report);
        populate_schema_violations(&scan, repo_root, s, &mut report);
    }

    // Transition rules + body-section linting. Both are warning-only
    // (legacy data may pre-date the rules).
    let rules = match crate::transitions::load(repo_root) {
        Ok(r) => {
            // N2: cross-validate status references against the schema
            // enum so a typo'd status surfaces here too.
            if let Some(s) = schema_value.as_ref() {
                let universe = schema::status_universe(s);
                if let Err(e) = crate::transitions::validate_status_refs(&r, &universe) {
                    report.parse_errors.push(ParseError {
                        location: crate::transitions::RULES_RELATIVE_PATH.to_string(),
                        message: format!("{e:#}"),
                        severity: ParseSeverity::Hard,
                    });
                    None
                } else {
                    Some(r)
                }
            } else {
                Some(r)
            }
        }
        Err(e) => {
            report.parse_errors.push(ParseError {
                location: crate::transitions::RULES_RELATIVE_PATH.to_string(),
                message: format!("{e:#}"),
                severity: ParseSeverity::Hard,
            });
            None
        }
    };
    if rules.is_some() || schema_value.is_some() {
        populate_transition_warnings(&scan, rules.as_ref(), schema_value.as_ref(), &mut report);
        // M2: stable, deterministic ordering for CLI text + JSON +
        // tests. `read_dir` traversal order is platform-dependent.
        report.transition_warnings.sort();
        report.missing_body_sections.sort();
    }

    // AGENTS.md drift. Only flag when the file already exists — the
    // file itself is opt-in (`issuectl agents init`). Both loaders
    // already return defaults on missing file, so a non-Err return
    // means we can trust the values; an Err signals parse/version
    // trouble and we MUST NOT regenerate from defaults (would
    // overwrite real policy with empty rules).
    let agents_path = agents::agents_path(repo_root);
    if !agents_path.exists() {
        report.agents_md_missing = true;
    }
    if agents_path.is_file() {
        if let Ok(text) = fs::read_to_string(&agents_path) {
            match (schema::load(repo_root), crate::transitions::load(repo_root)) {
                (Ok(s), Ok(r)) => match agents::locate_managed_block(&text) {
                    agents::BlockLocation::Malformed { reason } => {
                        report.agents_md_malformed = Some(reason);
                    }
                    _ => {
                        if !agents::managed_in_sync(&text, &s, &r) {
                            report.agents_md_drift = true;
                        }
                    }
                },
                (Err(e), _) | (_, Err(e)) => {
                    report.agents_md_check_skipped = Some(format!("{e:#}"));
                }
            }
        }
    }

    // Legacy `issues/AGENTS.md` scaffold (pre-v0.5.0): the old template
    // documented numbered `<NN>-<slug>/` layout, `open/` / `closed/`
    // subdirs, and sequential numbering — none of which apply now. Flag
    // only when known legacy markers appear so customized files survive.
    let issues_agents_path = repo_root.join("issues").join("AGENTS.md");
    if issues_agents_path.is_file() {
        if let Ok(text) = fs::read_to_string(&issues_agents_path) {
            if text != crate::skill::ISSUES_AGENTS_TEMPLATE && is_legacy_issues_agents(&text) {
                report.legacy_issues_agents_md = true;
            }
        }
    }

    report.gitignored_paths = detect_gitignored_canonical_paths(repo_root);

    let plan = plan_migrate_layout(repo_root)?;
    report.flat_layout_conflicts = plan.conflicts().to_vec();
    report.flat_layout_plan = Some(plan);

    // Round-2 finding O6: read-only `doctor` must surface pending
    // Notes migrations and conflicts so users see the work even
    // before running `--fix`.
    populate_notes_migration(&scan, &mut report);

    populate_extended_validation(&scan, schema_value.as_ref(), &mut report, clock);

    populate_attachment_health(&scan, repo_root, &mut report);

    Ok(report)
}

/// Slug uniqueness, legacy-migration plan, missing-item-md, parse
/// warnings, invalid slug detection. Mirrors the original main scan
/// loop but consumes `ScanResult` instead of re-reading from disk.
pub(crate) fn populate_slug_and_legacy(
    scan: &ScanResult,
    repo_root: &Path,
    report: &mut DoctorFindings,
) {
    let issues_dir = repo_root.join("issues");
    let mut all_slugs: BTreeMap<String, usize> = BTreeMap::new();

    for s in &scan.issues {
        let location = format!("{}/{}", s.folder, s.dir_name);
        if let Some(number) = s.legacy_number {
            let new_slug = slug::generate_unique(repo_root);
            // Always migrate to the canonical flat path — even if the
            // legacy `<NN>-<slug>` dir lives under
            // `issues/{open,closed}/`, doctor `--fix` should bring it
            // forward to the post-flat-layout home in one pass.
            let new_path = issues_dir.join(&new_slug);
            report.legacy_dirs.push(LegacyMigration {
                folder: s.folder.clone(),
                old_dir_name: s.dir_name.clone(),
                old_path: s.dir_path.clone(),
                new_slug: new_slug.clone(),
                new_path,
                old_number: number,
            });
            *all_slugs.entry(new_slug).or_insert(0) += 1;
        } else {
            // Report invalid slug + duplicate even when item.md is
            // missing — the directory is still a problem worth flagging.
            if !slug::is_valid(&s.dir_name) {
                report.invalid_slugs.push(location.clone());
            }
            *all_slugs.entry(s.dir_name.clone()).or_insert(0) += 1;
        }

        if !s.item_present {
            report.missing_item_md.push(location);
            continue;
        }

        // Surface parse warnings without printing them to stderr.
        // For LEGACY directories, only HARD errors are surfaced —
        // SOFT warnings (legacy numeric refs etc.) are noise on
        // dirs the migration pass rewrites wholesale. HARD errors
        // (frontmatter unparseable, file unreadable) MUST surface
        // even for legacy issues: `--fix`'s `rewrite_item_frontmatter`
        // calls `write::read_item`, which would panic mid-apply on
        // an unparseable file. Letting them flow through into
        // `critical_blockers` makes preflight refuse cleanly.
        if let Some(parsed) = &s.parsed {
            let severity = if parsed.has_hard_frontmatter_error() {
                ParseSeverity::Hard
            } else {
                ParseSeverity::Soft
            };
            if s.legacy_number.is_some() && severity == ParseSeverity::Soft {
                continue;
            }
            for w in &parsed.warnings {
                report.parse_errors.push(ParseError {
                    location: location.clone(),
                    message: w.clone(),
                    severity,
                });
            }
        }
    }

    for (slug_name, n) in &all_slugs {
        if *n > 1 {
            report.duplicate_slugs.push(slug_name.clone());
        }
    }
}

/// Orphan epic-reference detection. Uses the cached parser output for
/// each issue rather than re-reading every `item.md`.
pub(crate) fn populate_orphan_epic_refs(scan: &ScanResult, report: &mut DoctorFindings) {
    let mut existing_slugs: BTreeSet<String> = BTreeSet::new();
    for s in &scan.issues {
        existing_slugs.insert(s.dir_name.clone());
        if let Some((_, rest)) = parser::parse_legacy_dir(&s.dir_name) {
            existing_slugs.insert(rest);
        }
    }
    for s in &scan.issues {
        if !s.item_present {
            continue;
        }
        let Some(parsed) = &s.parsed else { continue };
        if let Some(epic) = parsed.issue.epic.as_deref() {
            let stripped = epic.strip_prefix('@').unwrap_or(epic);
            let exists = existing_slugs.contains(stripped) || stripped.parse::<u32>().is_ok();
            if !exists {
                report
                    .orphan_epic_refs
                    .push((s.dir_name.clone(), epic.to_string()));
            }
        }
    }
}

/// Attachment / fixture health: large binaries, non-AVIF images, and
/// relative body references that no longer resolve. All warning-only —
/// these never enter `blockers_for`, so they cannot block `--fix` or
/// flip the exit code. Walks the whole issue directory tree (item.md and
/// atomic-write tempfiles excluded, symlinks not followed) — that
/// naturally covers `attachments/` and `fixtures/` as well as any other
/// files an issue carries.
pub(crate) fn populate_attachment_health(
    scan: &ScanResult,
    repo_root: &Path,
    report: &mut DoctorFindings,
) {
    for s in &scan.issues {
        let mut files = Vec::new();
        collect_issue_files(&s.dir_path, &mut files);
        for path in &files {
            // The issue's own item.md is text we already lint elsewhere.
            if path == &s.item_path {
                continue;
            }
            if let Ok(meta) = fs::metadata(path) {
                if meta.len() > LARGE_BINARY_BYTES {
                    report.large_binaries.push((
                        s.dir_name.clone(),
                        rel(repo_root, path),
                        meta.len(),
                    ));
                }
            }
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if NON_AVIF_IMAGE_EXTS
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(ext))
                {
                    report
                        .non_avif_images
                        .push((s.dir_name.clone(), rel(repo_root, path)));
                }
            }
        }

        // Relative body references pointing inside the issue dir that no
        // longer resolve. Scan only the body — a YAML frontmatter value
        // can legitimately contain `[text](paren)` syntax, which would
        // otherwise register as a phantom broken reference. A target
        // carrying a GitHub-style `#L<n>` line anchor that also exists
        // at the repo root is a cross-file code permalink, not an
        // attachment — skip those. Crucially the skip is gated on the
        // anchor shape so a bare `![logo](README.md)` referencing a
        // missing sibling is NOT silently masked by an unrelated
        // `README.md` at the repo root.
        if let Some(text) = &s.text {
            let body = crate::item_text::split(text).body;
            for r in crate::refs::extract_relative_body_refs(body) {
                if s.dir_path.join(&r.path).exists() {
                    continue;
                }
                if r.has_line_anchor && repo_root.join(&r.path).exists() {
                    continue;
                }
                report
                    .broken_attachment_refs
                    .push((s.dir_name.clone(), r.path));
            }
        }
    }
    report.large_binaries.sort();
    report.non_avif_images.sort();
    report.broken_attachment_refs.sort();
}

/// Recursively collect regular files under `dir`, skipping symlinks and
/// atomic-write tempfiles. Used by `populate_attachment_health`.
pub(crate) fn collect_issue_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(ftype) = entry.file_type() else {
            continue;
        };
        if ftype.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ftype.is_dir() {
            collect_issue_files(&path, out);
        } else if ftype.is_file() {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(".issuectl-tmp-"))
            {
                continue;
            }
            out.push(path);
        }
    }
}

/// v0.5.0 validation suite (reference integrity, status/closed
/// consistency, timestamp sanity, unknown-key flagging, conflict
/// markers, orphan tempfiles, symlinked dirs, status-folder
/// mismatches). Reads no files — operates entirely on the cached
/// `ScanResult`.
pub(crate) fn populate_extended_validation(
    scan: &ScanResult,
    schema: Option<&schema::Schema>,
    report: &mut DoctorFindings,
    clock: &dyn crate::clock::Clock,
) {
    use chrono::NaiveDate;

    report.symlinked_dirs = scan.symlinked_dirs.clone();
    report.orphan_tempfiles = scan.tempfiles.clone();

    // Group present-issue records by slug across flat + legacy folders.
    let mut by_slug: BTreeMap<String, Vec<&ScannedIssue>> = BTreeMap::new();
    for s in &scan.issues {
        if !s.item_present {
            continue;
        }
        by_slug.entry(s.dir_name.clone()).or_default().push(s);
    }

    // Both open/<slug> AND closed/<slug>: ambiguous; never auto-fix.
    for (slug, hits) in &by_slug {
        let has_open = hits.iter().any(|h| h.folder == "open");
        let has_closed = hits.iter().any(|h| h.folder == "closed");
        if has_open && has_closed {
            report.both_open_and_closed.push(slug.clone());
        }
    }

    // Schema-known field names for unknown-key flagging. Use the
    // pre-loaded schema if available, otherwise fall back to the
    // built-in defaults so the universe of known keys is never empty.
    let owned_default;
    let known_schema = match schema {
        Some(s) => s,
        None => {
            owned_default = schema::default_schema();
            &owned_default
        }
    };
    let mut known: BTreeSet<String> = known_schema.fields.keys().cloned().collect();
    // Frontmatter keys the parser/canonical layer recognises but the
    // built-in schema may not declare (e.g. `commits`, `blocked_by`,
    // `number`).
    for k in [
        "created",
        "updated",
        "type",
        "reporter",
        "assignee",
        "owner",
        "status",
        "priority",
        "epic",
        "related",
        "labels",
        "closed",
        "commits",
        // `lane_seq` is a typed field lifted by the parser but, like
        // `commits`, intentionally NOT declared in the schema (the v1
        // string validator would reject the YAML integer). Recognise it
        // here so doctor doesn't flag it as an unknown key.
        "lane_seq",
        "slug",
        "number",
        "blocked_by",
        "reviewer",
        "review_status",
    ] {
        known.insert(k.to_string());
    }

    // Universe of "known users" for the reviewer-validation check. We
    // accept any name that appears as `reporter:`, `assignee:`, or
    // `owner:` on at least one issue in the repo — there is no
    // separate user catalog, so reusing the values already in the
    // graph is the lightest-weight signal. Empty strings are stripped
    // so a stray `reviewer: ""` (which the typed parser already
    // forbids via the trim check, but custom-field writes could
    // sneak through) doesn't validate against another empty entry.
    let mut known_users: BTreeSet<String> = BTreeSet::new();
    for hits in by_slug.values() {
        let primary = hits
            .iter()
            .find(|h| h.folder == "flat")
            .copied()
            .unwrap_or(hits[0]);
        let Some(fm) = primary.parsed.as_ref().and_then(|p| p.mapping.as_ref()) else {
            continue;
        };
        for key in ["reporter", "assignee", "owner"] {
            if let Some(v) = fm
                .get(serde_yaml::Value::String(key.into()))
                .and_then(|v| v.as_str())
            {
                let v = v.trim();
                if !v.is_empty() {
                    known_users.insert(v.to_string());
                }
            }
        }
    }

    let today = clock.today();
    let existing_slugs: BTreeSet<String> = by_slug.keys().cloned().collect();
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (slug, hits) in &by_slug {
        // For status reconciliation we want every legacy path
        // occurrence; for the rest, the canonical (flat) hit if any,
        // else the first legacy hit.
        let primary: &ScannedIssue = hits
            .iter()
            .find(|h| h.folder == "flat")
            .copied()
            .unwrap_or(hits[0]);

        let Some(text) = primary.text.as_deref() else {
            continue;
        };
        if has_conflict_markers(text) {
            report.conflict_markers.push(slug.clone());
        }

        let Some(fm) = primary.parsed.as_ref().and_then(|p| p.mapping.as_ref()) else {
            continue;
        };

        // Skip lints that flag findings critical_blockers treats as
        // hard refusals when the issue is queued for the NN-rename
        // pipeline. `--fix` migrates these wholesale (frontmatter
        // rewritten, refs translated, file renamed), so emitting
        // hard findings on them would refuse the very fix designed
        // to heal them. The typed signal is `legacy_number.is_some()`
        // — applies whether the dir lives at `issues/{open,closed}/`
        // (pre-migration) or at the flat root (post flat-layout
        // migration but before NN-rename). Mirrors the skip in
        // `populate_schema_violations`.
        let primary_is_legacy = primary.legacy_number.is_some();
        if primary_is_legacy {
            // Still run the per-hit status/folder reconciliation pass
            // below — that is exactly the legacy state `--fix` heals,
            // and emitting `closed_with_active_status` /
            // `open_with_closing_status` here is what triggers the
            // reconciliation.
            for hit in hits {
                if hit.folder != "open" && hit.folder != "closed" {
                    continue;
                }
                let Some(fm) = hit.parsed.as_ref().and_then(|p| p.mapping.as_ref()) else {
                    continue;
                };
                let Some(hit_status) = fm
                    .get(serde_yaml::Value::String("status".into()))
                    .and_then(|v| v.as_str())
                else {
                    continue;
                };
                match hit.folder.as_str() {
                    "closed" if crate::issue_fields::ACTIVE_STATUSES.contains(&hit_status) => {
                        report.closed_with_active_status.push((
                            slug.clone(),
                            hit_status.to_string(),
                            hit.item_path.clone(),
                        ));
                    }
                    "open" if crate::issue_fields::is_closing_status(hit_status) => {
                        report.open_with_closing_status.push((
                            slug.clone(),
                            hit_status.to_string(),
                            hit.item_path.clone(),
                        ));
                    }
                    _ => {}
                }
            }
            continue;
        }

        // Unknown-key flagging.
        for (k, _) in fm.iter() {
            if let serde_yaml::Value::String(name) = k {
                if !known.contains(name) {
                    report.unknown_keys.push((slug.clone(), name.clone()));
                }
            }
        }

        // Reviewer must be a known user. The check fires only when
        // `reviewer:` is present and the value is a non-empty string;
        // shape errors are surfaced by schema validation and the typed
        // parser, not here.
        if let Some(reviewer) = fm
            .get(serde_yaml::Value::String("reviewer".into()))
            .and_then(|v| v.as_str())
        {
            let reviewer = reviewer.trim();
            if !reviewer.is_empty() && !known_users.contains(reviewer) {
                report
                    .unknown_reviewers
                    .push((slug.clone(), reviewer.to_string()));
            }
        }

        let status = fm
            .get(serde_yaml::Value::String("status".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let closed = fm
            .get(serde_yaml::Value::String("closed".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        let closed_by = fm
            .get(serde_yaml::Value::String("closed_by".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        let created = fm
            .get(serde_yaml::Value::String("created".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let updated = fm
            .get(serde_yaml::Value::String("updated".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Status/closed consistency. Schema-aware: a project that
        // declares `archived` (or similar) as a closing status via
        // `status_classes:` in `.schema.yaml` flags `archived without
        // closed:` here just like a built-in `done`. Both branches go
        // through the same `status_class` lookup so the layering
        // (schema → built-in → default-active) is applied
        // consistently. An unknown status defaults to `Active` in
        // `status_class`; we suppress the active-side check for
        // unknowns so doctor doesn't pile a confusing "active status
        // must not carry closed:" on top of the schema-validation
        // failure that already flagged the typo.
        if let Some(s) = &status {
            let class = schema::status_class(known_schema, s);
            let recognised = known_schema.status_classes.contains_key(s.as_str())
                || crate::issue_fields::ACTIVE_STATUSES.contains(&s.as_str())
                || crate::issue_fields::is_closing_status(s);
            match class {
                // Gate on the schema's `required_when` declaration so
                // the closing-side rule is the SAME one `schema::validate`
                // enforces (and so relaxing/removing `closed.required_when`
                // in `.schema.yaml` relaxes this finding too). The
                // built-in default declares it, so behaviour is unchanged
                // for stock repos.
                schema::StatusClass::Closing
                    if closed.is_none()
                        && schema::field_required_for_status(known_schema, "closed", s) =>
                {
                    report.status_consistency.push((
                        slug.clone(),
                        format!("closing status {s:?} requires `closed:` date"),
                    ));
                }
                schema::StatusClass::Active if recognised && closed.is_some() => {
                    report.status_consistency.push((
                        slug.clone(),
                        format!("active status {s:?} must not carry `closed:`"),
                    ));
                }
                _ => {}
            }

            // `closed_by:` tracks `closed:` — the close path scrubs it on
            // the active edge, so an active issue carrying a closer is
            // self-inconsistent (legacy or hand-edited state). Flag it on
            // any recognised active status, independently of the `closed:`
            // check above so a stranded `closed_by` still surfaces even
            // when `closed:` was already cleared.
            if matches!(class, schema::StatusClass::Active) && recognised && closed_by.is_some() {
                report.status_consistency.push((
                    slug.clone(),
                    format!("active status {s:?} must not carry `closed_by:`"),
                ));
            }
        }

        // Timestamp sanity.
        let parse = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok();
        let cd = created.as_deref().and_then(parse);
        let ud = updated.as_deref().and_then(parse);
        let xd = closed.as_deref().and_then(parse);
        if let (Some(c), Some(u)) = (cd, ud) {
            if c > u {
                report.timestamp_issues.push((
                    slug.clone(),
                    format!("created ({c}) is after updated ({u})"),
                ));
            }
        }
        for (label, d) in [("created", cd), ("updated", ud), ("closed", xd)] {
            if let Some(d) = d {
                if d > today {
                    report
                        .timestamp_issues
                        .push((slug.clone(), format!("{label} date {d} is in the future")));
                }
            }
        }
        if let (Some(u), Some(x)) = (ud, xd) {
            if x > u {
                report
                    .timestamp_issues
                    .push((slug.clone(), format!("closed ({x}) is after updated ({u})")));
            }
        }

        // Reference integrity.
        let check_ref = |raw: &str| -> Option<String> {
            let trimmed = raw.trim();
            let bare = trimmed
                .strip_prefix('@')
                .or_else(|| trimmed.strip_prefix('#'))
                .unwrap_or(trimmed);
            if bare.is_empty() {
                return None;
            }
            if bare.chars().all(|c| c.is_ascii_digit()) {
                return Some(format!("{bare} (legacy numeric ref)"));
            }
            if !crate::slug::is_valid(bare) {
                return Some(bare.to_string());
            }
            if !existing_slugs.contains(bare) {
                return Some(bare.to_string());
            }
            None
        };

        if let Some(epic_v) = fm.get(serde_yaml::Value::String("epic".into())) {
            let epic_str = match epic_v {
                serde_yaml::Value::String(s) => Some(s.clone()),
                serde_yaml::Value::Number(n) => Some(n.to_string()),
                _ => None,
            };
            if let Some(epic) = epic_str {
                if let Some(missing) = check_ref(&epic) {
                    report
                        .broken_refs
                        .push((slug.clone(), "epic".into(), missing));
                }
            }
        }
        for key in ["related", "blocked_by"] {
            if let Some(serde_yaml::Value::Sequence(seq)) =
                fm.get(serde_yaml::Value::String(key.into()))
            {
                let mut deps = Vec::new();
                for item in seq {
                    if let Some(s) = item.as_str() {
                        if let Some(missing) = check_ref(s) {
                            report
                                .broken_refs
                                .push((slug.clone(), key.to_string(), missing));
                        } else if key == "blocked_by" {
                            let bare = s.trim().strip_prefix('@').unwrap_or(s.trim()).to_string();
                            if bare == *slug {
                                // Self-dep: surface explicitly so the user
                                // gets a focused remediation, and skip it
                                // for the cycle graph so we don't double-
                                // report it as a (trivial) 1-node cycle.
                                if !report.blocked_by_self.contains(slug) {
                                    report.blocked_by_self.push(slug.clone());
                                }
                            } else if existing_slugs.contains(&bare) {
                                deps.push(bare);
                            }
                        }
                    }
                }
                if key == "blocked_by" && !deps.is_empty() {
                    graph.insert(slug.clone(), deps);
                }
            }
        }

        // Status/folder reconciliation (legacy folders only). Use each
        // hit's own cached mapping — a slug present in flat AND legacy
        // can have divergent status fields.
        let in_both_legacy_folders =
            hits.iter().any(|h| h.folder == "open") && hits.iter().any(|h| h.folder == "closed");
        if in_both_legacy_folders {
            continue;
        }
        for hit in hits {
            if hit.folder != "open" && hit.folder != "closed" {
                continue;
            }
            let Some(fm) = hit.parsed.as_ref().and_then(|p| p.mapping.as_ref()) else {
                continue;
            };
            let Some(hit_status) = fm
                .get(serde_yaml::Value::String("status".into()))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            // A status value that `--fix` will coerce is owned by the
            // alias pass — skip reconciliation for it so the two passes
            // don't both rewrite the same field (which classified the
            // pre-coercion value with the lenient `Active` default and
            // would otherwise clobber the coerced result).
            if schema::would_coerce(known_schema, "status", hit_status).is_some() {
                continue;
            }
            match hit.folder.as_str() {
                "closed"
                    if (known_schema.status_classes.contains_key(hit_status)
                        || crate::issue_fields::ACTIVE_STATUSES.contains(&hit_status)
                        || crate::issue_fields::is_closing_status(hit_status))
                        && schema::status_class(known_schema, hit_status)
                            == schema::StatusClass::Active =>
                {
                    report.closed_with_active_status.push((
                        slug.clone(),
                        hit_status.to_string(),
                        hit.item_path.clone(),
                    ));
                }
                "open" if schema::is_closing(known_schema, hit_status) => {
                    report.open_with_closing_status.push((
                        slug.clone(),
                        hit_status.to_string(),
                        hit.item_path.clone(),
                    ));
                }
                _ => {}
            }
        }
    }

    report.blocked_by_cycles = detect_cycles(&graph);
}

/// Canonical issuectl-tracked files that should never be ignored by
/// `.gitignore`. If any of these exist locally but `git check-ignore`
/// says they're masked, teammates and CI won't see them — the local
/// developer will believe `agents init` / schema setup worked.
const GITIGNORE_CANONICAL_PATHS: &[&str] = &[".issuectl/AGENTS.md", "issues/.schema.yaml"];

/// Files larger than this under an issue directory are flagged as large
/// binaries (warning-only). 1 MiB: a git-tracked issue tracker shouldn't
/// carry artifacts this size inline without a deliberate choice, because
/// every revision is kept forever in history. The suggested remedies are
/// external storage or a `.gitignore` entry.
const LARGE_BINARY_BYTES: u64 = 1 << 20;

/// Raster image extensions the AVIF convention asks contributors to
/// convert. Flagged as warning-only nudges, independent of size.
const NON_AVIF_IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tiff", "tif"];

/// Run `git check-ignore -- <path>...` against the canonical paths
/// that exist on disk and return those that git would actually ignore.
/// Silent no-op when this is not a git repo or `git` is unavailable.
///
/// Deliberately does NOT pass `--no-index`. Without that flag, git
/// returns exit 1 (not ignored) for any tracked file even when the
/// path matches a `.gitignore` pattern — which is the correct
/// semantics for the "teammates won't see this file" warning. With
/// `--no-index`, git reports tracked-but-pattern-matched files as
/// ignored, producing false positives in the common migration
/// scenario where someone committed `.issuectl/AGENTS.md` and later
/// added `.issuectl/` to `.gitignore`.
pub(crate) fn detect_gitignored_canonical_paths(repo_root: &Path) -> Vec<String> {
    let candidates: Vec<&str> = GITIGNORE_CANONICAL_PATHS
        .iter()
        .copied()
        .filter(|rel| repo_root.join(rel).exists())
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("check-ignore")
        .arg("--")
        .args(&candidates)
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    // git check-ignore exit codes:
    //   0 — at least one path is ignored
    //   1 — no paths ignored
    //   128 — fatal error (e.g. not a git repo)
    if !matches!(output.status.code(), Some(0)) {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut hits: Vec<String> = stdout
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();
    hits.sort();
    hits.dedup();
    hits
}

pub(crate) fn has_conflict_markers(text: &str) -> bool {
    use crate::body_sections::{closes_fence, opening_fence, Fence};
    let mut fence: Option<Fence> = None;
    for line in text.lines() {
        match fence {
            Some(open) if closes_fence(line, open) => {
                fence = None;
                continue;
            }
            Some(_) => continue,
            None => {
                if let Some(o) = opening_fence(line) {
                    fence = Some(o);
                    continue;
                }
            }
        }
        let trimmed = line.trim_end();
        if trimmed.starts_with("<<<<<<< ")
            || trimmed.starts_with(">>>>>>> ")
            || trimmed.starts_with("||||||| ")
            || trimmed == "======="
        {
            return true;
        }
    }
    false
}

/// 3-color DFS: each cycle reported once, rotated so the
/// lexicographically-smallest slug appears first. Adding a `visited`
/// set (the "black" color) caps the work at O(V + E) for cycle
/// detection — without it, a dense DAG re-explores subtrees from
/// every starting node and degrades exponentially.
///
/// This is *not* full Johnson's enumeration: we report at least one
/// cycle per strongly-connected component, not every elementary
/// cycle inside it. Doctor only needs to flag that cycles exist;
/// listing every elementary cycle adds nothing actionable.
pub(crate) fn detect_cycles(graph: &BTreeMap<String, Vec<String>>) -> Vec<Vec<String>> {
    let mut found: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut stack: Vec<String> = Vec::new();
    let mut on_stack: BTreeSet<String> = BTreeSet::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();

    fn dfs(
        node: &str,
        graph: &BTreeMap<String, Vec<String>>,
        stack: &mut Vec<String>,
        on_stack: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        found: &mut BTreeSet<Vec<String>>,
    ) {
        stack.push(node.to_string());
        on_stack.insert(node.to_string());
        if let Some(neigh) = graph.get(node) {
            for n in neigh {
                if on_stack.contains(n) {
                    let start = stack.iter().position(|s| s == n).unwrap();
                    let cycle: Vec<String> = stack[start..].to_vec();
                    let min_idx = cycle
                        .iter()
                        .enumerate()
                        .min_by(|a, b| a.1.cmp(b.1))
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    let mut rotated: Vec<String> = cycle[min_idx..].to_vec();
                    rotated.extend_from_slice(&cycle[..min_idx]);
                    found.insert(rotated);
                } else if graph.contains_key(n) && !visited.contains(n) {
                    dfs(n, graph, stack, on_stack, visited, found);
                }
            }
        }
        stack.pop();
        on_stack.remove(node);
        visited.insert(node.to_string());
    }

    for node in graph.keys() {
        if !visited.contains(node) {
            dfs(
                node,
                graph,
                &mut stack,
                &mut on_stack,
                &mut visited,
                &mut found,
            );
        }
    }
    found.into_iter().collect()
}

pub(crate) fn populate_notes_migration(scan: &ScanResult, report: &mut DoctorFindings) {
    for s in &scan.issues {
        if s.folder != "flat" || !s.item_present {
            continue;
        }
        let Some(text) = s.text.as_deref() else {
            continue;
        };
        match classify_notes(text) {
            NotesScan::NoOp => {}
            // Both SafeRename and Merge are forward-fixable by
            // `migrate_notes_heading`; the apply pass re-classifies and
            // routes each to a rename or a merge.
            NotesScan::SafeRename | NotesScan::Merge => {
                report.notes_to_rename.push(s.dir_name.clone())
            }
            NotesScan::Conflict => report.notes_conflicts.push(s.dir_name.clone()),
        }
    }
}

pub(crate) fn populate_schema_violations(
    scan: &ScanResult,
    repo_root: &Path,
    schema: &schema::Schema,
    report: &mut DoctorFindings,
) {
    for s in &scan.issues {
        if !s.item_present {
            continue;
        }
        // Skip dirs queued for the NN-rename pipeline — `--fix`
        // rewrites their frontmatter wholesale, so flagging schema
        // violations on them is noise that would refuse the very
        // fix designed to heal them. The typed signal is
        // `legacy_number.is_some()`, set by `legacy_number_from_mapping`
        // when frontmatter has neither `number:` nor `slug:` and the
        // dirname parses as `<NN>-<rest>`. This applies regardless of
        // folder: pre-migration a numbered-legacy lives under
        // `issues/{open,closed}/`, but after the flat-layout migration
        // moves it up, the same dir lives at `issues/<NN>-<rest>/`
        // pending NN-rename. A user-named flat slug like
        // `12-things-to-do` carries `slug:` in frontmatter (written
        // by `issuectl new`) and so does NOT trip this skip.
        if s.legacy_number.is_some() {
            continue;
        }
        let location = format!(
            "{}",
            s.item_path
                .strip_prefix(repo_root)
                .unwrap_or(&s.item_path)
                .display()
        );
        if let Some(err) = &s.read_error {
            report.parse_errors.push(ParseError {
                location,
                message: err.clone(),
                severity: ParseSeverity::Hard,
            });
            continue;
        }
        let Some(parsed) = s.parsed.as_ref() else {
            continue;
        };
        if parsed.fm_missing {
            report.parse_errors.push(ParseError {
                location,
                message: "missing or unterminated frontmatter".into(),
                severity: ParseSeverity::Hard,
            });
            continue;
        }
        if let Some(err) = &parsed.fm_yaml_error {
            report.parse_errors.push(ParseError {
                location,
                message: err.clone(),
                severity: ParseSeverity::Hard,
            });
            continue;
        }
        let Some(fm) = parsed.mapping.as_ref() else {
            continue;
        };
        for v in schema::validate(schema, fm) {
            // The built-in `closed` required-when-closing rule is
            // surfaced by the lifecycle-aware status/closed consistency
            // check in `populate_extended_validation`; suppress only
            // THAT specific violation here so the same condition isn't
            // reported twice. Any OTHER `required_when` (a user-declared
            // conditional field) has no other reporting channel, so it
            // must flow through to `schema_violations`.
            if let schema::ViolationKind::RequiredWhen { field, .. } = &v {
                if field == "closed" {
                    continue;
                }
            }
            // An enum violation on a value `doctor --fix` would coerce
            // is reported as a pending coercion instead of a violation.
            if let schema::ViolationKind::InvalidEnum { field, value, .. } = &v {
                if schema::would_coerce(schema, field, value).is_some() {
                    continue;
                }
            }
            report
                .schema_violations
                .push((location.clone(), v.message()));
        }
    }
}

/// Detect legacy `status` / `type` values that map to a canonical value
/// via the schema's alias tables. Records them as pending coercions so
/// the read-only report shows the planned rewrite and `--fix` applies
/// it. Skips dirs queued for the NN-rename pipeline (same typed
/// `legacy_number.is_some()` signal as `populate_schema_violations`).
pub(crate) fn populate_alias_coercions(
    scan: &ScanResult,
    schema: &schema::Schema,
    report: &mut DoctorFindings,
) {
    for s in &scan.issues {
        if !s.item_present || s.legacy_number.is_some() {
            continue;
        }
        let Some(fm) = s.parsed.as_ref().and_then(|p| p.mapping.as_ref()) else {
            continue;
        };
        for field in ["status", "type"] {
            let Some(value) = fm
                .get(serde_yaml::Value::String(field.into()))
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
            else {
                continue;
            };
            if let Some(to) = schema::would_coerce(schema, field, value) {
                report.alias_coercions.push((
                    s.dir_name.clone(),
                    field.to_string(),
                    value.to_string(),
                    to.to_string(),
                    s.item_path.clone(),
                ));
            }
        }
    }
    report.alias_coercions.sort();
}

pub(crate) fn populate_transition_warnings(
    scan: &ScanResult,
    rules: Option<&crate::transitions::TransitionRules>,
    schema: Option<&schema::Schema>,
    report: &mut DoctorFindings,
) {
    let rules_active = rules.map(|r| !r.status_rules.is_empty()).unwrap_or(false);
    let sections_active = schema.map(|s| !s.body_sections.is_empty()).unwrap_or(false);
    if !rules_active && !sections_active {
        return;
    }
    for s in &scan.issues {
        if !s.item_present {
            continue;
        }
        // Mirror the typed `legacy_number.is_some()` skip used by
        // `populate_schema_violations` and `populate_extended_validation`
        // — a numbered-legacy lifted to flat by phase 5 is the same
        // issue pending NN-rename; transition warnings on it would
        // refuse the very fix designed to heal it.
        if s.legacy_number.is_some() {
            continue;
        }
        let Some(parsed) = s.parsed.as_ref() else {
            continue;
        };
        // S5: only skip when essential frontmatter is absent. A legacy
        // numeric-epic ref produces a warning but leaves `status` /
        // `type` intact, so the lint can still run usefully.
        if essential_frontmatter_absent_from_mapping(parsed.mapping.as_ref()) {
            continue;
        }
        let issue = &parsed.issue;
        if let Some(rules) = rules {
            for msg in crate::transitions::evaluate_existing(rules, issue) {
                report.transition_warnings.push((issue.slug.clone(), msg));
            }
        }
        if let Some(sch) = schema {
            for missing in schema::missing_body_sections(sch, &issue.issue_type, &issue.body) {
                report
                    .missing_body_sections
                    .push((issue.slug.clone(), missing));
            }
        }
    }
}

pub(crate) fn essential_frontmatter_absent_from_mapping(
    mapping: Option<&serde_yaml::Mapping>,
) -> bool {
    let Some(m) = mapping else { return true };
    let has = |k: &str| {
        m.get(serde_yaml::Value::String(k.into()))
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    };
    !has("status") || !has("type")
}
