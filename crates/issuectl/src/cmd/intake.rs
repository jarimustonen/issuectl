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

/// A legacy reception form the migration (§6) has not yet converted, for the
/// queue `target` being viewed. `open + needs-triage` is the legacy
/// `untriaged`; `open + deferred` is the legacy `deferred`. Surfacing these
/// keeps the pre-migration population from being silently abandoned — in
/// whichever queue view the item *would* land after migration. Other targets
/// (e.g. `needs-info`) have no legacy label form.
pub(crate) fn is_legacy_for(issue: &models::Issue, target: &str) -> bool {
    issue.status == "open"
        && match target {
            "untriaged" => has_label(issue, "needs-triage"),
            "deferred" => has_label(issue, "deferred"),
            _ => false,
        }
}

/// Effective provenance for queue filtering: the first-class `provenance`
/// field, or — for an as-yet-unmigrated legacy item — `telegram` derived
/// from the `via:telegram` label. Without this, `queue --provenance
/// telegram` would drop exactly the legacy Telegram items the transition is
/// meant to surface (they carry the label, not the field yet).
pub(crate) fn queue_provenance(issue: &models::Issue) -> Option<&str> {
    intake_provenance(issue).or_else(|| has_label(issue, "via:telegram").then_some("telegram"))
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
    // A row is either a real item in `target` (`legacy = false`) or a
    // recognised legacy form that migrates INTO `target` (`legacy = true`).
    // Keying off `target` (not the raw flag) keeps this correct if the CLI
    // later accepts `--state untriaged` explicitly, and surfaces legacy
    // `deferred` under `--state deferred`.
    let mut rows: Vec<(&models::Issue, bool)> = issues
        .iter()
        .filter_map(|i| {
            let legacy = is_legacy_for(i, target);
            if i.status == target || legacy {
                Some((i, legacy))
            } else {
                None
            }
        })
        .filter(|(i, _)| issue_type.as_deref().is_none_or(|t| i.issue_type == t))
        .filter(|(i, _)| {
            provenance
                .as_deref()
                .is_none_or(|p| queue_provenance(i) == Some(p))
        })
        .filter(|(i, _)| !needs_analysis || !has_triage_analysis(&i.body))
        .collect();
    // Stable order: oldest `created` first (items lacking a date sort
    // last), slug as the deterministic tiebreak.
    rows.sort_by(|(a, _), (b, _)| {
        let ka = (a.created.is_none(), a.created.clone(), a.slug.clone());
        let kb = (b.created.is_none(), b.created.clone(), b.slug.clone());
        ka.cmp(&kb)
    });
    let legacy_count = rows.iter().filter(|(_, legacy)| *legacy).count();

    if json {
        let arr: Vec<serde_json::Value> = rows
            .iter()
            .map(|(i, legacy)| {
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
                    "legacy": legacy,
                    "version": canonical::canonical_hash(i),
                })
            })
            .collect();
        let mut obj = serde_json::json!({ "state": target, "items": arr });
        if legacy_count > 0 {
            obj["legacy_pending"] = serde_json::json!(legacy_count);
            obj["migration_hint"] =
                serde_json::json!("run `issuectl intake migrate` to migrate legacy items");
        }
        println!("{}", serde_json::to_string_pretty(&obj)?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("No items in the {target} queue.");
        return Ok(());
    }
    println!(
        "Intake queue ({target}) — {} item(s), oldest first:",
        rows.len()
    );
    // The legacy label that surfaces this target (for the nudge wording).
    let legacy_label = if target == "deferred" {
        "deferred"
    } else {
        "needs-triage"
    };
    for (i, legacy) in &rows {
        let prov = queue_provenance(i).unwrap_or("-");
        let mut flag = String::new();
        if !has_triage_analysis(&i.body) {
            flag.push_str("  [needs-analysis]");
        }
        if *legacy {
            flag.push_str("  [legacy]");
        }
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
    if legacy_count > 0 {
        println!(
            "\nNote: {legacy_count} legacy item(s) shown [legacy] (open + {legacy_label}) — run `issuectl intake migrate` to migrate them."
        );
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
