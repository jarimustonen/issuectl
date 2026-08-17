use super::*;

pub(crate) fn cmd_dag(json: bool, reservations: Option<String>) -> Result<()> {
    let root = find_root();
    let issues = load();
    let schema = schema::load(&root)?;
    let reservations = reservations
        .map(|src| load_reservations(&src))
        .transpose()?;
    let view = dag::compute(&issues, &schema, reservations.as_ref());
    if json {
        println!("{}", serde_json::to_string_pretty(&view)?);
    } else {
        print_dag_human(&view);
    }
    Ok(())
}

/// Resolve the `--reservations` argument (a file path, `-` for stdin, or
/// an inline JSON string) into a parsed [`dag::Reservations`].
pub(crate) fn load_reservations(src: &str) -> Result<dag::Reservations> {
    let text = if src == "-" {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .context("cannot read reservations from stdin")?;
        buf
    } else if Path::new(src).is_file() {
        fs::read_to_string(src).with_context(|| format!("cannot read reservations file {src}"))?
    } else {
        // Treat as inline JSON.
        src.to_string()
    };
    let value: serde_json::Value = serde_json::from_str(&text).with_context(|| {
        format!("reservations is neither a readable file nor valid JSON: {src:?}")
    })?;
    dag::Reservations::from_json(&value).map_err(|e| anyhow::anyhow!(e))
}

/// Human-readable rendering of the scheduling DAG. `--json` is the
/// machine contract; this is the terminal view.
pub(crate) fn print_dag_human(view: &dag::DagView) {
    let mark = |i: &dag::DagIssue| -> &'static str {
        if i.spawnable {
            "▶"
        } else if i.is_head_of_line {
            "◆"
        } else {
            " "
        }
    };
    if view.lanes.is_empty() && view.unscheduled.is_empty() {
        println!("(no issues)");
        return;
    }
    println!("spawnable heads: {}", view.spawnable_heads);
    for lane in &view.lanes {
        let head = lane.head_of_line.as_deref().unwrap_or("—");
        println!(
            "lane {} (depth: {}, head-of-line: {head})",
            lane.lane, lane.depth
        );
        for i in &lane.issues {
            print_dag_row(mark(i), i);
        }
        println!();
    }
    if !view.unscheduled.is_empty() {
        println!("unscheduled");
        for i in &view.unscheduled {
            print_dag_row(mark(i), i);
        }
    }
}

pub(crate) fn print_dag_row(mark: &str, i: &dag::DagIssue) {
    let mut suffix = String::new();
    if !i.blockers_open.is_empty() {
        suffix.push_str(&format!(" blocked-by:{}", i.blockers_open.join(",")));
    }
    if !i.blockers_missing.is_empty() {
        suffix.push_str(&format!(" missing-dep:{}", i.blockers_missing.join(",")));
    }
    if i.reserved {
        suffix.push_str(" [reserved]");
    }
    if !i.collision.is_empty() {
        suffix.push_str(&format!(" collision:{}", i.collision.join(",")));
    }
    println!(
        "  {mark} {:<28} {:<12} {}{}",
        i.slug, i.status, i.title, suffix
    );
}

/// Render an epic (or, with no slug, every top-level epic) and its child
/// issues as a tree. Read-only: children are derived on read from each
/// issue's `epic:` back-reference via `epic_tree::build`. `--json` emits
/// the tree structurally — a single node object for one epic, an array of
/// nodes for the no-slug forest — matching how `show`/`ls` shape theirs.
pub(crate) fn cmd_epic_tree(json: bool, slug: Option<&str>) -> Result<()> {
    let issues = load();

    let Some(slug) = slug else {
        // No slug → forest of every top-level epic.
        let forest = epic_tree::build_forest(&issues);
        if json {
            println!("{}", serde_json::to_string_pretty(&forest)?);
        } else if forest.is_empty() {
            println!("(no epics)");
        } else {
            for (idx, node) in forest.iter().enumerate() {
                if idx > 0 {
                    println!();
                }
                print_epic_tree(node);
            }
        }
        return Ok(());
    };

    // Prefix / `@` expansion, mirroring `show`: a unique prefix resolves,
    // an ambiguous one surfaces its error under the unified contract, and
    // a no-match returns the input unchanged so the not-found path fires.
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

    match epic_tree::build(&issues, &resolved) {
        Some(tree) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&tree)?);
            } else {
                print_epic_tree(&tree);
            }
            Ok(())
        }
        None => fail(
            json,
            1,
            "not-found",
            &format!("issue {slug} not found"),
            serde_json::Value::Null,
        ),
    }
}

