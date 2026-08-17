use super::*;

pub(crate) fn cmd_cycle_current(json: bool) -> Result<()> {
    let label = cycle_mod::current_cycle();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "cycle": label }))?
        );
    } else {
        println!("{label}");
    }
    Ok(())
}

/// Resolve the user-supplied cycle name. `current` is a magic alias
/// that expands to the current-cycle label; every other string is
/// returned verbatim. Trimmed so trailing whitespace from a shell
/// pipeline doesn't silently miss matches.
pub(crate) fn resolve_cycle_name(name: &str) -> String {
    let n = name.trim();
    if n.eq_ignore_ascii_case("current") {
        cycle_mod::current_cycle()
    } else {
        n.to_string()
    }
}

pub(crate) fn cmd_cycle_plan(json: bool, name: &str, all: bool, closed: bool) -> Result<()> {
    let cycle = resolve_cycle_name(name);
    let folder_filter = folder_default_filter(all, closed, false);
    let issues = load();
    let filtered: Vec<_> = issues
        .into_iter()
        .filter(|i| {
            cycle_mod::issue_cycle(i) == Some(cycle.as_str())
                && folder_filter.map(|f| i.folder == f).unwrap_or(true)
        })
        .collect();

    if json {
        let with_version: Vec<_> = filtered
            .iter()
            .map(|i| {
                let mut v = serde_json::to_value(i).expect("Issue serializes");
                if let serde_json::Value::Object(ref mut m) = v {
                    m.insert(
                        "version".into(),
                        serde_json::Value::String(canonical::canonical_hash(i)),
                    );
                }
                v
            })
            .collect();
        let out = serde_json::json!({ "cycle": cycle, "issues": with_version });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if filtered.is_empty() {
        println!("(no issues in cycle {cycle})");
    } else {
        println!("Cycle {cycle}:");
        print_issue_table(&filtered);
    }
    Ok(())
}

pub(crate) fn cmd_cycle_status(json: bool, name: Option<&str>, all: bool) -> Result<()> {
    let issues = load();

    if all {
        let groups = cycle_mod::group_by_cycle(&issues);
        let rollups: Vec<_> = groups
            .keys()
            .map(|c| cycle_mod::status_for(&issues, c))
            .collect();
        if json {
            println!("{}", serde_json::to_string_pretty(&rollups)?);
        } else if rollups.is_empty() {
            println!("(no cycles found)");
        } else {
            println!(
                "{:<14} {:>5} {:>7} {:>6}",
                "CYCLE", "OPEN", "CLOSED", "TOTAL"
            );
            for r in &rollups {
                println!(
                    "{:<14} {:>5} {:>7} {:>6}",
                    r.cycle, r.open, r.closed, r.total
                );
            }
        }
        return Ok(());
    }

    let cycle = match name {
        Some(n) => resolve_cycle_name(n),
        None => cycle_mod::current_cycle(),
    };
    let s = cycle_mod::status_for(&issues, &cycle);
    if json {
        println!("{}", serde_json::to_string_pretty(&s)?);
    } else {
        println!(
            "Cycle {}:  open: {}, closed: {}, total: {}",
            s.cycle, s.open, s.closed, s.total
        );
        if !s.by_status.is_empty() {
            println!();
            println!("By status (open):");
            for (k, v) in &s.by_status {
                println!("  {k:<14} {v}");
            }
        }
        if !s.by_type.is_empty() {
            println!();
            println!("By type (open):");
            for (k, v) in &s.by_type {
                println!("  {k:<14} {v}");
            }
        }
    }
    Ok(())
}

pub(crate) fn cmd_schedule_list(json: bool) -> Result<()> {
    let root = find_root();
    let defs = recurrence::load_definitions(&root)?;
    let manifest = recurrence::load_manifest(&root).unwrap_or_default();
    if json {
        let value: Vec<serde_json::Value> = defs
            .iter()
            .map(|d| {
                let state = manifest.recurrences.get(&d.name);
                serde_json::json!({
                    "name": d.name,
                    "title": d.file.title,
                    "schedule": d.file.schedule,
                    "template": d.template_label(),
                    "type": d.file.issue_type,
                    "priority": d.file.priority,
                    "labels": d.file.labels,
                    "assignee": d.file.assignee,
                    "reporter": d.file.reporter,
                    "last_fire": state.and_then(|s| s.last_fire.clone()),
                    "materialized_count": state.map(|s| s.occurrences.len()).unwrap_or(0),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if defs.is_empty() {
        println!("(no recurrences in .issuectl/recurrences/)");
    } else {
        println!("{:<24} {:<18} TITLE", "NAME", "SCHEDULE");
        for d in &defs {
            println!("{:<24} {:<18} {}", d.name, d.file.schedule, d.file.title);
        }
    }
    Ok(())
}

pub(crate) fn cmd_schedule_run(json: bool, dry_run: bool) -> Result<()> {
    let root = find_root();
    let report = recurrence::run_now(&root, dry_run)?;
    if json {
        // Custom-shape so `path` flattens to a plain string instead
        // of PathBuf's debug rendering — matches the rest of the
        // CLI's JSON contract.
        let materialized: Vec<serde_json::Value> = report
            .materialized
            .iter()
            .map(|m| {
                serde_json::json!({
                    "recurrence": m.recurrence,
                    "occurrence": m.occurrence,
                    "slug": m.slug,
                    "title": m.title,
                    "path": m.path.display().to_string(),
                })
            })
            .collect();
        let value = serde_json::json!({
            "dry_run": report.dry_run,
            "recurrences_evaluated": report.recurrences_evaluated,
            "skipped_already_materialized": report.skipped_already_materialized,
            "materialized": materialized,
            "subscribed": report.subscribed,
            "capped": report.capped,
            "errors": report
                .errors
                .iter()
                .map(|(n, m)| serde_json::json!({"recurrence": n, "message": m}))
                .collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        let prefix = if dry_run { "[dry-run] " } else { "" };
        if report.materialized.is_empty() {
            println!(
                "{prefix}no occurrences due ({} recurrence(s) evaluated)",
                report.recurrences_evaluated
            );
        } else {
            for m in &report.materialized {
                if dry_run {
                    println!(
                        "{prefix}would materialize {} @ {}",
                        m.recurrence, m.occurrence
                    );
                } else {
                    println!(
                        "materialized {} @ {} → {}",
                        m.recurrence, m.occurrence, m.slug
                    );
                }
            }
        }
        for name in &report.subscribed {
            eprintln!(
                "subscribed recurrence {name:?} at this run; first issue will materialize at next cron tick"
            );
        }
        for name in &report.capped {
            eprintln!(
                "warning: recurrence {name:?} hit the per-run catch-up cap ({} occurrences); rerun to continue",
                recurrence::MAX_CATCHUP_PER_RUN
            );
        }
        for (name, msg) in &report.errors {
            eprintln!("warning: recurrence {name}: {msg}");
        }
    }
    Ok(())
}

pub(crate) fn cmd_workload(json: bool) -> Result<()> {
    let issues = load();
    let w = estimate_mod::workload(&issues);
    // Only flag mixed on the same scope `workload` rolls up (open +
    // in-progress) — long-closed issues with both fields aren't
    // actionable noise on the user's current load summary.
    let open_issues: Vec<_> = issues
        .iter()
        .filter(|i| i.folder != "closed")
        .cloned()
        .collect();
    let mixed = estimate_mod::mixed_issues(&open_issues);

    if json {
        let out = serde_json::json!({
            "total": w.total,
            "total_points": w.total_points,
            "unestimated": w.unestimated,
            "by_assignee": w.by_assignee,
            "by_priority": w.by_priority,
            "by_cycle": w.by_cycle,
            "by_epic": w.by_epic,
            "mixed_estimate_issues": mixed,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!(
        "Workload (open + in-progress): {} issues, {:.1} points  ({} unestimated)",
        w.total, w.total_points, w.unestimated
    );
    if !mixed.is_empty() {
        const SHOW: usize = 5;
        let shown = mixed
            .iter()
            .take(SHOW)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if mixed.len() > SHOW {
            format!(" (and {} more)", mixed.len() - SHOW)
        } else {
            String::new()
        };
        println!(
            "warning: {} issue(s) carry both `size:` and `estimate:` — pick one (preferring `estimate`): {shown}{suffix}",
            mixed.len()
        );
    }
    print_workload_rows("By assignee", &w.by_assignee);
    print_workload_rows("By priority", &w.by_priority);
    print_workload_rows("By cycle", &w.by_cycle);
    print_workload_rows("By epic", &w.by_epic);
    Ok(())
}

pub(crate) fn print_workload_rows(header: &str, rows: &[estimate_mod::WorkloadRow]) {
    println!();
    println!("{header}:");
    if rows.is_empty() {
        println!("  (no issues)");
        return;
    }
    println!(
        "  {:<20} {:>6} {:>8} {:>12}",
        "KEY", "COUNT", "POINTS", "UNESTIMATED"
    );
    for r in rows {
        println!(
            "  {:<20} {:>6} {:>8.1} {:>12}",
            truncate_key(&r.key, 20),
            r.count,
            r.points,
            r.unestimated
        );
    }
}

pub(crate) fn truncate_key(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let taken: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{taken}…")
    }
}

pub(crate) fn cmd_burndown(json: bool, cycle_name: &str) -> Result<()> {
    let cycle = resolve_cycle_name(cycle_name);
    let issues = load();
    let b = estimate_mod::burndown(&issues, &cycle);
    if json {
        println!("{}", serde_json::to_string_pretty(&b)?);
    } else {
        print!("{}", estimate_mod::render_ascii(&b));
    }
    Ok(())
}

pub(crate) fn cmd_duplicates(
    json: bool,
    slug: Option<&str>,
    threshold: Option<f64>,
    all: bool,
) -> Result<()> {
    let threshold = threshold.unwrap_or(duplicates::DEFAULT_THRESHOLD);
    let issues = load();

    match slug {
        Some(slug) => {
            let target = match issues.iter().find(|i| i.slug == slug) {
                Some(t) => t,
                None => fail(
                    json,
                    1,
                    "not-found",
                    &format!("issue {slug} not found"),
                    serde_json::Value::Null,
                ),
            };
            // The target is always a valid candidate scope; `--all`
            // only controls whether *closed* issues are compared
            // against it.
            let pool = issues
                .iter()
                .filter(|c| all || c.folder == "open" || c.slug == slug);
            let matches = duplicates::find_duplicates(target, pool, threshold);

            if json {
                let out: Vec<_> = matches
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "slug": m.slug,
                            "title": m.title,
                            "score": m.score,
                            "title_overlap": m.title_overlap,
                            "body_overlap": m.body_overlap,
                            "label_overlap": m.label_overlap,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if matches.is_empty() {
                println!("No likely duplicates of {slug} (threshold {threshold:.2}).");
            } else {
                println!("Likely duplicates of {slug} (threshold {threshold:.2}):");
                for m in &matches {
                    println!("  {:.2}  {}  {}", m.score, m.slug, m.title);
                }
            }
        }
        None => {
            let pool: Vec<_> = if all {
                issues
            } else {
                issues.into_iter().filter(|i| i.folder == "open").collect()
            };
            let pairs = duplicates::find_all_pairs(&pool, threshold);

            if json {
                let out: Vec<_> = pairs
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "a_slug": p.a_slug,
                            "a_title": p.a_title,
                            "b_slug": p.b_slug,
                            "b_title": p.b_title,
                            "score": p.score,
                            "title_overlap": p.title_overlap,
                            "body_overlap": p.body_overlap,
                            "label_overlap": p.label_overlap,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if pairs.is_empty() {
                println!("No likely duplicate pairs (threshold {threshold:.2}).");
            } else {
                println!("Likely duplicate pairs (threshold {threshold:.2}):");
                for p in &pairs {
                    println!(
                        "  {:.2}  {} <-> {}\n        {}\n        {}",
                        p.score, p.a_slug, p.b_slug, p.a_title, p.b_title
                    );
                }
            }
        }
    }

    Ok(())
}
