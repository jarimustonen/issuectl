use super::*;

pub(crate) fn cmd_intake_file(json: bool, req: mutate::intake::FileRequest) -> Result<()> {
    match mutate::intake::file(&find_root(), req) {
        Ok(out) => {
            if json {
                let report = serde_json::json!({
                    "slug": out.slug,
                    "status": "untriaged",
                    "dir": out.issue_dir.to_string_lossy(),
                    "version": out.version,
                    "deduplicated": out.deduplicated,
                });
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else if out.deduplicated {
                println!(
                    "Existing issue {} returned (deduplicated on source-ref)",
                    out.slug
                );
            } else {
                println!("Filed {} (untriaged)", out.slug);
            }
            Ok(())
        }
        Err(e) => fail(
            json,
            e.exit_code(),
            e.code(),
            &format!("{e}"),
            serde_json::Value::Null,
        ),
    }
}

/// Run the legacy-intake data migration (§6). Dry-run by default; `apply`
/// commits. The `--json` output is a single result object: conflicts are
/// reported in-band as skipped actions (no new error code, exit 0). A
/// per-issue **write failure** under `--apply` is reported on its action
/// and makes the command exit `1` — an ambiguous item is expected, a failed
/// write is not.
pub(crate) fn cmd_intake_migrate(json: bool, apply: bool) -> Result<()> {
    let report = match mutate::intake_migrate::migrate(&find_root(), apply) {
        Ok(r) => r,
        Err(e) => fail(
            json,
            e.exit_code(),
            e.code(),
            &format!("{e}"),
            serde_json::Value::Null,
        ),
    };
    let failed = report.failed_count();

    if json {
        let actions: Vec<serde_json::Value> = report
            .actions
            .iter()
            .map(|a| {
                let action = if a.conflict.is_some() {
                    "skip"
                } else if a.error.is_some() {
                    "error"
                } else {
                    "migrate"
                };
                serde_json::json!({
                    "slug": a.slug,
                    "action": action,
                    "conflict": a.conflict,
                    "error": a.error,
                    "status_change": a.status_change.as_ref().map(|(from, to)| {
                        serde_json::json!({ "from": from, "to": to })
                    }),
                    "dropped_labels": a.dropped_labels,
                    "set_provenance": a.set_provenance,
                    "warnings": a.warnings,
                    "applied": a.applied,
                })
            })
            .collect();
        // `migrated` is *planned* in dry-run and *applied* in apply mode
        // (successful, non-error). Name the mode explicitly so automation
        // never reads a dry-run plan as committed work.
        let out = serde_json::json!({
            "applied": report.applied,
            "summary": {
                "total": report.actions.len(),
                "migrated": report.migrated_count(),
                "skipped": report.skipped_count(),
                "failed": failed,
            },
            "actions": actions,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        if failed > 0 {
            std::process::exit(1);
        }
        return Ok(());
    }

    let mode = if report.applied {
        "applied"
    } else {
        "dry-run (nothing written)"
    };
    let verb = if report.applied {
        "migrated"
    } else {
        "to migrate"
    };
    if report.actions.is_empty() {
        println!("Intake migration — {mode}: no legacy items to migrate.");
        return Ok(());
    }
    println!(
        "Intake migration — {mode}: {} {verb}, {} skipped (conflict), {failed} failed:",
        report.migrated_count(),
        report.skipped_count()
    );
    for a in &report.actions {
        if let Some(reason) = &a.conflict {
            println!("  SKIP  {}  — {reason}", a.slug);
            continue;
        }
        if let Some(err) = &a.error {
            println!("  FAIL  {}  — {err}", a.slug);
            continue;
        }
        let mut parts: Vec<String> = Vec::new();
        if let Some((from, to)) = &a.status_change {
            parts.push(format!("status {from} → {to}"));
        }
        if !a.dropped_labels.is_empty() {
            parts.push(format!("drop label(s) {}", a.dropped_labels.join(", ")));
        }
        if let Some(p) = &a.set_provenance {
            parts.push(format!("provenance → {p}"));
        }
        println!(
            "  {}  {}  {}",
            if a.applied { "DONE" } else { "PLAN" },
            a.slug,
            parts.join("; ")
        );
        for w in &a.warnings {
            println!("        warn: {w}");
        }
    }
    if !report.applied {
        println!("\nRe-run with `issuectl intake migrate --apply` to write these changes.");
    }
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Name of the body section a read-only analysis worker appends to. Its
/// presence is the *derived* "has been analysed" signal — there is no
/// stored analysis state / lease (design OD-2).
const TRIAGE_ANALYSIS_SECTION: &str = "Triage analysis";

pub(crate) fn has_triage_analysis(body: &str) -> bool {
    body_sections::all_h2_sections(body).contains_key(TRIAGE_ANALYSIS_SECTION)
}

pub(crate) fn intake_provenance(issue: &models::Issue) -> Option<&str> {
    issue.extra.get("provenance").and_then(|v| v.as_str())
}

pub(crate) fn has_label(issue: &models::Issue, label: &str) -> bool {
    issue
        .labels
        .as_ref()
        .is_some_and(|ls| ls.iter().any(|l| l == label))
}

/// A legacy reception form the migration (§6) has not yet converted.
/// These items are deliberately excluded from queue rows because intake
/// transitions validate first-class status, but their count drives the
/// migration warning so they are not hidden silently.
pub(crate) fn is_legacy_intake_item(issue: &models::Issue) -> bool {
    issue.status == "open" && (has_label(issue, "needs-triage") || has_label(issue, "deferred"))
}

pub(crate) fn queue_provenance(issue: &models::Issue) -> Option<&str> {
    intake_provenance(issue).or_else(|| {
        let mut channels = issue.labels.iter().flatten().filter_map(|label| {
            label
                .strip_prefix("via:")
                .filter(|channel| !channel.is_empty())
        });
        let channel = channels.next()?;
        channels
            .all(|candidate| candidate == channel)
            .then_some(channel)
    })
}

pub(crate) fn cmd_intake_queue(
    json: bool,
    issue_type: Option<String>,
    provenance: Option<String>,
    needs_analysis: bool,
    state: Option<String>,
) -> Result<()> {
    // Default view is the actionable reception queue; `deferred` /
    // `needs-info` are excluded unless explicitly requested via --state.
    let target = state.as_deref().unwrap_or("untriaged");
    let issues = load();
    let legacy_count = issues
        .iter()
        .filter(|issue| is_legacy_intake_item(issue))
        .count();
    let legacy_warning = (legacy_count > 0).then(|| {
        format!(
            "{legacy_count} legacy label-based intake item(s) hidden from the status-based queue; run `issuectl intake migrate --apply` to admit them"
        )
    });
    // Queue rows are strict on first-class status so every listed item is
    // accepted by the corresponding intake transition.
    let mut rows: Vec<&models::Issue> = issues
        .iter()
        .filter(|i| i.status == target)
        .filter(|i| issue_type.as_deref().is_none_or(|t| i.issue_type == t))
        .filter(|i| {
            provenance
                .as_deref()
                .is_none_or(|p| queue_provenance(i) == Some(p))
        })
        .filter(|i| !needs_analysis || !has_triage_analysis(&i.body))
        .collect();
    // Stable order: oldest `created` first (items lacking a date sort
    // last), slug as the deterministic tiebreak.
    rows.sort_by(|a, b| {
        let ka = (a.created.is_none(), a.created.clone(), a.slug.clone());
        let kb = (b.created.is_none(), b.created.clone(), b.slug.clone());
        ka.cmp(&kb)
    });

    if json {
        let arr: Vec<serde_json::Value> = rows
            .iter()
            .map(|i| {
                serde_json::json!({
                    "slug": i.slug,
                    "type": i.issue_type,
                    "status": i.status,
                    "priority": i.priority,
                    "created": i.created,
                    "provenance": intake_provenance(i),
                    "reporter": i.reporter,
                    "title": i.title,
                    "needs_analysis": !has_triage_analysis(&i.body),
                    "legacy": false,
                    "version": canonical::canonical_hash(i),
                })
            })
            .collect();
        let obj = serde_json::json!({
            "state": target,
            "items": arr,
            "warnings": legacy_warning.into_iter().collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("No items in the {target} queue.");
    } else {
        println!(
            "Intake queue ({target}) — {} item(s), oldest first:",
            rows.len()
        );
        for i in &rows {
            let prov = queue_provenance(i).unwrap_or("-");
            let flag = if !has_triage_analysis(&i.body) {
                "  [needs-analysis]"
            } else {
                ""
            };
            println!(
                "  {}  {}  {}  ({})  {}{}",
                i.created.as_deref().unwrap_or("????-??-??"),
                i.issue_type,
                i.slug,
                prov,
                i.title,
                flag
            );
        }
    }
    if let Some(warning) = legacy_warning {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

pub(crate) fn cmd_intake_show(json: bool, slug: &str) -> Result<()> {
    let root = find_root();
    let resolved = match repo::resolve_slug_input(&root, slug) {
        Ok(s) => s,
        Err(e) => fail(
            json,
            1,
            "ambiguous-slug",
            &format!("{e:#}"),
            serde_json::Value::Null,
        ),
    };
    let issues = load();
    let Some(issue) = issues.iter().find(|i| i.slug == resolved) else {
        fail(
            json,
            1,
            "not-found",
            &format!("issue {slug} not found"),
            serde_json::Value::Null,
        )
    };

    // Attachments live under `<issue-dir>/attachments/`.
    let attachments: Vec<String> = repo::locate_issue_full(&root, &issue.slug)
        .ok()
        .and_then(|l| l.item_path.parent().map(Path::to_path_buf))
        .map(|dir| {
            let adir = dir.join(mutate::new_issue::ATTACHMENTS_DIRNAME);
            fs::read_dir(&adir)
                .map(|rd| {
                    let mut names: Vec<String> = rd
                        .flatten()
                        .filter_map(|e| e.file_name().into_string().ok())
                        .collect();
                    names.sort();
                    names
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let analysis = body_sections::all_h2_sections(&issue.body)
        .get(TRIAGE_ANALYSIS_SECTION)
        .cloned();

    if json {
        let mut v = serde_json::to_value(issue).expect("Issue serializes");
        if let serde_json::Value::Object(ref mut m) = v {
            m.insert(
                "version".into(),
                serde_json::Value::String(canonical::canonical_hash(issue)),
            );
            m.insert(
                "attachments".into(),
                serde_json::to_value(&attachments).unwrap(),
            );
            m.insert(
                "analysis".into(),
                match &analysis {
                    Some(a) => serde_json::Value::String(a.clone()),
                    None => serde_json::Value::Null,
                },
            );
        }
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }

    print_issue_detail(issue);
    if !attachments.is_empty() {
        println!("\nAttachments:");
        for a in &attachments {
            println!("  {a}");
        }
    }
    match analysis {
        Some(a) => println!("\n## {TRIAGE_ANALYSIS_SECTION}\n{a}"),
        None => println!("\n(no {TRIAGE_ANALYSIS_SECTION} section yet)"),
    }
    Ok(())
}
