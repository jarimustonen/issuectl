use super::*;

pub(crate) fn render_text(
    report: &DoctorFindings,
    outcome: Option<&ApplyOutcome>,
    fix: bool,
    verbose: bool,
) {
    let outcome_default = ApplyOutcome::default();
    let oc = outcome.unwrap_or(&outcome_default);
    let has_problems = !report.legacy_dirs.is_empty()
        || !report.inbox_drafts.is_empty()
        || !oc.inbox_drafts_migrated.is_empty()
        || !planned_moves(report).is_empty()
        || !oc.flat_layout_migrated.is_empty()
        || !report.flat_layout_conflicts.is_empty()
        || !report.invalid_slugs.is_empty()
        || !report.duplicate_slugs.is_empty()
        || !report.missing_item_md.is_empty()
        || !report.orphan_epic_refs.is_empty()
        || !report.parse_errors.is_empty()
        || !oc.notes_renamed.is_empty()
        || !report.notes_to_rename.is_empty()
        || !report.notes_conflicts.is_empty()
        || !report.schema_violations.is_empty()
        || !report.alias_coercions.is_empty()
        || !oc.alias_coercions_applied.is_empty()
        || report.schema_parse_error.is_some()
        || !report.broken_refs.is_empty()
        || !report.blocked_by_cycles.is_empty()
        || !report.blocked_by_self.is_empty()
        || !report.status_consistency.is_empty()
        || !report.timestamp_issues.is_empty()
        || !report.unknown_keys.is_empty()
        || !report.unknown_reviewers.is_empty()
        || !report.conflict_markers.is_empty()
        || !report.orphan_tempfiles.is_empty()
        || !oc.orphan_tempfiles_removed.is_empty()
        || !report.symlinked_dirs.is_empty()
        || !report.both_open_and_closed.is_empty()
        || !report.closed_with_active_status.is_empty()
        || !report.open_with_closing_status.is_empty()
        || !oc.status_reconciled.is_empty()
        || !report.transition_warnings.is_empty()
        || !report.missing_body_sections.is_empty()
        || report.agents_md_drift
        || report.agents_md_malformed.is_some()
        || report.agents_md_check_skipped.is_some()
        || oc.agents_md_regenerated
        || report.agents_md_missing
        || report.legacy_issues_agents_md
        || !report.gitignored_paths.is_empty()
        || oc.issues_agents_md_rewritten
        || !report.large_binaries.is_empty()
        || !report.non_avif_images.is_empty()
        || !report.broken_attachment_refs.is_empty()
        || !report.deferred_labels.is_empty()
        || !oc.deferred_labels_removed.is_empty()
        || !oc.blockers.is_empty()
        || oc.apply_error.is_some();
    if !oc.blockers.is_empty() {
        println!("doctor: cannot safely apply --fix until these issues are resolved:");
        for b in &oc.blockers {
            println!("  - {b}");
        }
        println!();
    }
    if let Some(err) = &oc.apply_error {
        println!("doctor: --fix aborted mid-pipeline; partial progress retained:");
        println!("  {err}");
        println!();
    }
    if !has_problems {
        if report.schema_missing {
            println!(
                "Repository OK — no migrations or fixes needed.\nNote: {} not present yet (will be auto-created on first write or `--fix`).",
                schema::SCHEMA_RELATIVE_PATH
            );
        } else {
            println!("Repository OK — no migrations or fixes needed.");
        }
        return;
    }

    if !oc.inbox_drafts_migrated.is_empty() {
        println!("Migrated deprecated inbox drafts to the flat layout:");
        for draft in &oc.inbox_drafts_migrated {
            println!(
                "  {}  ({} → {})",
                draft.slug,
                draft.from.display(),
                draft.to.display()
            );
        }
        println!();
    } else if !report.inbox_drafts.is_empty() {
        println!("Deprecated inbox drafts (re-run with --fix to migrate):");
        for (slug, _) in &report.inbox_drafts {
            println!("  {slug}");
        }
        println!();
    }

    if !oc.flat_layout_migrated.is_empty() {
        print_section(
            "Migrated to flat layout:",
            &oc.flat_layout_migrated,
            verbose,
            "issue(s) migrated to flat layout",
            |m| format!("{}  ({} → {})", m.slug, m.from.display(), m.to.display()),
        );
    } else {
        let planned: Vec<_> = planned_moves(report).into_iter().collect();
        print_section(
            "Issues still in legacy `issues/{open,closed}/<slug>/` layout:",
            &planned,
            verbose,
            "issue(s) still in legacy `issues/{open,closed}/<slug>/` layout — re-run with --fix",
            |m| {
                format!(
                    "{}  ({} → {})",
                    m.slug(),
                    m.from().display(),
                    m.to().display()
                )
            },
        );
    }
    if !report.flat_layout_conflicts.is_empty() {
        println!("Flat-layout migration conflicts:");
        for c in &report.flat_layout_conflicts {
            println!("  {}: {}", c.slug, c.detail);
        }
        println!();
    }

    if !report.legacy_dirs.is_empty() {
        let title = if fix {
            "Migrated legacy numbered issues:"
        } else {
            "Legacy numbered issues to migrate:"
        };
        let collapsed_phrase = if fix {
            "legacy numbered issue(s) migrated"
        } else {
            "legacy numbered issue(s) to migrate — re-run with --fix"
        };
        // Legacy <NN>-<slug> dirs are migrated to the canonical flat
        // path post-flat-layout; print the actual destination rather
        // than the (incorrect) "{folder}/{new}" pre-flat shape.
        print_section(title, &report.legacy_dirs, verbose, collapsed_phrase, |m| {
            format!(
                "{}/{}  →  {}",
                m.folder,
                m.old_dir_name,
                m.new_path.display()
            )
        });
    }
    if !report.invalid_slugs.is_empty() {
        println!("Slugs failing is_valid():");
        for s in &report.invalid_slugs {
            println!("  {s}");
        }
        println!();
    }
    if !report.duplicate_slugs.is_empty() {
        println!("Duplicate slugs (would-be after migration):");
        for s in &report.duplicate_slugs {
            println!("  {s}");
        }
        println!();
    }
    if !report.missing_item_md.is_empty() {
        println!("Directories missing item.md:");
        for s in &report.missing_item_md {
            println!("  {s}");
        }
        println!();
    }
    if !report.orphan_epic_refs.is_empty() {
        println!("Orphan epic references:");
        for (slug, epic) in &report.orphan_epic_refs {
            println!("  {slug} → epic: {epic}");
        }
        println!();
    }
    if !report.parse_errors.is_empty() {
        println!("Parse warnings:");
        for e in &report.parse_errors {
            println!("  {}: {}", e.location, e.message);
        }
        println!();
    }
    if !report.notes_to_rename.is_empty() {
        println!("`## Notes` sections to rename to `## Comments`:");
        for s in &report.notes_to_rename {
            println!("  {s}");
        }
        println!();
    }
    if !oc.notes_renamed.is_empty() {
        println!("Renamed `## Notes` → `## Comments`:");
        for s in &oc.notes_renamed {
            println!("  {s}");
        }
        println!();
    }
    if !report.notes_conflicts.is_empty() {
        println!("Files with both `## Notes` and `## Comments` (manual merge required):");
        for s in &report.notes_conflicts {
            println!("  {s}");
        }
        println!();
    }
    if report.schema_missing {
        println!(
            "Schema file missing at {} (will be auto-created on first `--fix` or write).",
            schema::SCHEMA_RELATIVE_PATH
        );
        println!();
    }
    if let Some(err) = &report.schema_parse_error {
        println!("Schema file parse error: {err}");
        println!();
    }
    print_section(
        "Schema violations:",
        &report.schema_violations,
        verbose,
        "schema violation(s)",
        |(loc, msg)| format!("{loc}: {msg}"),
    );
    if !oc.alias_coercions_applied.is_empty() {
        println!("Coerced legacy values via schema aliases:");
        for (slug, field, from, to) in &oc.alias_coercions_applied {
            println!("  {slug}: {field} {from} → {to}");
        }
        println!();
    } else if !report.alias_coercions.is_empty() {
        println!("Legacy values to coerce via schema aliases (re-run with --fix):");
        for (slug, field, from, to, _) in &report.alias_coercions {
            println!("  {slug}: {field} {from} → {to}");
        }
        println!();
    }
    print_section(
        "Broken cross-references:",
        &report.broken_refs,
        verbose,
        "broken cross-reference(s)",
        |(slug, kind, target)| format!("{slug}: {kind} → {target}"),
    );
    if !report.blocked_by_cycles.is_empty() {
        println!("Dependency cycles via `blocked_by`:");
        for cycle in &report.blocked_by_cycles {
            println!("  {} → {}", cycle.join(" → "), cycle[0]);
        }
        println!();
    }
    if !report.blocked_by_self.is_empty() {
        println!("Self-dependencies in `blocked_by`:");
        for slug in &report.blocked_by_self {
            println!("  {slug}: lists itself as a blocker");
        }
        println!();
    }
    if !report.status_consistency.is_empty() {
        println!("Status / closed-date inconsistencies:");
        for (slug, msg) in &report.status_consistency {
            println!("  {slug}: {msg}");
        }
        println!();
    }
    if !report.timestamp_issues.is_empty() {
        println!("Timestamp sanity issues:");
        for (slug, msg) in &report.timestamp_issues {
            println!("  {slug}: {msg}");
        }
        println!();
    }
    if !report.unknown_keys.is_empty() {
        println!("Unknown frontmatter keys (not declared by schema):");
        for (slug, key) in &report.unknown_keys {
            println!("  {slug}: {key}");
        }
        println!();
    }
    if !report.unknown_reviewers.is_empty() {
        println!("Unknown reviewers (not present as reporter/assignee/owner anywhere):");
        for (slug, who) in &report.unknown_reviewers {
            println!("  {slug}: {who}");
        }
        println!();
    }
    if !report.conflict_markers.is_empty() {
        println!("Files with git merge-conflict markers (manual fix required):");
        for s in &report.conflict_markers {
            println!("  {s}");
        }
        println!();
    }
    if !oc.orphan_tempfiles_removed.is_empty() {
        println!("Removed orphan tempfiles:");
        for p in &oc.orphan_tempfiles_removed {
            println!("  {}", p.display());
        }
        println!();
    } else if !report.orphan_tempfiles.is_empty() {
        println!("Orphan `.issuectl-tmp-*` files:");
        for p in &report.orphan_tempfiles {
            println!("  {}", p.display());
        }
        println!();
    }
    if !report.symlinked_dirs.is_empty() {
        println!("Symlinked issue directories (refused):");
        for s in &report.symlinked_dirs {
            println!("  {s}");
        }
        println!();
    }
    if !report.both_open_and_closed.is_empty() {
        println!(
            "Slugs present in BOTH `issues/open/` and `issues/closed/` (manual fix required):"
        );
        for s in &report.both_open_and_closed {
            println!("  {s}");
        }
        println!();
    }
    if !report.closed_with_active_status.is_empty() {
        println!("`closed/<slug>` with active status:");
        for (slug, st, _) in &report.closed_with_active_status {
            println!("  {slug} (status: {st})");
        }
        println!();
    }
    if !report.open_with_closing_status.is_empty() {
        println!("`open/<slug>` with closing status:");
        for (slug, st, _) in &report.open_with_closing_status {
            println!("  {slug} (status: {st})");
        }
        println!();
    }
    if !oc.status_reconciled.is_empty() {
        println!("Reconciled status/folder mismatches:");
        for s in &oc.status_reconciled {
            println!("  {s}");
        }
        println!();
    }
    if !report.transition_warnings.is_empty() {
        println!("Transition-rule warnings (warning-only — legacy data may pre-date the rules):");
        for (slug, msg) in &report.transition_warnings {
            println!("  {slug}: {msg}");
        }
        println!();
    }
    if !report.missing_body_sections.is_empty() {
        println!("Missing required body sections:");
        for (slug, section) in &report.missing_body_sections {
            println!("  {slug}: ## {section}");
        }
        println!();
    }
    if let Some(reason) = &report.agents_md_malformed {
        println!(
            "{} is malformed: {} — fix manually before re-running --fix.",
            agents::AGENTS_RELATIVE_PATH,
            reason
        );
        println!();
    }
    if let Some(err) = &report.agents_md_check_skipped {
        println!(
            "{} drift check skipped: {} (fix the schema/rules file first).",
            agents::AGENTS_RELATIVE_PATH,
            err
        );
        println!();
    }
    if report.agents_md_missing {
        println!(
            "{} not present — run `issuectl agents init` to opt in to the agent policy file.",
            agents::AGENTS_RELATIVE_PATH
        );
        println!();
    }
    if !report.gitignored_paths.is_empty() {
        for p in &report.gitignored_paths {
            println!(
                "{p} is gitignored — agents on other machines won't see it. Adjust .gitignore or move the file."
            );
        }
        println!();
    }
    if oc.issues_agents_md_rewritten {
        println!(
            "Rewrote stale issues/AGENTS.md (pre-v0.5.0 scaffold) with current pointer template."
        );
        println!();
    } else if report.legacy_issues_agents_md {
        println!(
            "issues/AGENTS.md still carries the pre-v0.5.0 scaffold (re-run with --fix to replace)."
        );
        println!();
    }
    if oc.agents_md_regenerated {
        println!(
            "Regenerated schema-derived block in {}.",
            agents::AGENTS_RELATIVE_PATH
        );
        println!();
    } else if report.agents_md_drift {
        println!(
            "{} schema-derived block is out of date (re-run with --fix to regenerate).",
            agents::AGENTS_RELATIVE_PATH
        );
        println!();
    }
    if !report.large_binaries.is_empty() {
        println!("Large binaries under issue dirs (consider external storage or .gitignore):");
        for (slug, path, bytes) in &report.large_binaries {
            println!("  {slug}: {path} ({} KiB)", bytes / 1024);
        }
        println!();
    }
    if !report.non_avif_images.is_empty() {
        println!("Non-AVIF images (convert to AVIF per the issue convention):");
        for (slug, path) in &report.non_avif_images {
            println!("  {slug}: {path}");
        }
        println!();
    }
    if !report.broken_attachment_refs.is_empty() {
        println!("Body references to missing files (relative paths that don't resolve):");
        for (slug, r) in &report.broken_attachment_refs {
            println!("  {slug}: {r}");
        }
        println!();
    }
    if !oc.deferred_labels_removed.is_empty() {
        println!("Removed retired `deferred` labels:");
        for slug in &oc.deferred_labels_removed {
            println!("  {slug}");
        }
        println!();
    }
    if !report.deferred_labels.is_empty() {
        println!("Retired `deferred` labels (remove them; re-run with --fix):");
        for (slug, _) in &report.deferred_labels {
            if let Some((_, reason)) = report
                .deferred_labels_require_intake_migrate
                .iter()
                .find(|(pending, _)| pending == slug)
            {
                println!("  {slug}: {reason}");
            } else {
                println!("  {slug}");
            }
        }
        println!();
    }
    if fix {
        // Coherent end-of-run summary. Previously every `--fix` run
        // printed an `Applied. …` count line even when the pipeline
        // had refused to mutate at preflight, or when unfixable
        // findings remained (issue: @doctor-fix-noop). Prefix
        // follows `stop_phase` first, then falls back to whether the
        // post-apply scan still surfaces critical findings.
        println!("{}", fix_summary(report, oc));
    } else {
        println!("Read-only — re-run with --fix to apply.");
    }
}