/// Human-readable epic tree. The root prints flush-left; descendants use
/// box-drawing connectors (`├─`/`└─`) with a per-level prefix so the
/// hierarchy is visible at a glance. `--json` is the machine contract.
pub(crate) fn print_epic_tree(root: &epic_tree::TreeNode) {
    print_epic_tree_row("", "", root);
    print_epic_tree_children("", root);
    let n = epic_tree::descendant_count(root);
    let label = if n == 1 { "descendant" } else { "descendants" };
    println!("\n{n} {label}");
}

/// Emit each child of `node`, tracking last-child so the connectors and
/// continuation prefix (`│  ` vs. three spaces) line up.
pub(crate) fn print_epic_tree_children(prefix: &str, node: &epic_tree::TreeNode) {
    let last = node.children.len().saturating_sub(1);
    for (idx, child) in node.children.iter().enumerate() {
        let is_last = idx == last;
        let connector = if is_last { "└─ " } else { "├─ " };
        print_epic_tree_row(prefix, connector, child);
        let child_prefix = format!("{prefix}{}", if is_last { "   " } else { "│  " });
        print_epic_tree_children(&child_prefix, child);
    }
}

/// One tree row: `<prefix><connector>@slug  [type/status/priority]  title`.
pub(crate) fn print_epic_tree_row(prefix: &str, connector: &str, node: &epic_tree::TreeNode) {
    println!(
        "{prefix}{connector}@{}  [{}/{}/{}]  {}",
        node.slug, node.issue_type, node.status, node.priority, node.title
    );
}

pub(crate) fn cmd_depend(
    json: bool,
    slug: &str,
    blocked_by: Vec<String>,
    add: bool,
    expected_version: Option<String>,
) -> Result<()> {
    let mut req = mutate::UpdateIssueRequest {
        expected_version,
        ..Default::default()
    };
    if add {
        req.add_blocked_by = blocked_by;
    } else {
        req.remove_blocked_by = blocked_by;
    }
    let root = find_root();
    let outcome = mutate::update_issue(&root, slug, req).map_err(anyhow::Error::new)?;
    let verb = if add {
        "Added blockers for"
    } else {
        "Removed blockers from"
    };
    finish_mutation(json, slug, &outcome, false, verb)
}

pub(crate) fn cmd_apply(json: bool, patch_path: &Path, dry_run: bool) -> Result<()> {
    let yaml_text = fs::read_to_string(patch_path)
        .with_context(|| format!("cannot read patch file {}", patch_path.display()))?;
    let (slug, mut req) = parse_apply_patch(&yaml_text, json)
        .with_context(|| format!("cannot parse patch fields in {}", patch_path.display()))?;
    req.dry_run = dry_run;
    let root = find_root();
    let outcome = mutate::update_issue(&root, &slug, req).map_err(anyhow::Error::new)?;
    finish_mutation(json, &slug, &outcome, dry_run, "Applied patch to")
}

/// Mutation to apply to every issue a `bulk` query matches. Mirrors the
/// subset of `UpdateArgs` that makes sense across many issues at once:
/// per-field set/clear and the label/related list ops. Per-issue
/// concerns (`expected_version`, commits) are intentionally absent —
/// bulk can't carry a distinct version per target.
#[derive(Default)]
pub(crate) struct BulkSpec {
    pub set: Vec<(String, String)>,
    pub clear: Vec<String>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
    pub add_related: Vec<String>,
    pub remove_related: Vec<String>,
}

