use super::*;

/// Pre-creation duplicate check for `create --check-duplicates`. Builds a
/// synthetic issue from the prospective fields, scores it against the
/// existing open issues, and prints any strong matches. Returns `true`
/// when at least one strong match was found (the caller then refuses to
/// create).
pub(crate) fn duplicate_precheck(json: bool, args: &NewArgs) -> bool {
    let candidate = models::Issue {
        slug: String::new(),
        folder: "open".to_string(),
        created: None,
        status: "open".to_string(),
        updated: None,
        priority: args.priority.clone(),
        issue_type: args.issue_type.clone(),
        reporter: None,
        assignee: None,
        owner: None,
        epic: None,
        related: None,
        labels: if args.labels.is_empty() {
            None
        } else {
            Some(args.labels.clone())
        },
        closed: None,
        closed_by: None,
        lane: None,
        collision: None,
        lane_seq: None,
        commits: None,
        title: args.title.clone(),
        body: args.description.clone().unwrap_or_default(),
        extra: BTreeMap::new(),
    };

    let existing = load();
    let open = existing.iter().filter(|i| i.folder == "open");
    let matches = duplicates::find_duplicates(&candidate, open, duplicates::STRONG_THRESHOLD);

    if matches.is_empty() {
        return false;
    }

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
        // Unified error contract: `{"error":{code,message,matches}}` on
        // stderr. The caller exits 2 (refused-but-actionable).
        emit_json_error(
            "duplicate-precheck",
            "strong duplicate(s) found; not created (re-run without --check-duplicates to create anyway)",
            serde_json::json!({ "matches": out }),
        );
    } else {
        eprintln!("Refusing to create: strong duplicate(s) found:");
        for m in &matches {
            eprintln!("  {:.2}  {}  {}", m.score, m.slug, m.title);
        }
        eprintln!("Re-run without --check-duplicates to create anyway.");
    }
    true
}