pub(crate) fn render_json(
    report: &DoctorFindings,
    outcome: Option<&ApplyOutcome>,
    fix: bool,
    repo_root: &Path,
) -> serde_json::Value {
    let outcome_default = ApplyOutcome::default();
    let oc = outcome.unwrap_or(&outcome_default);
    let inbox_drafts: Vec<serde_json::Value> = report
        .inbox_drafts
        .iter()
        .map(|(slug, path)| serde_json::json!({"slug": slug, "dir": rel(repo_root, path)}))
        .collect();
    let inbox_drafts_migrated: Vec<serde_json::Value> = oc
        .inbox_drafts_migrated
        .iter()
        .map(|draft| {
            serde_json::json!({
                "slug": draft.slug,
                "from": rel(repo_root, &draft.from),
                "to": rel(repo_root, &draft.to),
            })
        })
        .collect();
    let migrated_legacy: Vec<serde_json::Value> = oc
        .legacy_dirs_migrated
        .iter()
        .map(|m| {
            serde_json::json!({
                "folder": m.folder,
                "old_dir": m.old_dir_name,
                "old_number": m.old_number,
                "new_slug": m.new_slug,
            })
        })
        .collect();
    let migrations: Vec<serde_json::Value> = if !oc.legacy_dirs_migrated.is_empty() {
        migrated_legacy.clone()
    } else {
        report
            .legacy_dirs
            .iter()
            .map(|m| {
                serde_json::json!({
                    "folder": m.folder,
                    "old_dir": m.old_dir_name,
                    "old_number": m.old_number,
                    "new_slug": m.new_slug,
                })
            })
            .collect()
    };

    let orphans: Vec<serde_json::Value> = report
        .orphan_epic_refs
        .iter()
        .map(|(s, e)| serde_json::json!({"slug": s, "epic": e}))
        .collect();

    let parse_errors: Vec<serde_json::Value> = report
        .parse_errors
        .iter()
        .map(|e| serde_json::json!({"location": e.location, "message": e.message}))
        .collect();

    let flat_layout_planned: Vec<serde_json::Value> = planned_moves(report)
        .iter()
        .map(|m| {
            serde_json::json!({
                "slug": m.slug(),
                "from": rel(repo_root, m.from()),
                "to": rel(repo_root, m.to()),
            })
        })
        .collect();
    let flat_layout_migrated: Vec<serde_json::Value> = oc
        .flat_layout_migrated
        .iter()
        .map(|m| {
            serde_json::json!({
                "slug": m.slug,
                "from": rel(repo_root, &m.from),
                "to": rel(repo_root, &m.to),
            })
        })
        .collect();
    let flat_layout_conflicts: Vec<serde_json::Value> = report
        .flat_layout_conflicts
        .iter()
        .map(|c| serde_json::json!({"slug": c.slug, "detail": c.detail}))
        .collect();

    let schema_violations: Vec<serde_json::Value> = report
        .schema_violations
        .iter()
        .map(|(loc, msg)| serde_json::json!({"location": loc, "message": msg}))
        .collect();
    let alias_coercions: Vec<serde_json::Value> = report
        .alias_coercions
        .iter()
        .map(|(slug, field, from, to, _)| {
            serde_json::json!({"slug": slug, "field": field, "from": from, "to": to})
        })
        .collect();

    let broken_refs: Vec<serde_json::Value> = report
        .broken_refs
        .iter()
        .map(|(slug, kind, target)| {
            serde_json::json!({"slug": slug, "kind": kind, "target": target})
        })
        .collect();
    let status_consistency: Vec<serde_json::Value> = report
        .status_consistency
        .iter()
        .map(|(s, m)| serde_json::json!({"slug": s, "message": m}))
        .collect();
    let timestamp_issues: Vec<serde_json::Value> = report
        .timestamp_issues
        .iter()
        .map(|(s, m)| serde_json::json!({"slug": s, "message": m}))
        .collect();
    let unknown_keys: Vec<serde_json::Value> = report
        .unknown_keys
        .iter()
        .map(|(s, k)| serde_json::json!({"slug": s, "key": k}))
        .collect();
    let unknown_reviewers: Vec<serde_json::Value> = report
        .unknown_reviewers
        .iter()
        .map(|(s, r)| serde_json::json!({"slug": s, "reviewer": r}))
        .collect();
    let orphan_tempfiles: Vec<String> = report
        .orphan_tempfiles
        .iter()
        .map(|p| rel(repo_root, p))
        .collect();
    let orphan_tempfiles_removed: Vec<String> = oc
        .orphan_tempfiles_removed
        .iter()
        .map(|p| rel(repo_root, p))
        .collect();
    let closed_with_active: Vec<serde_json::Value> = report
        .closed_with_active_status
        .iter()
        .map(|(s, st, _)| serde_json::json!({"slug": s, "status": st}))
        .collect();
    let open_with_closing: Vec<serde_json::Value> = report
        .open_with_closing_status
        .iter()
        .map(|(s, st, _)| serde_json::json!({"slug": s, "status": st}))
        .collect();

    let large_binaries: Vec<serde_json::Value> = report
        .large_binaries
        .iter()
        .map(|(slug, path, bytes)| serde_json::json!({"slug": slug, "path": path, "bytes": bytes}))
        .collect();
    let non_avif_images: Vec<serde_json::Value> = report
        .non_avif_images
        .iter()
        .map(|(slug, path)| serde_json::json!({"slug": slug, "path": path}))
        .collect();
    let broken_attachment_refs: Vec<serde_json::Value> = report
        .broken_attachment_refs
        .iter()
        .map(|(slug, r)| serde_json::json!({"slug": slug, "ref": r}))
        .collect();
    let deferred_labels: Vec<String> = report
        .deferred_labels
        .iter()
        .map(|(slug, _)| slug.clone())
        .collect();
    let deferred_labels_require_intake_migrate: Vec<serde_json::Value> = report
        .deferred_labels_require_intake_migrate
        .iter()
        .map(|(slug, reason)| serde_json::json!({"slug": slug, "reason": reason}))
        .collect();

    let mut json_obj = serde_json::json!({
        "fix_applied": fix && oc.fix_applied(),
        "migrations": migrations,
        "flat_layout_planned": flat_layout_planned,
        "flat_layout_migrated": flat_layout_migrated,
        "flat_layout_conflicts": flat_layout_conflicts,
        "invalid_slugs": report.invalid_slugs,
        "duplicate_slugs": report.duplicate_slugs,
        "missing_item_md": report.missing_item_md,
        "orphan_epic_refs": orphans,
        "parse_errors": parse_errors,
        "schema_missing": report.schema_missing,
        "schema_parse_error": report.schema_parse_error,
        "schema_violations": schema_violations,
        "alias_coercions": alias_coercions,
        "files_rewritten": oc.files_rewritten,
        "notes_to_rename": report.notes_to_rename,
        "notes_renamed": oc.notes_renamed,
        "notes_conflicts": report.notes_conflicts,
        "broken_refs": broken_refs,
        "blocked_by_cycles": report.blocked_by_cycles,
        "status_consistency": status_consistency,
        "timestamp_issues": timestamp_issues,
        "unknown_keys": unknown_keys,
        "conflict_markers": report.conflict_markers,
        "orphan_tempfiles": orphan_tempfiles,
        "orphan_tempfiles_removed": orphan_tempfiles_removed,
        "symlinked_dirs": report.symlinked_dirs,
        "both_open_and_closed": report.both_open_and_closed,
        "closed_with_active_status": closed_with_active,
        "open_with_closing_status": open_with_closing,
        "status_reconciled": oc.status_reconciled,
        "transition_warnings": report
            .transition_warnings
            .iter()
            .map(|(s, m)| serde_json::json!({"slug": s, "message": m}))
            .collect::<Vec<_>>(),
        "missing_body_sections": report
            .missing_body_sections
            .iter()
            .map(|(s, sec)| serde_json::json!({"slug": s, "section": sec}))
            .collect::<Vec<_>>(),
        "agents_md_drift": report.agents_md_drift,
        "agents_md_malformed": report.agents_md_malformed,
        "agents_md_check_skipped": report.agents_md_check_skipped,
        "agents_md_regenerated": oc.agents_md_regenerated,
        "agents_md_missing": report.agents_md_missing,
        "legacy_issues_agents_md": report.legacy_issues_agents_md,
        "issues_agents_md_rewritten": oc.issues_agents_md_rewritten,
        "gitignored_paths": report.gitignored_paths,
    });
    // Inserted post-construction rather than inline: the read-only object
    // literal is already at the `serde_json::json!` macro recursion
    // limit, so more inline keys overflow it. Map is a sorted
    // BTreeMap, so insertion order does not affect the rendered output.
    if let serde_json::Value::Object(map) = &mut json_obj {
        map.insert(
            "inbox_drafts".to_string(),
            serde_json::Value::Array(inbox_drafts),
        );
        map.insert(
            "inbox_drafts_migrated".to_string(),
            serde_json::Value::Array(inbox_drafts_migrated.clone()),
        );
        map.insert(
            "large_binaries".to_string(),
            serde_json::Value::Array(large_binaries),
        );
        map.insert(
            "non_avif_images".to_string(),
            serde_json::Value::Array(non_avif_images),
        );
        map.insert(
            "broken_attachment_refs".to_string(),
            serde_json::Value::Array(broken_attachment_refs),
        );
        map.insert(
            "unknown_reviewers".to_string(),
            serde_json::Value::Array(unknown_reviewers),
        );
        map.insert(
            "blocked_by_self".to_string(),
            serde_json::json!(report.blocked_by_self),
        );
        map.insert(
            "deferred_labels".to_string(),
            serde_json::json!(deferred_labels),
        );
        map.insert(
            "deferred_labels_removed".to_string(),
            serde_json::json!(oc.deferred_labels_removed),
        );
        map.insert(
            "deferred_labels_require_intake_migrate".to_string(),
            serde_json::json!(deferred_labels_require_intake_migrate),
        );
    }
    // `apply_outcome` is the new structured envelope: emitted only on
    // `--fix` runs so the read-only JSON shape (golden snapshot) stays
    // byte-identical. Carries `fix_applied` (computed from the outcome
    // alone — no early-return path can lie about it), the preflight
    // `blockers` list (which makes `--json --fix` a structured bail
    // instead of an anyhow text on stderr), and a rollup of every
    // applied-action variant for scripts that prefer reading one
    // sub-object instead of N top-level keys.
    if fix {
        if let serde_json::Value::Object(map) = &mut json_obj {
            map.insert(
                "apply_outcome".to_string(),
                serde_json::json!({
                    "fix_applied": oc.fix_applied(),
                    "stop_phase": oc.stop_phase.as_str(),
                    "blockers": oc.blockers,
                    "schema_bootstrapped": oc.schema_bootstrapped,
                    "inbox_drafts_migrated": inbox_drafts_migrated,
                    "agents_md_regenerated": oc.agents_md_regenerated,
                    "issues_agents_md_rewritten": oc.issues_agents_md_rewritten,
                    "deferred_labels_removed": oc.deferred_labels_removed,
                    "files_rewritten": oc.files_rewritten,
                    "legacy_dirs_migrated": migrated_legacy,
                    "flat_layout_migrated": oc
                        .flat_layout_migrated
                        .iter()
                        .map(|m| {
                            serde_json::json!({
                                "slug": m.slug,
                                "from": rel(repo_root, &m.from),
                                "to": rel(repo_root, &m.to),
                            })
                        })
                        .collect::<Vec<_>>(),
                    "notes_renamed": oc.notes_renamed,
                    "notes_conflicts_at_apply": oc.notes_conflicts_at_apply,
                    "orphan_tempfiles_removed": oc
                        .orphan_tempfiles_removed
                        .iter()
                        .map(|p| rel(repo_root, p))
                        .collect::<Vec<_>>(),
                    "status_reconciled": oc.status_reconciled,
                    "alias_coercions_applied": oc
                        .alias_coercions_applied
                        .iter()
                        .map(|(slug, field, from, to)| {
                            serde_json::json!({"slug": slug, "field": field, "from": from, "to": to})
                        })
                        .collect::<Vec<_>>(),
                    "apply_error": oc.apply_error,
                }),
            );
        }
    }
    json_obj
}