impl BulkSpec {
    fn is_empty(&self) -> bool {
        self.set.is_empty()
            && self.clear.is_empty()
            && self.add_labels.is_empty()
            && self.remove_labels.is_empty()
            && self.add_related.is_empty()
            && self.remove_related.is_empty()
    }
}

/// One issue's outcome from a `bulk` run. `diff` is `Some` only in
/// dry-run mode (the bytes the write would have produced).
#[derive(Debug)]
pub(crate) struct BulkResult {
    pub slug: String,
    pub version: String,
    pub final_dir: PathBuf,
    pub diff: Option<String>,
}

/// Route a single field patch onto an `UpdateIssueRequest`. Built-in
/// single-value fields go to their typed slots (so e.g. a `status` set
/// gets the closed-date handling and a `status`/`type` clear hits the
/// canonical "cannot be cleared" error); everything else lands in the
/// schema-validated custom-field slot. Mirrors `cmd_set`'s routing,
/// extended with `type`.
pub(crate) fn route_bulk_field(
    req: &mut mutate::UpdateIssueRequest,
    key: &str,
    patch: mutate::Patch<String>,
) {
    match key {
        "status" => req.status = patch,
        "type" => req.issue_type = patch,
        "priority" => req.priority = patch,
        "assignee" => req.assignee = patch,
        "owner" => req.owner = patch,
        "epic" => req.epic = patch,
        other => {
            req.custom_fields.insert(other.to_string(), patch);
        }
    }
}

/// Build a fresh request from the spec. `mutate::bulk_update` calls this
/// factory once per target per phase (validate, then write) because
/// `UpdateIssueRequest` is not `Clone` and each write consumes its own
/// request; the mutation content is identical every time.
pub(crate) fn build_bulk_request(spec: &BulkSpec, dry_run: bool) -> mutate::UpdateIssueRequest {
    use mutate::Patch;
    let mut req = mutate::UpdateIssueRequest {
        dry_run,
        ..Default::default()
    };
    for (k, v) in &spec.set {
        route_bulk_field(&mut req, k, Patch::Set(v.clone()));
    }
    for k in &spec.clear {
        route_bulk_field(&mut req, k, Patch::Clear);
    }
    req.add_labels = spec.add_labels.clone();
    req.remove_labels = spec.remove_labels.clone();
    req.add_related = spec.add_related.clone();
    req.remove_related = spec.remove_related.clone();
    req
}

/// CLI-side spec checks that don't need disk access: at least one
/// mutation, and no key named twice or in both `--set` and `--clear`
/// (a `BTreeMap` would silently keep the last write otherwise). Mirrors
/// the `--field`/`--clear-field` dedup rules in `do_update`.
pub(crate) fn validate_bulk_spec(spec: &BulkSpec) -> Result<()> {
    if spec.is_empty() {
        bail!("bulk requires at least one mutation (--set/--clear/--add-label/--remove-label/--add-related/--remove-related)");
    }
    let mut seen_set: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for (k, _) in &spec.set {
        if !seen_set.insert(k.as_str()) {
            bail!("--set {k:?} given more than once");
        }
    }
    let mut seen_clear: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for k in &spec.clear {
        if !seen_clear.insert(k.as_str()) {
            bail!("--clear {k:?} given more than once");
        }
    }
    if let Some(overlap) = seen_set.intersection(&seen_clear).next() {
        bail!("field {overlap:?} appears in both --set and --clear");
    }
    Ok(())
}