pub(crate) fn cmd_new(json: bool, args: NewArgs, check_duplicates: bool) -> Result<()> {
    let root = find_root();
    if check_duplicates && duplicate_precheck(json, &args) {
        // A strong match was found and printed; refuse to create.
        std::process::exit(2);
    }
    // Whether the caller asked to schedule the issue at creation. When it
    // did, `--json` echoes the resulting `lane`/`lane_seq`/`collision`
    // back so a one-call create confirms the fields landed; a plain `create`
    // keeps its historical output shape untouched (no lane keys added).
    // `args` is moved into `do_new`, so capture the flag first.
    let lane_requested =
        args.lane.is_some() || args.lane_seq.is_some() || !args.collision.is_empty();
    let out = do_new(&root, args)?;
    if json {
        let mut report = serde_json::json!({
            "slug": out.slug,
            "title": out.title,
            // `path` = the item.md file; `dir` = the issue directory.
            // Shared vocabulary across all commands (see AGENTS.md).
            "path": out.item_path.to_string_lossy(),
            "dir": out
                .item_path
                .parent()
                .map(|p| p.to_string_lossy().into_owned()),
            "warnings": out.warnings,
        });
        if lane_requested {
            // Echo the values `do_new` captured under the creation lock —
            // NOT a fresh disk read, which would race a concurrent writer
            // (the rule `UpdateOutcome` follows). `collision` is already
            // the deduped, on-disk list; `lane_seq` stays a JSON number.
            report["lane"] = serde_json::json!(out.lane);
            report["lane_seq"] = serde_json::json!(out.lane_seq);
            report["collision"] = serde_json::json!(out.collision);
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Created {}: {}", out.slug, out.title);
        println!("  {}", out.item_path.display());
        emit_warnings_to_stderr(&out.warnings);
    }
    Ok(())
}

pub(crate) fn cmd_export(
    json: bool,
    format: ExportFmt,
    query_str: Option<String>,
    all: bool,
    closed: bool,
) -> Result<()> {
    let mut q = match query_str.as_deref() {
        Some(s) => query::parse(s).context("parsing export query")?,
        None => query::Query::default(),
    };
    let me = whoami();
    query::resolve_me(&mut q, me.as_deref()).context("resolving `:me` in query")?;
    let folder_filter = folder_default_filter(all, closed, query_str.is_some());

    let issues = load();
    let graph = query::build_blocked_by_graph(&issues);
    let ctx = query::MatchCtx::today(&graph);
    let filtered: Vec<_> = issues
        .into_iter()
        .filter(|i| {
            folder_filter.map(|f| i.folder == f).unwrap_or(true) && query::matches_with(&q, i, &ctx)
        })
        .collect();

    let rendered = issuectl_core::transfer::export(&filtered, format.into())?;
    print!("{rendered}");
    if !rendered.ends_with('\n') {
        println!();
    }
    let _ = json; // export format is selected by `format`, not `--json`
    Ok(())
}

pub(crate) struct ImportOutcome {
    created: Vec<(String, String)>,
    failed: Vec<(String, String)>,
}

/// Create every parsed import record through `do_new`, accumulating
/// successes and per-record failures. Failures never abort the run — a
/// single malformed record should not lose the rest of an import.
pub(crate) fn run_import(
    records: Vec<issuectl_core::transfer::ImportRecord>,
    default_type: &str,
) -> ImportOutcome {
    let root = find_root();
    let mut outcome = ImportOutcome {
        created: Vec::new(),
        failed: Vec::new(),
    };
    for rec in records {
        let title = rec.title.clone();
        let args = rec.into_new_args(default_type);
        match do_new(&root, args) {
            Ok(out) => outcome.created.push((out.slug, out.title)),
            Err(e) => outcome.failed.push((title, format!("{e:#}"))),
        }
    }
    outcome
}

pub(crate) fn report_import(json: bool, outcome: ImportOutcome) -> Result<()> {
    let ImportOutcome { created, failed } = outcome;
    if json {
        let report = serde_json::json!({
            "created": created
                .iter()
                .map(|(slug, title)| serde_json::json!({"slug": slug, "title": title}))
                .collect::<Vec<_>>(),
            "failed": failed
                .iter()
                .map(|(title, error)| serde_json::json!({"title": title, "error": error}))
                .collect::<Vec<_>>(),
            "created_count": created.len(),
            "failed_count": failed.len(),
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for (slug, title) in &created {
            println!("Created {slug}: {title}");
        }
        for (title, error) in &failed {
            eprintln!("Failed to import {title:?}: {error}");
        }
        println!(
            "Imported {} issue(s); {} failed.",
            created.len(),
            failed.len()
        );
    }
    // Distinct exit codes so scripts can tell total from partial
    // failure: 1 = nothing imported, 2 = some imported but some failed.
    // Exit 0 only when every record landed.
    if !failed.is_empty() {
        std::process::exit(if created.is_empty() { 1 } else { 2 });
    }
    Ok(())
}

pub(crate) fn cmd_import_json(json: bool, file: &Path, default_type: &str) -> Result<()> {
    let raw = fs::read_to_string(file)
        .with_context(|| format!("cannot read import file {}", file.display()))?;
    let records = issuectl_core::transfer::parse_json(&raw)?;
    let outcome = run_import(records, default_type);
    report_import(json, outcome)
}

pub(crate) fn cmd_import_github(
    json: bool,
    repo: &str,
    state: &str,
    limit: u32,
    default_type: &str,
) -> Result<()> {
    let output = std::process::Command::new("gh")
        .args([
            "issue",
            "list",
            "--repo",
            repo,
            "--state",
            state,
            "--limit",
            &limit.to_string(),
            "--json",
            "number,title,body,labels,state,assignees,url",
        ])
        .output()
        .context(
            "failed to run `gh` — install the GitHub CLI and authenticate with `gh auth login`",
        )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`gh issue list` failed: {}", stderr.trim());
    }
    let stdout = String::from_utf8(output.stdout).context("`gh` produced non-UTF8 output")?;
    let records = issuectl_core::transfer::parse_github(&stdout)?;
    let outcome = run_import(records, default_type);
    report_import(json, outcome)
}

#[derive(Default)]
pub(crate) struct UpdateArgs {
    pub slug: String,
    pub status: Option<String>,
    pub issue_type: Option<String>,
    pub assignee: Option<String>,
    pub no_reporter: bool,
    pub no_assignee: bool,
    pub owner: Option<String>,
    pub no_owner: bool,
    pub priority: Option<String>,
    pub epic: Option<String>,
    pub no_epic: bool,
    pub lane: Option<String>,
    pub no_lane: bool,
    pub lane_seq: Option<i64>,
    pub no_lane_seq: bool,
    pub add_collision: Vec<String>,
    pub remove_collision: Vec<String>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
    pub add_related: Vec<String>,
    pub remove_related: Vec<String>,
    pub add_blocked_by: Vec<String>,
    pub remove_blocked_by: Vec<String>,
    pub add_commits: Vec<String>,
    pub custom_fields: Vec<(String, String)>,
    pub clear_fields: Vec<String>,
    /// Replacement issue body (resolved from `--description`/`--body` or
    /// `--body-file`). `None` leaves the body untouched; `Some` replaces
    /// the whole existing body under the same flock as the frontmatter
    /// PATCH (see `mutate::UpdateIssueRequest::set_body`).
    pub set_body: Option<String>,
    pub expected_version: Option<String>,
}

pub(crate) struct UpdateOutcome {
    pub final_dir: PathBuf,
    pub moved_to_closed: bool,
    pub moved_to_open: bool,
    pub version: String,
    // Post-mutation values of the fields the action verbs echo in their
    // `--json` result so a caller can confirm the write without a second
    // `show` round-trip (see issue action-verb-json-echo-mutation). Read
    // straight off the updated `Issue` the mutate call returns under its
    // flock — never re-read from disk, which would race a concurrent
    // writer.
    pub status: String,
    pub priority: String,
    pub labels: Option<Vec<String>>,
    // The `closed_by:` the write actually recorded, read off the updated
    // `Issue`. `close` normalizes the `--as` author in the core (a single
    // leading `@` stripped), so echoing this — not the raw CLI input —
    // keeps the human/JSON confirmation in step with the stored token
    // (`--as "@example-user"` and `--as example-user` both echo `example-user`).
    pub closed_by: Option<String>,
    /// Non-fatal advisories from the write (e.g. a replacement body that
    /// carries a reserved-legacy section heading via `--body-file`).
    /// Empty for pure-frontmatter updates.
    pub warnings: Vec<String>,
}

/// Merge the post-mutation core fields (`status`/`priority`/`labels`) into
/// an action verb's `--json` result object. Every mutating verb echoes the
/// *same* field set from one shared place so the shapes cannot drift apart
/// (issue action-verb-json-echo-mutation). `labels` mirrors `show`: `null`
/// when the issue carries none, so a consumer parses the two identically.
pub(crate) fn echo_mutated_fields(
    report: &mut serde_json::Value,
    status: &str,
    priority: &str,
    labels: &Option<Vec<String>>,
) {
    report["status"] = serde_json::json!(status);
    report["priority"] = serde_json::json!(priority);
    report["labels"] = serde_json::json!(labels);
}

pub(crate) fn cmd_update(json: bool, args: UpdateArgs) -> Result<()> {
    let root = find_root();
    let slug = args.slug.clone();
    let out = do_update(&root, args)?;
    if json {
        let mut report = serde_json::json!({
            "slug": slug,
            "dir": out.final_dir.to_string_lossy(),
            "moved_to_closed": out.moved_to_closed,
            "moved_to_open": out.moved_to_open,
            "version": out.version,
        });
        echo_mutated_fields(&mut report, &out.status, &out.priority, &out.labels);
        report["warnings"] = serde_json::json!(out.warnings);
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if out.moved_to_closed {
        println!(
            "Updated {slug}: closing status set ({})",
            out.final_dir.display()
        );
    } else if out.moved_to_open {
        println!("Updated {slug}: re-opened ({})", out.final_dir.display());
    } else {
        println!("Updated {slug}");
    }
    emit_warnings_to_stderr(&out.warnings);
    Ok(())
}

pub(crate) fn do_update(root: &Path, args: UpdateArgs) -> Result<UpdateOutcome> {
    use mutate::Patch;
    let mut req = mutate::UpdateIssueRequest {
        expected_version: args.expected_version,
        set_body: args.set_body,
        ..Default::default()
    };
    if let Some(s) = args.status {
        req.status = Patch::Set(s);
    }
    if let Some(t) = args.issue_type {
        req.issue_type = Patch::Set(t);
    }
    if let Some(a) = args.assignee {
        req.assignee = Patch::Set(a);
    } else if args.no_assignee {
        req.assignee = Patch::Clear;
    }
    if args.no_reporter {
        req.reporter = Patch::Clear;
    }
    if let Some(o) = args.owner {
        req.owner = Patch::Set(o);
    } else if args.no_owner {
        req.owner = Patch::Clear;
    }
    if let Some(p) = args.priority {
        req.priority = Patch::Set(p);
    }
    if let Some(e) = args.epic {
        req.epic = Patch::Set(e);
    } else if args.no_epic {
        req.epic = Patch::Clear;
    }
    if let Some(l) = args.lane {
        req.lane = Patch::Set(l);
    } else if args.no_lane {
        req.lane = Patch::Clear;
    }
    if let Some(n) = args.lane_seq {
        req.lane_seq = Patch::Set(n);
    } else if args.no_lane_seq {
        req.lane_seq = Patch::Clear;
    }
    req.add_collision = args.add_collision;
    req.remove_collision = args.remove_collision;
    req.add_labels = args.add_labels;
    req.remove_labels = args.remove_labels;
    req.add_related = args.add_related;
    req.remove_related = args.remove_related;
    req.add_blocked_by = args.add_blocked_by;
    req.remove_blocked_by = args.remove_blocked_by;
    req.add_commits = args
        .add_commits
        .iter()
        .map(|spec| {
            let (hash, summary) = parse_commit_spec(spec)?;
            Ok::<_, anyhow::Error>(mutate::CommitSpec { hash, summary })
        })
        .collect::<Result<_, _>>()?;

    // `--field foo=a --field foo=b` collapses silently in a BTreeMap;
    // reject it explicitly so the user gets a precise error rather
    // than the last-wins surprise. `cmd_new` enforces the same
    // invariant — match its `BTreeSet<&str>` shape (no allocations,
    // deterministic overlap iteration).
    let mut seen_fields: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for (k, _) in &args.custom_fields {
        if !seen_fields.insert(k.as_str()) {
            bail!("--field {k:?} given more than once");
        }
    }
    let mut seen_clears: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for k in &args.clear_fields {
        if !seen_clears.insert(k.as_str()) {
            bail!("--clear-field {k:?} given more than once");
        }
    }
    if let Some(overlap) = seen_fields.intersection(&seen_clears).next() {
        bail!("field {overlap:?} appears in both --field and --clear-field");
    }
    for (k, v) in args.custom_fields {
        req.custom_fields.insert(k, mutate::Patch::Set(v));
    }
    for k in args.clear_fields {
        req.custom_fields.insert(k, mutate::Patch::Clear);
    }

    let outcome = mutate::update_issue(root, &args.slug, req).map_err(anyhow::Error::new)?;
    Ok(UpdateOutcome {
        final_dir: outcome.issue_dir,
        moved_to_closed: outcome.moved_to_closed,
        moved_to_open: outcome.moved_to_open,
        version: outcome.version,
        status: outcome.issue.status,
        priority: outcome.issue.priority,
        labels: outcome.issue.labels,
        closed_by: outcome.issue.closed_by,
        warnings: outcome.warnings,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_close(
    json: bool,
    slug: &str,
    status: Option<String>,
    author: Option<String>,
    comment: Option<String>,
    commits: Vec<String>,
    stamp: bool,
    expected_version: Option<String>,
) -> Result<()> {
    let root = find_root();
    // Guard the one combination `--stamp` can corrupt: recording a
    // commit that resolves to the *current* HEAD, which stamping then
    // rewrites — the issue would keep a reference to the pre-stamp sha.
    // Reject before any mutation so nothing is half-written; the user
    // can `close --stamp` and then `update --add-commit <new-sha>`.
    if stamp {
        if let Some(hash) = stamp_would_orphan_commit(&root, &commits) {
            bail!(
                "--commit {hash} resolves to the current HEAD, which --stamp rewrites; \
                 drop --commit and record the stamped sha afterwards with `update --add-commit`"
            );
        }
    }
    let out = do_close(
        &root,
        slug,
        status,
        author,
        comment,
        commits,
        expected_version,
    )?;
    // Stamp the `Fixes-Issue: @<slug>` changelog trailer into HEAD only
    // after the close itself has landed — a fail-safe side effect that
    // must never block the close: any unexpected git fault is downgraded
    // to a `Skipped` outcome rather than propagated (the close already
    // succeeded, so failing the command here would mislead automation
    // into thinking the close failed).
    let stamp_outcome = if stamp {
        Some(
            git_trailers::stamp_fixes_trailer(&root, slug).unwrap_or_else(|e| {
                git_trailers::StampOutcome::Skipped {
                    reason: format!("stamping failed: {e:#}"),
                }
            }),
        )
    } else {
        None
    };
    // Echo the closer the write actually recorded (normalized in the
    // core — a single leading `@` stripped), not the raw `--as` input,
    // so the human/JSON confirmation matches the stored `closed_by:`.
    let closed_by = out.closed_by.clone();
    if json {
        let mut report = serde_json::json!({
            "slug": slug,
            "dir": out.final_dir.to_string_lossy(),
            "moved_to_closed": out.moved_to_closed,
            "version": out.version,
        });
        // Echo the same core-field set every mutating verb does, so a
        // caller confirms the resulting closing `status` (and the
        // unchanged priority/labels) from this one result
        // (issue action-verb-json-echo-mutation).
        echo_mutated_fields(&mut report, &out.status, &out.priority, &out.labels);
        if let Some(by) = closed_by {
            report["closed_by"] = serde_json::Value::String(by);
        }
        if let Some(outcome) = &stamp_outcome {
            report["stamp"] = stamp_report_json(outcome);
        }
        report["warnings"] = serde_json::json!(out.warnings);
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if let Some(outcome) = &stamp_outcome {
        eprintln!("{}", stamp_report_human(outcome));
    }
    if out.moved_to_closed {
        // Echo the closer when `--as` recorded one, so the human output
        // confirms the attribution that landed in `closed_by:`.
        match &closed_by {
            Some(by) => println!("Closed {slug} (by {by}) ({})", out.final_dir.display()),
            None => println!("Closed {slug} ({})", out.final_dir.display()),
        }
    } else {
        println!("Updated {slug}");
    }
    emit_warnings_to_stderr(&out.warnings);
    Ok(())
}

/// Return the offending `--commit` hash if any recorded commit resolves
/// to the current HEAD (which `--stamp` would rewrite, orphaning the
/// recorded reference). `None` when there is no repo/HEAD or nothing
/// collides — a best-effort check that never itself fails the command.
pub(crate) fn stamp_would_orphan_commit(root: &Path, commits: &[String]) -> Option<String> {
    if commits.is_empty() {
        return None;
    }
    let head = git_rev_parse(root, "HEAD")?;
    for spec in commits {
        let hash = spec.split(':').next().unwrap_or(spec).trim();
        if hash.is_empty() {
            continue;
        }
        if let Some(resolved) = git_rev_parse(root, &format!("{hash}^{{commit}}")) {
            if resolved == head {
                return Some(hash.to_string());
            }
        }
    }
    None
}

/// `git rev-parse --verify --quiet <rev>` → the full sha, or `None` on
/// any failure (not a repo, unknown rev). Never errors.
pub(crate) fn git_rev_parse(root: &Path, rev: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", rev])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Render a `StampOutcome` as the `stamp` object echoed in `close
/// --json` output, so a caller confirms whether the changelog trailer
/// landed (and, when skipped, why) from the same result. Shape is a
/// stable `{ "status": ... }` discriminator so consumers switch on one
/// field rather than probing for presence.
pub(crate) fn stamp_report_json(outcome: &git_trailers::StampOutcome) -> serde_json::Value {
    use git_trailers::StampOutcome::*;
    match outcome {
        Stamped { old, sha } => {
            serde_json::json!({ "status": "stamped", "sha": sha, "previous_sha": old })
        }
        AlreadyPresent { sha } => serde_json::json!({ "status": "already_present", "sha": sha }),
        Skipped { reason } => serde_json::json!({ "status": "skipped", "reason": reason }),
    }
}

/// Human-readable one-liner for a `StampOutcome`, written to stderr so
/// it stays out of any parsed stdout.
pub(crate) fn stamp_report_human(outcome: &git_trailers::StampOutcome) -> String {
    use git_trailers::StampOutcome::*;
    let short = |sha: &str| sha[..sha.len().min(12)].to_string();
    match outcome {
        Stamped { sha, .. } => {
            format!(
                "Stamped Fixes-Issue trailer into {} (HEAD rewritten).",
                short(sha)
            )
        }
        AlreadyPresent { .. } => {
            "Fixes-Issue trailer already present on HEAD; nothing to stamp.".to_string()
        }
        Skipped { reason } => format!("Trailer not stamped: {reason}"),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn do_close(
    root: &Path,
    slug: &str,
    status: Option<String>,
    author: Option<String>,
    comment: Option<String>,
    commits: Vec<String>,
    expected_version: Option<String>,
) -> Result<UpdateOutcome> {
    // M4: read+decide+mutate now happen atomically inside
    // `mutate::close_issue` under one flock. The previous read-then-call
    // pattern was racy — a concurrent writer could flip the status or
    // type between the unlocked read here and the locked update later.
    let commit_specs = commits
        .iter()
        .map(|spec| {
            let (hash, summary) = parse_commit_spec(spec)?;
            Ok::<_, anyhow::Error>(mutate::CommitSpec { hash, summary })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let outcome = mutate::close_issue(
        root,
        slug,
        status,
        author,
        comment,
        commit_specs,
        expected_version,
    )
    .map_err(anyhow::Error::new)?;
    Ok(UpdateOutcome {
        final_dir: outcome.issue_dir,
        moved_to_closed: outcome.moved_to_closed,
        moved_to_open: outcome.moved_to_open,
        version: outcome.version,
        status: outcome.issue.status,
        priority: outcome.issue.priority,
        labels: outcome.issue.labels,
        closed_by: outcome.issue.closed_by,
        warnings: outcome.warnings,
    })
}

pub(crate) fn cmd_rename(json: bool, old: &str, new: &str, dry_run: bool) -> Result<()> {
    let root = find_root();
    let outcome = repo::rename_issue(&root, old, new, dry_run)?;
    for s in &outcome.skipped {
        eprintln!(
            "Warning: skipped {} ({}); any references to {old} there were left untouched — run `issuectl doctor`",
            s.slug, s.reason
        );
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
        return Ok(());
    }
    let total: usize = outcome.changes.iter().map(|c| c.occurrences).sum();
    let files = outcome
        .changes
        .iter()
        .map(|c| c.slug.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    if dry_run {
        println!(
            "Would rename {old} → {new} and rewrite {total} reference(s) across {files} file(s)"
        );
        for c in &outcome.changes {
            println!("  {} {} ({})", c.slug, c.field, c.occurrences);
        }
    } else {
        println!(
            "Renamed {old} → {new} ({}); rewrote {total} reference(s) across {files} file(s)",
            outcome.new_dir.display()
        );
    }
    Ok(())
}

pub(crate) fn cmd_stale(json: bool, days: i64) -> Result<()> {
    let root = find_root();
    let report = issuectl_core::stale::find_stale(&root, days);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if report.stale.is_empty() {
        println!("No issues stale for {days}+ days.");
        return Ok(());
    }
    println!(
        "{} issue(s) with no activity in {days}+ days:",
        report.stale.len()
    );
    for s in &report.stale {
        let wip = if s.in_progress { " [in-progress]" } else { "" };
        let who = s
            .assignee
            .as_deref()
            .map(|a| format!(" — {a}"))
            .unwrap_or_default();
        println!(
            "  {} ({}){wip}  {} days, last {} via {}{who}",
            s.slug, s.status, s.days_inactive, s.last_activity, s.source
        );
    }
    Ok(())
}

pub(crate) fn cmd_archive(json: bool, older_than: i64, dry_run: bool) -> Result<()> {
    let root = find_root();
    let report =
        mutate::archive::archive_closed(&root, older_than, dry_run).map_err(anyhow::Error::new)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let verb = if dry_run { "Would archive" } else { "Archived" };
    if report.archived.is_empty() {
        println!("Nothing to archive (closed {older_than}+ days ago).");
    } else {
        println!("{verb} {} issue(s):", report.archived.len());
        for mv in &report.archived {
            println!("  {} → {}", mv.slug, mv.to.display());
        }
    }
    for sk in &report.skipped {
        eprintln!("Warning: skipped {} ({})", sk.slug, sk.reason);
    }
    Ok(())
}

/// Locate an issue by slug. Returns (folder, item.md path) where
/// `folder` is the kanban-bucket label derived from frontmatter status.
/// Delegates to `repo::locate_issue`, which handles flat layout plus
/// legacy compat reads.
pub fn locate_issue(root: &Path, slug: &str) -> Result<(String, PathBuf)> {
    repo::locate_issue(root, slug)
}

pub(crate) fn cmd_sync_commits(
    json: bool,
    range: Option<String>,
    no_branch_fallback: bool,
    dry_run: bool,
) -> Result<()> {
    let root = find_root();
    let report = sync_commits::run(
        &root,
        sync_commits::SyncOptions {
            range,
            no_branch_fallback,
            dry_run,
        },
    )?;

    if json {
        let planned: Vec<_> = report
            .planned
            .iter()
            .map(|p| {
                serde_json::json!({
                    "slug": p.slug,
                    "hash": p.hash,
                    "summary": p.summary,
                    "kind": match p.kind {
                        sync_commits::AttributionKind::Refs => "refs",
                        sync_commits::AttributionKind::Fixes => "fixes",
                        sync_commits::AttributionKind::Branch => "branch",
                    },
                    "already_present": p.already_present,
                })
            })
            .collect();
        let applied: serde_json::Map<_, _> = report
            .applied
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::from(*v)))
            .collect();
        let envelope = serde_json::json!({
            "range": report.range,
            "branch_slug": report.branch,
            "planned": planned,
            "applied": serde_json::Value::Object(applied),
            "fixes_hints": report.fixes_hints.iter().collect::<Vec<_>>(),
            "unknown_slugs": report.unknown_slugs.iter().collect::<Vec<_>>(),
            "load_warnings": report.load_warnings,
            "warnings": report.warnings,
            "dry_run": report.dry_run,
        });
        println!("{}", serde_json::to_string_pretty(&envelope)?);
    } else {
        println!("Range: {}", report.range);
        if let Some(b) = &report.branch {
            println!("Branch fallback slug: @{b}");
        }
        if report.planned.is_empty() {
            println!("No commits with Refs-Issue / Fixes-Issue trailers in range.");
        } else {
            for p in &report.planned {
                let tag = match p.kind {
                    sync_commits::AttributionKind::Refs => "refs",
                    sync_commits::AttributionKind::Fixes => "fixes",
                    sync_commits::AttributionKind::Branch => "branch",
                };
                let suffix = if p.already_present {
                    " (already present)"
                } else if dry_run {
                    " (would add)"
                } else {
                    ""
                };
                println!(
                    "  @{slug:<32} {hash} {summary} [{tag}]{suffix}",
                    slug = p.slug,
                    hash = p.hash,
                    summary = p.summary,
                );
            }
        }
        if !report.applied.is_empty() {
            let total: usize = report.applied.values().sum();
            println!(
                "Added {total} commit(s) across {n} issue(s).",
                n = report.applied.len()
            );
        } else if !dry_run && !report.planned.is_empty() {
            println!("No new commits to add (all already present).");
        }
        for slug in &report.fixes_hints {
            eprintln!("Hint: @{slug} has Fixes-Issue trailer — consider `issuectl close {slug}`",);
        }
        for slug in &report.unknown_slugs {
            eprintln!("Warning: trailer references unknown slug @{slug} (no issue with that slug)",);
        }
        for w in &report.load_warnings {
            eprintln!("Warning: {w}");
        }
        for w in &report.warnings {
            eprintln!("Warning: {w}");
        }
    }
    Ok(())
}

pub(crate) fn parse_commit_spec(spec: &str) -> Result<(String, String)> {
    let (hash, summary) = spec
        .split_once(':')
        .with_context(|| format!("commit spec must be HASH:summary, got {spec:?}"))?;
    let hash = hash.trim();
    let summary = summary.trim();
    if hash.is_empty() || summary.is_empty() {
        bail!("commit spec must be HASH:summary, got {spec:?}");
    }
    Ok((hash.to_string(), summary.to_string()))
}

/// Resolve the note text from exactly one of: the positional `message`,
/// the `--message`/`--body` flag (`message_flag`), `--stdin`, or
/// `--from-file`/`--body-file PATH`. clap's `note_body` arg group guards
/// against more than one being set — so the `message.or(message_flag)`
/// below (the positional and its flag spelling are necessarily two clap
/// fields) can never discard a value — while this fn enforces that at
/// least one is present and that the resulting text is non-empty (a blank
/// note is a no-op that would only clutter the issue body). Returns the
/// text with surrounding whitespace trimmed so a stray trailing newline
/// (e.g. from `echo … | issuectl note --stdin`) doesn't bloat the body.
pub(crate) fn read_message_arg(
    message: Option<String>,
    message_flag: Option<String>,
    stdin: bool,
    from_file: Option<PathBuf>,
) -> Result<String> {
    let text = match (message, message_flag) {
        (Some(_), Some(_)) => {
            bail!("internal error: clap `note_body` did not enforce a single note text source")
        }
        (Some(message), None) | (None, Some(message)) => message,
        (None, None) => {
            if let Some(path) = from_file {
                read_capped_file(&path, "note")?
            } else if stdin {
                read_capped_stdin("note")?
            } else {
                bail!(
                    "provide the note text as an argument, or use --message/--body/--comment, \
                     --body-file PATH (- for stdin), --stdin, or --from-file PATH"
                );
            }
        }
    };
    let text = text.trim();
    if text.is_empty() {
        bail!("note text is empty");
    }
    Ok(text.to_string())
}

/// Upper bound on text read from stdin or a `--from-file` path. Bounds
/// memory so an unbounded source (`/dev/zero`, a runaway producer) fails
/// with a clear error instead of OOM-ing the process. 10 MiB is far above
/// any realistic note or issue body yet small enough to never threaten the
/// process.
const MAX_INPUT_BYTES: u64 = 10 * 1024 * 1024;

/// Read up to [`MAX_INPUT_BYTES`] from `reader`, erroring if the source
/// exceeds the cap or isn't valid UTF-8. `what`/`source` name the input in
/// error messages (e.g. "note" from "stdin"). Reads one byte past `limit`
/// so an exactly-at-limit input passes but anything larger is rejected
/// — and so an unbounded source (`/dev/zero`) is short-circuited after
/// `limit + 1` bytes rather than buffered to exhaustion. `limit` is a
/// parameter (not the constant directly) so tests can exercise the cap
/// with a tiny bound instead of allocating megabytes.
pub(crate) fn read_capped<R: std::io::Read>(
    reader: R,
    limit: u64,
    what: &str,
    source: impl std::fmt::Display,
) -> Result<String> {
    use std::io::Read;
    let mut buf = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut buf)
        .with_context(|| format!("cannot read {what} from {source}"))?;
    if buf.len() as u64 > limit {
        bail!("{what} from {source} exceeds the {limit}-byte limit");
    }
    String::from_utf8(buf).map_err(|e| {
        // Surface the byte offset so an agent (or human) can locate the
        // bad byte instead of guessing across a multi-megabyte input.
        let offset = e.utf8_error().valid_up_to();
        anyhow::anyhow!(
            "{what} from {source} is not valid UTF-8 (first invalid byte at offset {offset})"
        )
    })
}

/// Read `what` text from stdin, capped at [`MAX_INPUT_BYTES`]. Refuses to
/// block on an interactive terminal — an agent or script that forgot to
/// pipe input gets a clear error rather than a hung process.
pub(crate) fn read_capped_stdin(what: &str) -> Result<String> {
    use std::io::IsTerminal;
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        bail!(
            "stdin is a terminal; pipe the {what} text in, or pass a real file path to --from-file"
        );
    }
    read_capped(stdin.lock(), MAX_INPUT_BYTES, what, "stdin")
}

/// Read `what` text from a `--from-file` path, capped at [`MAX_INPUT_BYTES`].
/// A path of `-` means stdin (the standard Unix convention), routed through
/// [`read_capped_stdin`] so it gets the same TTY guard and cap. To read a
/// file literally named `-`, pass `./-`.
pub(crate) fn read_capped_file(path: &Path, what: &str) -> Result<String> {
    if path.as_os_str() == "-" {
        return read_capped_stdin(what);
    }
    let file = fs::File::open(path)
        .with_context(|| format!("cannot read {what} from {}", path.display()))?;
    read_capped(file, MAX_INPUT_BYTES, what, path.display())
}

/// Read the initial issue body for `issuectl create --body-file PATH`.
/// A path of `-` means stdin (via [`read_capped_file`]'s convention),
/// capped at [`MAX_INPUT_BYTES`].
///
/// Strips only *trailing* whitespace, not leading — a body is a whole
/// document whose leading content is the user's intent (a file may open
/// with a 4-space indented code block that a full `trim()` would
/// silently corrupt), while a stray final newline from an editor or
/// `echo … |` shouldn't bloat the stored body. This mirrors
/// `cmd_body_set`'s `body set --from-file` convention exactly, and is
/// idempotent with `render_new_item_from_fm`'s own `trim_end` of the
/// description. An empty (or whitespace-only) body is rejected as a
/// validation error so `--body-file` matches the non-empty contract of
/// the inline `--description`/`--body` flag rather than silently
/// creating an issue with a blank body.
pub(crate) fn read_body_file_arg(path: &Path) -> Result<String> {
    let body = read_capped_file(path, "body")?;
    let body = body.trim_end();
    if body.is_empty() {
        bail!("--body-file {} is empty", path.display());
    }
    Ok(body.to_string())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_note(
    json: bool,
    slug: &str,
    author: &str,
    message: Option<String>,
    message_flag: Option<String>,
    stdin: bool,
    from_file: Option<PathBuf>,
    decision: bool,
    agent_run: bool,
    dry_run: bool,
    expected_version: Option<String>,
) -> Result<()> {
    let message = read_message_arg(message, message_flag, stdin, from_file)?;
    let message = message.as_str();
    let section = if decision {
        body_sections::DECISIONS
    } else if agent_run {
        body_sections::AGENT_RUNS
    } else {
        body_sections::COMMENTS
    };
    let root = find_root();
    let outcome = mutate::note_issue(
        &root,
        slug,
        author,
        message,
        section,
        expected_version,
        dry_run,
    )
    .map_err(anyhow::Error::new)?;
    finish_mutation(json, slug, &outcome, dry_run, "Appended note to")
}

pub(crate) fn cmd_set(
    json: bool,
    slug: &str,
    field: &str,
    value: Option<String>,
    clear: bool,
    dry_run: bool,
    expected_version: Option<String>,
) -> Result<()> {
    use mutate::Patch;
    let mut req = mutate::UpdateIssueRequest {
        expected_version,
        dry_run,
        ..Default::default()
    };
    let patch = if clear {
        Patch::Clear
    } else {
        Patch::Set(value.expect("required by clap unless --clear"))
    };
    // Status is the one field whose `Patch::Clear` is rejected by the
    // mutate layer; for everything else we let the shared
    // validate()/under-lock path produce the canonical error message
    // so CLI and API agree byte-for-byte. Custom fields land in the
    // schema-validated `custom_fields` slot — reserved keys like
    // `labels` / `related` produce a hint pointing at the right verb.
    match field {
        "status" => req.status = patch,
        "priority" => req.priority = patch,
        "assignee" => req.assignee = patch,
        "owner" => req.owner = patch,
        "epic" => req.epic = patch,
        other => {
            req.custom_fields.insert(other.to_string(), patch);
        }
    }
    let root = find_root();
    let outcome = mutate::update_issue(&root, slug, req).map_err(anyhow::Error::new)?;
    finish_mutation(json, slug, &outcome, dry_run, "Updated")
}

pub(crate) fn cmd_check(
    json: bool,
    slug: &str,
    task: &str,
    dry_run: bool,
    expected_version: Option<String>,
) -> Result<()> {
    let root = find_root();
    let outcome = mutate::toggle_checkbox(&root, slug, task, expected_version, dry_run)
        .map_err(anyhow::Error::new)?;
    finish_mutation(json, slug, &outcome, dry_run, "Toggled checkbox in")
}

/// Collapse the two accepted `label` invocation forms — the positional
/// `label <slug> add|remove <label>` and the flag form
/// `label <slug> --add|--remove <label>` — into a single `(op, label)`
/// pair. The `label_target` `ArgGroup` plus the `op` → `label` `requires`
/// edge (see the `Label` variant) make clap accept *exactly one complete
/// form*, so every reachable input matches one of the three valid arms.
/// The `_` arm is therefore an internal-invariant guard, not a
/// user-facing usage error: incomplete/mixed invocations are already
/// rejected by clap as `usage-error` before dispatch ever calls this.
pub(crate) fn resolve_label_target(
    op: Option<LabelOp>,
    label: Option<String>,
    add: Option<String>,
    remove: Option<String>,
) -> Result<(LabelOp, String)> {
    match (op, label, add, remove) {
        (Some(op), Some(label), None, None) => Ok((op, label)),
        (None, None, Some(label), None) => Ok((LabelOp::Add, label)),
        (None, None, None, Some(label)) => Ok((LabelOp::Remove, label)),
        _ => Err(anyhow::anyhow!(
            "internal error: label arguments were not constrained to exactly \
             one form by clap (this is a bug in the `label_target` arg group)"
        )),
    }
}

pub(crate) fn cmd_label(
    json: bool,
    slug: &str,
    op: LabelOp,
    label: &str,
    dry_run: bool,
    expected_version: Option<String>,
) -> Result<()> {
    let mut req = mutate::UpdateIssueRequest {
        expected_version,
        dry_run,
        ..Default::default()
    };
    match op {
        LabelOp::Add => req.add_labels.push(label.to_string()),
        LabelOp::Remove => req.remove_labels.push(label.to_string()),
    }
    let root = find_root();
    let outcome = mutate::update_issue(&root, slug, req).map_err(anyhow::Error::new)?;
    finish_mutation(json, slug, &outcome, dry_run, "Updated labels for")
}
