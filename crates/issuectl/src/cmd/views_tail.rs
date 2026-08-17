use super::*;

pub(crate) fn cmd_activity(json: bool, since: Option<String>, limit: Option<usize>) -> Result<()> {
    let since_days = match since.as_deref() {
        Some(s) => Some(report_mod::parse_since_days(s)?),
        None => None,
    };
    let root = find_root();
    let entries = report_mod::activity(&root, since_days, limit)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else if entries.is_empty() {
        println!("(no issue-file activity in range)");
    } else {
        for e in &entries {
            println!(
                "{}  {}  {}  {}",
                e.date,
                e.sha,
                e.slugs.join(","),
                e.summary
            );
        }
    }
    Ok(())
}

pub(crate) fn cmd_timeline(json: bool, slug: &str) -> Result<()> {
    let root = find_root();
    let events = report_mod::timeline(&root, slug)?;
    if json {
        let out = serde_json::json!({ "slug": slug, "events": events });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if events.is_empty() {
        println!("(no history for {slug})");
    } else {
        for e in &events {
            let arrow = match &e.prev_status {
                Some(p) => format!("{p} → {}", e.status),
                None => format!("(created) {}", e.status),
            };
            println!("{}  {}  {:<28} {}", e.date, e.sha, arrow, e.summary);
        }
    }
    Ok(())
}

pub(crate) fn cmd_changelog(json: bool, range: &str) -> Result<()> {
    let root = find_root();
    let issues = repo::load_issues(&root);
    let report = report_mod::changelog(&root, range, &issues)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report_mod::render_changelog_markdown(&report));
    }
    Ok(())
}

pub(crate) fn cmd_metrics(json: bool, since: Option<String>) -> Result<()> {
    let since_days = match since.as_deref() {
        Some(s) => Some(report_mod::parse_since_days(s)?),
        None => None,
    };
    let root = find_root();
    let issues = repo::load_issues(&root);
    let m = report_mod::metrics_today(&issues, since_days);
    if json {
        println!("{}", serde_json::to_string_pretty(&m)?);
    } else {
        match m.since_days {
            Some(d) => println!("Since {d}d:"),
            None => println!("All-time:"),
        }
        println!("  throughput: {}", m.throughput);
        if let Some(cs) = &m.cycle_time_days {
            println!(
                "  cycle time (days): median {}, p90 {}, mean {:.1} (n={})",
                cs.median, cs.p90, cs.mean, cs.sample
            );
        } else {
            println!("  cycle time: (no samples)");
        }
        if !m.closed_by_assignee.is_empty() {
            println!("\nClosed in window by assignee:");
            for (k, v) in &m.closed_by_assignee {
                println!("  {k:<20} {v}");
            }
        }
        if !m.workload_by_assignee.is_empty() {
            println!("\nOpen workload by assignee:");
            for (k, v) in &m.workload_by_assignee {
                println!("  {k:<20} {v}");
            }
        }
    }
    Ok(())
}