/// Apply `spec` to every issue matching `q`, as one batch under a single
/// repo-wide flock (see [`mutate::bulk_update`]). Every target is
/// validated before any write lands, so a bad value writes nothing, and
/// there is no concurrent-writer race between validation and write.
pub(crate) fn bulk_apply(
    root: &Path,
    q: &query::Query,
    spec: &BulkSpec,
    dry_run: bool,
) -> Result<Vec<BulkResult>> {
    let issues = repo::load_issues(root);
    let graph = query::build_blocked_by_graph(&issues);
    let ctx = query::MatchCtx::today(&graph);
    let slugs: Vec<String> = issues
        .into_iter()
        .filter(|i| query::matches_with(q, i, &ctx))
        .map(|i| i.slug)
        .collect();
    if slugs.is_empty() {
        return Ok(Vec::new());
    }

    let outcomes = mutate::bulk_update(root, &slugs, |dr| build_bulk_request(spec, dr), dry_run)
        .map_err(anyhow::Error::new)?;

    let results = slugs
        .into_iter()
        .zip(outcomes)
        .map(|(slug, outcome)| {
            let diff = dry_run.then(|| {
                let before = outcome.before_serialized.as_deref().unwrap_or("");
                let after = outcome.pending_serialized.as_deref().unwrap_or(before);
                render_unified_diff(before, after, &outcome.issue_dir)
            });
            BulkResult {
                slug,
                version: outcome.version,
                final_dir: outcome.issue_dir,
                diff,
            }
        })
        .collect();
    Ok(results)
}

