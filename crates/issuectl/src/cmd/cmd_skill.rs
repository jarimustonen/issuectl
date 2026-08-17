use super::*;

pub(crate) fn cmd_body_set(
    json: bool,
    slug: &str,
    stdin: bool,
    from_file: Option<PathBuf>,
    expected_version: Option<String>,
) -> Result<()> {
    let body = if let Some(path) = from_file {
        read_capped_file(&path, "body")?
    } else if stdin {
        read_capped_stdin("body")?
    } else {
        bail!("specify exactly one of --stdin or --from-file");
    };
    // Strip only *trailing* whitespace, not leading: a stray final newline
    // from `echo … |` or an editor's end-of-file newline shouldn't bloat the
    // stored body, but a body legitimately starts with whitespace (a leading
    // 4-space indented code block, intentional spacing) that a full `trim()`
    // would silently corrupt. This is the deliberate divergence from `note`,
    // whose text is short prose that is fully trimmed and rejected when empty;
    // a body is a whole document and its leading content is the user's intent.
    // `update_body` re-adds the canonical leading newline.
    let body = body.trim_end().to_string();
    let root = find_root();
    let outcome = mutate::update_body(&root, slug, expected_version, body, false)
        .map_err(anyhow::Error::new)?;
    if json {
        let report = serde_json::json!({
            "slug": slug,
            "version": outcome.version,
            "dir": outcome.issue_dir.to_string_lossy(),
            "warnings": outcome.warnings,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Updated body of {slug}");
        emit_warnings_to_stderr(&outcome.warnings);
    }
    Ok(())
}

pub(crate) fn cmd_skill_list(json: bool) -> Result<()> {
    let catalog = skill::skill_catalog();
    if json {
        println!("{}", serde_json::to_string_pretty(&catalog)?);
        return Ok(());
    }

    let agent_width = catalog
        .iter()
        .flat_map(|entry| &entry.install_targets)
        .map(|target| target.agent.len())
        .max()
        .unwrap_or(0);
    for entry in &catalog {
        println!("{}  {}", entry.name, entry.description);
        for target in &entry.install_targets {
            println!(
                "  [{:agent_width$}] {}  {}",
                target.agent, target.label, target.path
            );
        }
    }
    Ok(())
}

pub(crate) fn cmd_skill_install(agent: &str, force: bool) -> Result<()> {
    let agents = match agent {
        "claude" => vec![skill::Agent::Claude],
        "codex" => vec![skill::Agent::Codex],
        "all" => vec![skill::Agent::Claude, skill::Agent::Codex],
        other => bail!("unknown agent {other:?}; expected claude, codex, or all"),
    };
    let root = find_root();
    // Dual-home Claude skills into pi.dev's skill dir (~/.pi/agent/skills).
    // Resolved from $HOME; `None` (HOME unset) simply skips the pi mirror.
    let pi_root = skill::pi_skills_root();
    skill::install_skill(&root, &agents, force, pi_root.as_deref())
}

pub(crate) fn cmd_skill_print(agent: &str) -> Result<()> {
    let resolved = skill::Agent::from_str(agent)?;
    skill::print_skill(resolved)
}

/// Resolve the pi.dev corpus root, erroring with a clear message when `$HOME`
/// is unresolvable (the same condition under which the install-time mirror is
/// silently skipped — but here the user explicitly asked to inspect it).
pub(crate) fn pi_corpus_root() -> Result<std::path::PathBuf> {
    skill::pi_skills_root()
        .context("cannot resolve the pi.dev skill corpus root: $HOME is unset or not absolute")
}

pub(crate) fn cmd_skill_pi_status(json: bool) -> Result<()> {
    let pi_root = pi_corpus_root()?;
    let report = skill::pi_status(&pi_root)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("pi.dev skill corpus  {}", report.root);
    println!("Running issuectl {}", report.version);
    println!();
    if report.skills.is_empty() {
        println!("  (no skill entries found)");
        return Ok(());
    }
    let width = report
        .skills
        .iter()
        .map(|s| s.name.len())
        .max()
        .unwrap_or(0);
    for s in &report.skills {
        let mark = match s.state {
            skill::PiSkillState::UpToDate => "✓",
            skill::PiSkillState::Unmanaged => "·",
            skill::PiSkillState::Stale | skill::PiSkillState::Modified => "⚠",
            skill::PiSkillState::Missing | skill::PiSkillState::Orphan => "✗",
            skill::PiSkillState::Inaccessible => "?",
        };
        let detail = match s.state {
            skill::PiSkillState::Stale => {
                // Direction-neutral: recorded and running versions differ, but
                // we don't order them (a downgrade is possible), so we don't
                // claim the copy is "older" — just that `--force` rewrites it to
                // the running version.
                let from = s.recorded_version.as_deref().unwrap_or("unknown");
                format!(
                    " (recorded {from} ≠ running {}; `issuectl skill install --force` rewrites it to {})",
                    report.version, report.version
                )
            }
            skill::PiSkillState::Modified => {
                " (hand-edited since install; run `issuectl skill install --force` to restore)"
                    .to_string()
            }
            skill::PiSkillState::Orphan => {
                " (no longer shipped; run `issuectl skill pi-prune --force` to remove)".to_string()
            }
            skill::PiSkillState::Missing => {
                " (copy gone; run `issuectl skill pi-prune --force` to clear the record)"
                    .to_string()
            }
            skill::PiSkillState::Inaccessible => {
                " (could not read this entry — permission or I/O error; left untouched, not pruned)"
                    .to_string()
            }
            skill::PiSkillState::Unmanaged => " (not written by issuectl)".to_string(),
            skill::PiSkillState::UpToDate => String::new(),
        };
        println!(
            "  {mark} {:width$}  {}{}",
            s.name,
            s.state.label(),
            detail,
            width = width
        );
    }
    if !report.has_findings() {
        println!();
        println!("  Everything issuectl owns is up to date.");
    }
    Ok(())
}

pub(crate) fn cmd_skill_pi_prune(json: bool, force: bool) -> Result<()> {
    let pi_root = pi_corpus_root()?;
    let outcome = skill::pi_prune(&pi_root, force)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
        return Ok(());
    }

    let kind_label = |k: skill::PiPruneKind| match k {
        skill::PiPruneKind::Orphan => "orphan",
        skill::PiPruneKind::Missing => "missing record",
    };
    let plural = |n: usize| if n == 1 { "y" } else { "ies" };

    if outcome.removed.is_empty() && outcome.skipped.is_empty() {
        println!("Nothing to prune — the pi.dev skill corpus has no orphaned issuectl entries.");
        return Ok(());
    }

    if !outcome.removed.is_empty() {
        // `force` (the input) distinguishes an applied run from a dry run —
        // `outcome.applied` is also false for a no-op, so it's not the right
        // signal for the header.
        if force {
            println!(
                "Removed {} entr{} from the pi.dev skill corpus:",
                outcome.removed.len(),
                plural(outcome.removed.len())
            );
        } else {
            println!(
                "Dry run — would remove {} entr{} (pass --force to apply):",
                outcome.removed.len(),
                plural(outcome.removed.len())
            );
        }
        for item in &outcome.removed {
            println!("  - {} ({})", item.name, kind_label(item.kind));
        }
    }

    if !outcome.skipped.is_empty() {
        println!();
        println!(
            "Left {} orphan entr{} in place for safety (symlink, extra files, or unremovable):",
            outcome.skipped.len(),
            plural(outcome.skipped.len())
        );
        for item in &outcome.skipped {
            println!("  - {} ({})", item.name, item.path);
        }
        println!("  Inspect and remove these by hand if you want them gone.");
    }

    if !force && !outcome.removed.is_empty() {
        println!();
        println!("  Re-run with `issuectl skill pi-prune --force` to apply.");
    }
    Ok(())
}

// ── Triage / pick / completions / scan-todos ───────────────────────────────

pub(crate) fn cmd_triage(json: bool, slug: Option<String>) -> Result<()> {
    let root = find_root();
    match slug {
        None => {
            // List inbox drafts.
            let issues = repo::load_issues(&root);
            let drafts: Vec<_> = issues.iter().filter(|i| i.folder == "inbox").collect();
            if json {
                let out: Vec<_> = drafts
                    .iter()
                    .map(|i| {
                        serde_json::json!({
                            "slug": i.slug,
                            "title": i.title,
                            "type": i.issue_type,
                            "created": i.created,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if drafts.is_empty() {
                println!("Inbox is empty.");
            } else {
                println!("Inbox drafts ({}):", drafts.len());
                for i in &drafts {
                    println!("  {}  {}", i.slug, i.title);
                }
                println!("\nPromote one with: issuectl triage <slug>");
            }
            Ok(())
        }
        Some(slug) => {
            // Triage expects a real on-disk inbox slug; expand prefixes
            // through the central resolver so `triage extrem` works.
            let resolved =
                repo::resolve_slug_input(&root, &slug).map_err(|e| anyhow::anyhow!("{e:#}"))?;
            let out = mutate::triage::triage(&root, &resolved).map_err(anyhow::Error::new)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "slug": out.slug,
                        "from": out.from.to_string_lossy(),
                        "to": out.to.to_string_lossy(),
                    }))?
                );
            } else {
                println!(
                    "Triaged {}: {} -> {}",
                    out.slug,
                    out.from.display(),
                    out.to.display()
                );
            }
            Ok(())
        }
    }
}

pub(crate) fn cmd_pick(json: bool, q: Option<String>, all: bool, first: bool) -> Result<()> {
    let root = find_root();
    let issues = repo::load_issues(&root);
    // Default: open-only (no inbox). With --all, include closed AND inbox.
    let needle = q.as_deref().map(|s| s.to_lowercase());
    let candidates: Vec<_> = issues
        .iter()
        .filter(|i| {
            if !all && i.folder != "open" {
                return false;
            }
            match &needle {
                None => true,
                Some(n) => {
                    i.slug.to_lowercase().contains(n)
                        || i.title.to_lowercase().contains(n)
                        || i.labels
                            .as_ref()
                            .map(|ls| ls.iter().any(|l| l.to_lowercase().contains(n)))
                            .unwrap_or(false)
                }
            }
        })
        .collect();

    if candidates.is_empty() {
        if json {
            emit_json_error(
                "no-match",
                "no issues match the picker query",
                serde_json::Value::Null,
            );
        } else {
            eprintln!("No matching issues.");
        }
        std::process::exit(1);
    }
    if candidates.len() == 1 || first {
        let chosen = candidates[0];
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "slug": chosen.slug,
                    "title": chosen.title,
                }))?
            );
        } else {
            println!("{}", chosen.slug);
        }
        return Ok(());
    }
    // Multiple matches — print menu on stderr, read selection from stdin.
    use std::io::{BufRead, Write};
    let stderr = std::io::stderr();
    let mut e = stderr.lock();
    writeln!(e, "{} matches:", candidates.len())?;
    for (idx, i) in candidates.iter().enumerate() {
        writeln!(e, "  [{:>3}] {}  {}", idx + 1, i.slug, i.title)?;
    }
    write!(e, "Select [1-{}]: ", candidates.len())?;
    e.flush()?;
    let stdin = std::io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    let idx: usize = line
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid selection: {line:?}"))?;
    if idx == 0 || idx > candidates.len() {
        bail!("selection out of range: {idx}");
    }
    let chosen = candidates[idx - 1];
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "slug": chosen.slug,
                "title": chosen.title,
            }))?
        );
    } else {
        println!("{}", chosen.slug);
    }
    Ok(())
}

pub(crate) fn cmd_completions(shell: ShellArg) -> Result<()> {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let bin = "issuectl";
    clap_complete::generate(
        clap_complete::Shell::from(shell),
        &mut cmd,
        bin,
        &mut std::io::stdout(),
    );
    Ok(())
}

pub(crate) fn cmd_complete_values(kind: CompleteKind) -> Result<()> {
    let root = find_root();
    match kind {
        CompleteKind::Slugs => {
            let issues = repo::load_issues(&root);
            for i in issues.iter().filter(|i| i.folder == "open") {
                println!("{}", i.slug);
            }
        }
        CompleteKind::SlugsAll => {
            for i in repo::load_issues(&root) {
                println!("{}", i.slug);
            }
        }
        CompleteKind::Statuses => {
            // Surface every status the schema knows about (built-in defaults
            // when no project schema is declared).
            let schema = issuectl_core::schema::load(&root)
                .unwrap_or_else(|_| issuectl_core::schema::default_schema());
            for s in issuectl_core::schema::status_universe(&schema) {
                println!("{s}");
            }
        }
        CompleteKind::Labels => {
            let mut all: std::collections::BTreeSet<String> = Default::default();
            for i in repo::load_issues(&root) {
                if let Some(ls) = i.labels {
                    for l in ls {
                        all.insert(l);
                    }
                }
            }
            for l in all {
                println!("{l}");
            }
        }
        CompleteKind::Users => {
            let mut all: std::collections::BTreeSet<String> = Default::default();
            for i in repo::load_issues(&root) {
                if let Some(r) = i.reporter {
                    all.insert(r);
                }
                if let Some(a) = i.assignee {
                    all.insert(a);
                }
                if let Some(o) = i.owner {
                    all.insert(o);
                }
            }
            for u in all {
                println!("{u}");
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct TodoHit {
    file: PathBuf,
    line: usize,
    slug: Option<String>,
    status: &'static str, // "tracked" | "stale" | "unknown" | "untracked"
    context: String,
}

pub(crate) fn cmd_scan_todos(json: bool, create_inbox: bool) -> Result<()> {
    let root = find_root();
    let issues = repo::load_issues(&root);
    // Build slug -> closing-or-not map.
    let schema = issuectl_core::schema::load(&root)
        .unwrap_or_else(|_| issuectl_core::schema::default_schema());
    let mut known: std::collections::BTreeMap<String, bool> = Default::default();
    for i in &issues {
        let closing = issuectl_core::schema::is_closing(&schema, &i.status);
        known.insert(i.slug.clone(), closing);
    }
    let hits = scan_todos_walk(&root, &known)?;

    if json {
        let arr: Vec<_> = hits
            .iter()
            .map(|h| {
                serde_json::json!({
                    "file": h.file.to_string_lossy(),
                    "line": h.line,
                    "slug": h.slug,
                    "status": h.status,
                    "context": h.context,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
    } else {
        if hits.is_empty() {
            println!("No TODO(issue: …) markers found.");
        }
        for h in &hits {
            println!(
                "{} {}:{} {}{}",
                h.status,
                h.file.display(),
                h.line,
                h.slug.as_deref().unwrap_or("-"),
                if h.context.is_empty() {
                    String::new()
                } else {
                    format!("  {}", h.context)
                }
            );
        }
    }

    if create_inbox {
        let untracked: Vec<&TodoHit> = hits.iter().filter(|h| h.status == "untracked").collect();
        for h in untracked {
            let title = if h.context.is_empty() {
                format!("TODO from {}:{}", h.file.display(), h.line)
            } else {
                h.context.clone()
            };
            let args = mutate::new_issue::NewArgs {
                issue_type: "task".into(),
                title: title.clone(),
                priority: "normal".into(),
                description: Some(format!(
                    "_Source: {}:{}_\n\n```\n{}\n```\n",
                    h.file.display(),
                    h.line,
                    h.context
                )),
                inbox: true,
                ..mutate::new_issue::NewArgs::default()
            };
            match do_new(&root, args) {
                Ok(out) => {
                    if !json {
                        println!(
                            "  + inbox draft {} for {}:{}",
                            out.slug,
                            h.file.display(),
                            h.line
                        );
                    }
                }
                Err(e) => {
                    eprintln!(
                        "warn: could not create inbox draft for {}:{}: {e:#}",
                        h.file.display(),
                        h.line
                    );
                }
            }
        }
    }
    Ok(())
}

/// Walk the repo tree, scanning every text-ish file for `TODO(issue: …)`
/// markers. Skips `.git`, `target`, `node_modules`, `issues/`, and any
/// path whose name starts with `.`. Lines are captured as `context` up
/// to 200 chars for the report.
pub(crate) fn scan_todos_walk(
    root: &Path,
    known: &std::collections::BTreeMap<String, bool>,
) -> Result<Vec<TodoHit>> {
    let mut hits = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let ft = match entry.file_type() {
                Ok(f) => f,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            if name.starts_with('.') {
                continue;
            }
            if ft.is_dir() {
                if matches!(name.as_str(), "target" | "node_modules" | "issues") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            // Cap per-file size to keep large lockfiles from dominating
            // the walk.
            if let Ok(meta) = entry.metadata() {
                if meta.len() > 1_000_000 {
                    continue;
                }
            }
            scan_one_file(&path, root, known, &mut hits);
        }
    }
    hits.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    Ok(hits)
}

pub(crate) fn scan_one_file(
    path: &Path,
    root: &Path,
    known: &std::collections::BTreeMap<String, bool>,
    hits: &mut Vec<TodoHit>,
) {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return,
    };
    // Skip binary-ish files: presence of a NUL byte is a strong signal.
    if bytes.contains(&0) {
        return;
    }
    let text = match std::str::from_utf8(&bytes) {
        Ok(s) => s,
        Err(_) => return,
    };
    let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
    for (idx, line) in text.lines().enumerate() {
        if let Some(hit) = parse_todo_marker(line) {
            let status = match &hit {
                TodoMarker::Tracked(s) => match known.get(s) {
                    Some(true) => "stale",
                    Some(false) => "tracked",
                    None => "unknown",
                },
                TodoMarker::Untracked => "untracked",
            };
            let slug = match hit {
                TodoMarker::Tracked(s) => Some(s),
                TodoMarker::Untracked => None,
            };
            hits.push(TodoHit {
                file: rel.clone(),
                line: idx + 1,
                slug,
                status,
                context: line.trim().chars().take(200).collect(),
            });
        }
    }
}

pub(crate) enum TodoMarker {
    Tracked(String),
    Untracked,
}

/// Recognise the `TODO(issue: <slug>)` and `TODO(issue:)` shapes.
/// Whitespace inside the parens is tolerated. Only the first marker on
/// a line is reported.
pub(crate) fn parse_todo_marker(line: &str) -> Option<TodoMarker> {
    let needle = "TODO(issue:";
    let start = line.find(needle)?;
    let rest = &line[start + needle.len()..];
    let end = rest.find(')')?;
    let inner = rest[..end].trim();
    if inner.is_empty() {
        return Some(TodoMarker::Untracked);
    }
    // Tolerate a leading `@`.
    let inner = inner.strip_prefix('@').unwrap_or(inner);
    if !slug::is_valid(inner) {
        return Some(TodoMarker::Untracked);
    }
    Some(TodoMarker::Tracked(inner.to_string()))
}

// ── Display helpers ─────────────────────────────────────────────────────────

const TABLE_HEADERS: &[&str] = &["Slug", "Title", "Type", "Status", "Pri", "Assignee"];

pub(crate) fn print_issue_table(issues: &[models::Issue]) {
    if issues.is_empty() {
        println!("No issues found.");
        return;
    }

    let rows: Vec<Vec<String>> = issues
        .iter()
        .map(|i| {
            vec![
                i.slug.clone(),
                truncate(&i.title, 50),
                i.issue_type.clone(),
                i.status.clone(),
                i.priority.clone(),
                i.effective_assignee().to_string(),
            ]
        })
        .collect();

    let mut widths: Vec<usize> = TABLE_HEADERS.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (j, cell) in row.iter().enumerate() {
            widths[j] = widths[j].max(cell.len());
        }
    }

    let header: String = TABLE_HEADERS
        .iter()
        .enumerate()
        .map(|(j, h)| format!("{:width$}", h, width = widths[j] + 1))
        .collect::<Vec<_>>()
        .join("");
    println!("{}", header.trim_end());

    let sep: String = widths
        .iter()
        .map(|w| "─".repeat(*w + 1))
        .collect::<Vec<_>>()
        .join("");
    println!("{}", sep.trim_end());

    for row in &rows {
        let line: String = row
            .iter()
            .enumerate()
            .map(|(j, cell)| format!("{:width$}", cell, width = widths[j] + 1))
            .collect::<Vec<_>>()
            .join("");
        println!("{}", line.trim_end());
    }

    println!("\n{} issue(s)", rows.len());
}

pub(crate) fn print_issue_detail(issue: &models::Issue) {
    println!("{}  {}", issue.slug, issue.title);
    println!("{}", "─".repeat(60));
    println!("Status:   {}  ({})", issue.status, issue.folder);
    println!("Type:     {}", issue.issue_type);
    println!("Priority: {}", issue.priority);
    if let Some(ref a) = issue.assignee {
        println!("Assignee: {}", a);
    }
    if let Some(ref o) = issue.owner {
        println!("Owner:    {}", o);
    }
    if let Some(ref r) = issue.reporter {
        println!("Reporter: {}", r);
    }
    if let Some(ref c) = issue.created {
        println!("Created:  {}", c);
    }
    if let Some(ref u) = issue.updated {
        println!("Updated:  {}", u);
    }
    if let Some(ref e) = issue.epic {
        println!("Epic:     @{}", e);
    }
    if let Some(ref lbs) = issue.labels {
        if !lbs.is_empty() {
            println!("Labels:   {}", lbs.join(", "));
        }
    }
    if let Some(ref rel) = issue.related {
        if !rel.is_empty() {
            println!("Related:  {}", rel.join(", "));
        }
    }
    if let Some(ref cl) = issue.closed {
        println!("Closed:   {}", cl);
    }
    if let Some(ref cb) = issue.closed_by {
        println!("Closed by: {}", cb);
    }
    if let Some(ref commits) = issue.commits {
        if !commits.is_empty() {
            println!("Commits:");
            for c in commits {
                println!("  {}  {}", c.hash, c.summary);
            }
        }
    }
    println!();
    println!("{}", issue.body);
}

/// Truncate `text` to roughly `max_len` Unicode scalar values, ending
/// with `…` when truncated. Note: this counts `chars()` (scalar
/// values), not grapheme clusters or terminal-display columns — CJK
/// wide characters and emoji ZWJ sequences may still misalign the
/// table. Switching to `unicode-width` is tracked as a follow-up;
/// this guard exists only to avoid panicking on UTF-8 byte boundaries.
pub(crate) fn truncate(text: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }
    let char_count = text.chars().count();
    if char_count <= max_len {
        text.to_string()
    } else {
        let take = max_len.saturating_sub(1);
        let mut out: String = text.chars().take(take).collect();
        out.push('…');
        out
    }
}

pub(crate) fn count_by_json<'a, F>(issues: &[&'a models::Issue], key_fn: F) -> serde_json::Value
where
    F: Fn(&'a models::Issue) -> &str,
{
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for issue in issues {
        let key = key_fn(issue).to_string();
        *counts.entry(key).or_insert(0) += 1;
    }
    serde_json::to_value(counts).unwrap_or_default()
}

pub(crate) fn print_counts<F>(header: &str, issues: &[&models::Issue], key_fn: F)
where
    F: Fn(&models::Issue) -> &str,
{
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for issue in issues {
        let key = key_fn(issue).to_string();
        *counts.entry(key).or_insert(0) += 1;
    }

    println!("{}", header);
    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
    for (key, count) in sorted {
        println!("  {:20} {}", key, count);
    }
    println!();
}