pub(crate) fn cmd_bulk(json: bool, query_str: &str, spec: BulkSpec, dry_run: bool) -> Result<()> {
    validate_bulk_spec(&spec)?;
    let mut q = query::parse(query_str).context("parsing bulk query")?;
    let me = whoami();
    query::resolve_me(&mut q, me.as_deref()).context("resolving `:me` in query")?;
    let root = find_root();
    let results = bulk_apply(&root, &q, &spec, dry_run)?;

    if json {
        let arr: Vec<_> = results
            .iter()
            .map(|r| {
                let mut o = serde_json::json!({
                    "slug": r.slug,
                    "version": r.version,
                    "dir": r.final_dir.to_string_lossy(),
                });
                if let Some(d) = &r.diff {
                    o["diff"] = serde_json::Value::String(d.clone());
                }
                o
            })
            .collect();
        let report = serde_json::json!({
            "dry_run": dry_run,
            "count": results.len(),
            "results": arr,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    if results.is_empty() {
        println!("No issues match the query.");
        return Ok(());
    }
    if dry_run {
        println!("{} issue(s) would be updated:", results.len());
        for r in &results {
            println!("  {}", r.slug);
        }
        println!();
        for r in &results {
            if let Some(d) = &r.diff {
                if !d.is_empty() {
                    print!("{d}");
                }
            }
        }
    } else {
        println!("Updated {} issue(s):", results.len());
        for r in &results {
            println!("  {}", r.slug);
        }
    }
    Ok(())
}

/// Parse the YAML patch text into `(slug, UpdateIssueRequest)`,
/// applying every CLI-side rule that doesn't require disk access.
/// Extracted so tests can pin the `--json` `expected_version`
/// rejection rules (round-2 #4) without spinning up `find_root` or
/// the global `ROOT_OVERRIDE`.
pub(crate) fn parse_apply_patch(
    yaml_text: &str,
    json: bool,
) -> Result<(String, mutate::UpdateIssueRequest)> {
    let mut yaml: serde_yaml::Value =
        serde_yaml::from_str(yaml_text).context("cannot parse as YAML")?;
    let map = yaml
        .as_mapping_mut()
        .ok_or_else(|| anyhow::anyhow!("patch file must be a YAML mapping at the top level"))?;
    let slug = map
        .remove(serde_yaml::Value::String("slug".into()))
        .ok_or_else(|| anyhow::anyhow!("patch file must declare `slug:`"))?
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("`slug:` must be a string"))?
        .to_string();
    if !slug::is_valid(&slug) {
        bail!("invalid slug shape: {slug:?}");
    }
    // `dry_run` is `#[serde(skip)]` on UpdateIssueRequest, so a
    // user-supplied `dry_run: true` would otherwise fail with a generic
    // "unknown field" error from `deny_unknown_fields`. Catch it here
    // with a precise message — dry-run is a CLI execution mode, not a
    // patch field.
    if map.contains_key(serde_yaml::Value::String("dry_run".into())) {
        bail!("`dry_run` is a CLI flag; use `issuectl apply --dry-run`, not a patch field");
    }
    let req: mutate::UpdateIssueRequest =
        serde_yaml::from_value(yaml).context("cannot parse patch fields")?;
    // Validate `--json` D4=B contract AFTER deserialization so we
    // catch `expected_version: null` and empty/whitespace strings —
    // a presence-only `map.contains_key` check passed `null` through,
    // which then deserialized to `None` and bypassed concurrency.
    if json {
        match req.expected_version.as_deref() {
            Some(v) if !v.trim().is_empty() && v.trim() == v => {}
            _ => bail!(
                "patch must include a non-empty `expected_version:` when invoked with --json \
                 (per design D4=B); fetch with `issuectl show <slug> --json`"
            ),
        }
    }
    Ok((slug, req))
}

/// Shared CLI epilogue for the new mutation verbs. On `--dry-run`
/// prints a unified diff between the bytes mutate.rs captured under
/// the flock; otherwise prints (or emits the JSON envelope for) the
/// standard mutation response. `--json` emits the same envelope as
/// `update` (slug, final_dir, moved_to_closed, moved_to_open,
/// version), with `dry_run: true` and `diff` added when planning.
pub(crate) fn finish_mutation(
    json: bool,
    slug: &str,
    outcome: &mutate::UpdateOutcome,
    dry_run: bool,
    human_verb: &str,
) -> Result<()> {
    if dry_run {
        // Both halves were captured under the held flock in mutate.rs.
        // No disk reads here — reading "before" outside the lock would
        // race a concurrent writer and produce a misleading diff.
        let before = outcome.before_serialized.as_deref().unwrap_or("");
        let after = outcome.pending_serialized.as_deref().unwrap_or(before);
        let diff = render_unified_diff(before, after, &outcome.issue_dir);
        if json {
            let report = serde_json::json!({
                "slug": slug,
                "dir": outcome.issue_dir.to_string_lossy(),
                "version": outcome.version,
                "moved_to_closed": outcome.moved_to_closed,
                "moved_to_open": outcome.moved_to_open,
                "dry_run": true,
                "diff": diff,
                "warnings": outcome.warnings,
            });
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print!("{diff}");
            emit_warnings_to_stderr(&outcome.warnings);
        }
        return Ok(());
    }
    if json {
        // Echo the post-mutation core fields so callers of the shared
        // mutation verbs (`label`, `set`, `check`, …) can confirm the write
        // from this result alone (issue action-verb-json-echo-mutation).
        let mut report = serde_json::json!({
            "slug": slug,
            "dir": outcome.issue_dir.to_string_lossy(),
            "version": outcome.version,
            "moved_to_closed": outcome.moved_to_closed,
            "moved_to_open": outcome.moved_to_open,
            "warnings": outcome.warnings,
        });
        echo_mutated_fields(
            &mut report,
            &outcome.issue.status,
            &outcome.issue.priority,
            &outcome.issue.labels,
        );
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{human_verb} {slug}");
        emit_warnings_to_stderr(&outcome.warnings);
    }
    Ok(())
}

/// Print non-fatal advisories from `UpdateOutcome::warnings` to stderr
/// so the human-readable CLI surface mirrors the JSON `warnings` key.
/// Body-only verbs (`note`, `check`) emit transition-rule mismatches
/// here without blocking the write — see `mutate::transition_warnings`.
pub(crate) fn emit_warnings_to_stderr(warnings: &[String]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
}

/// Render a git-style unified diff between `before` and `after`. The
/// header path is rendered as `issues/<slug>/item.md` rather than the
/// absolute path so the output looks like a normal `git diff` rather
/// than `--- a//abs/path/...` (the leading double-slash from joining
/// `a/` with an absolute path).
pub(crate) fn render_unified_diff(before: &str, after: &str, issue_dir: &Path) -> String {
    if before == after {
        return String::new();
    }
    let rel = issue_dir
        .file_name()
        .map(|n| format!("issues/{}/item.md", n.to_string_lossy()))
        .unwrap_or_else(|| issue_dir.join("item.md").display().to_string());
    let diff = similar::TextDiff::from_lines(before, after);
    let header_old = format!("--- a/{rel}\n");
    let header_new = format!("+++ b/{rel}\n");
    let body = diff.unified_diff().context_radius(3).to_string();
    format!("{header_old}{header_new}{body}")
}
