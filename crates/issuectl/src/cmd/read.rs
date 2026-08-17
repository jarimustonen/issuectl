use super::*;

// ── Commands ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_list(
    json: bool,
    query_str: Option<String>,
    assignee: Option<String>,
    issue_type: Option<String>,
    priority: Option<String>,
    status: Option<String>,
    epic: Option<String>,
    label: Option<String>,
    all: bool,
    closed: bool,
) -> Result<()> {
    let mut q = match query_str.as_deref() {
        Some(s) => query::parse(s).context("parsing positional query")?,
        None => query::Query::default(),
    };
    let me = whoami();
    query::resolve_me(&mut q, me.as_deref()).context("resolving `:me` in query")?;

    // Translate flag filters into query terms. Flag values are
    // pre-validated by clap (PossibleValuesParser) so they can't
    // smuggle in `:`/`-` syntax that would re-enter the parser.
    if let Some(a) = assignee {
        q.push(query::Term::Field {
            field: query::FieldName::Assignee,
            m: query::FieldMatch::Equals(a),
            negated: false,
        });
    }
    if let Some(t) = issue_type {
        q.push(query::Term::Field {
            field: query::FieldName::Type,
            m: query::FieldMatch::Equals(t),
            negated: false,
        });
    }
    if let Some(p) = priority {
        q.push(query::Term::Field {
            field: query::FieldName::Priority,
            m: query::FieldMatch::Equals(p),
            negated: false,
        });
    }
    if let Some(s) = status {
        q.push(query::Term::Field {
            field: query::FieldName::Status,
            m: query::FieldMatch::Equals(s),
            negated: false,
        });
    }
    if let Some(e) = epic {
        q.push(query::Term::Field {
            field: query::FieldName::Epic,
            m: query::FieldMatch::Equals(e),
            negated: false,
        });
    }
    if let Some(l) = label {
        q.push(query::Term::Field {
            field: query::FieldName::Label,
            m: query::FieldMatch::Equals(l),
            negated: false,
        });
    }

    let folder_filter = list_folder_filter(&q, all, closed, query_str.is_some());

    let issues = load();
    // `repo::load_issues` already returns issues sorted by slug, so
    // we don't re-sort here. Build a blocker graph once so `blocks:`
    // queries can resolve against the loaded set (plain `query::matches`
    // can't see other issues and would return false for every `blocks:`
    // term).
    let graph = query::build_blocked_by_graph(&issues);
    let ctx = query::MatchCtx::today(&graph);
    let filtered: Vec<_> = issues
        .into_iter()
        .filter(|i| {
            folder_filter.map(|f| i.folder == f).unwrap_or(true) && query::matches_with(&q, i, &ctx)
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
                    // Same top-level `blocked_by` projection `show --json`
                    // applies, so a `jq '.blocked_by'` over list output reads
                    // the canonical array instead of `null` (the real value was
                    // buried under `.extra.blocked_by`). The derived reverse
                    // `blocks` view is `show`-only and deliberately not added
                    // here — a per-row reverse scan across the whole list is out
                    // of scope for a flat listing.
                    project_blocked_by(m, i);
                }
                v
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&with_version)?);
    } else {
        print_issue_table(&filtered);
    }

    Ok(())
}

/// Render a list of canonical (already `@`-stripped, valid) slugs as a
/// JSON array of `@`-prefixed refs — the frontmatter-facing form used by
/// `blocked_by`/`blocks` in `show --json`.
pub(crate) fn refs_json(slugs: &[String]) -> serde_json::Value {
    serde_json::Value::Array(
        slugs
            .iter()
            .map(|s| serde_json::Value::String(format!("@{s}")))
            .collect(),
    )
}

/// Derived reverse `blocks` view for `slug`: the sorted slugs of every
/// issue whose `blocked_by:` list names this one. The forward relationship
/// is stored in frontmatter; this reverse view is derived at read time —
/// the same reverse-edge derivation the query layer materialises repo-wide
/// in `query::build_blocked_by_graph`, computed here for a single subject.
/// A self-blocking issue (hand-edited `blocked_by` naming itself) is
/// excluded so it never appears in its own `blocks`. Slugs are unique per
/// issue, so the result needs no dedup.
pub(crate) fn blocks_of(issues: &[models::Issue], slug: &str) -> Vec<String> {
    let mut out: Vec<String> = issues
        .iter()
        .filter(|i| i.slug != slug && i.blocked_by().iter().any(|dep| dep == slug))
        .map(|i| i.slug.clone())
        .collect();
    out.sort();
    out
}

/// Project `blocked_by` onto the top level of a serialized issue object
/// `m` and strip the raw `extra.blocked_by` copy, so every `--json` path
/// that emits an issue carries exactly one representation of the field.
///
/// `blocked_by` lives in `Issue::extra` — intentionally NOT a typed field
/// (see `Issue::blocked_by`; typing it would let serde consume the key
/// before `extra` is built, dropping it from query/context bundles, and
/// would fold a non-canonical, `@`-sigil-carrying, hand-editable raw value
/// straight into `canonical_hash`, breaking the version token of every
/// existing issue). Plain serde therefore buries it inside the `extra`
/// object and never surfaces a top-level key, so `.blocked_by` reads
/// `null` while the real value hides under `.extra.blocked_by`. Lift the
/// canonical, `@`-prefixed ref list (sorted, deduped, validated by
/// `Issue::blocked_by`) to the top level — always present, empty when
/// there are no blockers, mirroring `.related`/`.labels` — and drop the
/// raw nested copy (whose shape/order/sigils could disagree) so consumers
/// read one authoritative representation of the *field*. Shared by the
/// issue-emitting `--json` paths (`show`, `ls`, `search`) so they serialize
/// `blocked_by` identically; this is the read-time wire migration off
/// `extra`, leaving on-disk frontmatter and the hash untouched.
///
/// Note: `version` (`canonical_hash`) is derived from the *raw* frontmatter
/// via `extra`, so it deliberately does NOT match this canonical projection
/// — it is an opaque optimistic-concurrency token, not a checksum a client
/// reconstructs from the wire. Two issues with identical projected
/// `blocked_by` can carry different `version`s (they wrote the raw list
/// differently); that is intended.
pub(crate) fn project_blocked_by(
    m: &mut serde_json::Map<String, serde_json::Value>,
    issue: &models::Issue,
) {
    if let Some(serde_json::Value::Object(extra)) = m.get_mut("extra") {
        extra.remove("blocked_by");
        if extra.is_empty() {
            m.remove("extra");
        }
    }
    m.insert("blocked_by".into(), refs_json(&issue.blocked_by()));
}

pub(crate) fn cmd_show(json: bool, slug: &str) -> Result<()> {
    let issues = load();
    // Prefix expansion: `show extremely` resolves to `extremely-quiet-otter`
    // when unique. `locate_issue_full` does this for mutating verbs; `show`
    // bypasses it (in-memory lookup), so route through `resolve_slug_input`
    // here. An ambiguous prefix surfaces its error; a no-match returns the
    // input unchanged so the existing not-found error path fires below.
    let root = find_root();
    let resolved = match repo::resolve_slug_input(&root, slug) {
        Ok(s) => s,
        Err(e) => {
            // Ambiguous prefix — surface the error to the user under the
            // unified output contract. `fail` diverges (`-> !`), so it can
            // be this arm's tail expression (a `return` around it trips
            // the unreachable-expression lint).
            fail(
                json,
                1,
                "ambiguous-slug",
                &format!("{e:#}"),
                serde_json::Value::Null,
            )
        }
    };
    let issue = issues.iter().find(|i| i.slug == resolved);

    match issue {
        Some(i) => {
            if json {
                let mut v = serde_json::to_value(i).expect("Issue serializes");
                if let serde_json::Value::Object(ref mut m) = v {
                    m.insert(
                        "version".into(),
                        serde_json::Value::String(canonical::canonical_hash(i)),
                    );
                    // Lift `blocked_by` to a single canonical top-level array
                    // and strip the raw `extra` copy (see `project_blocked_by`);
                    // the same projection `ls --json` applies, so both paths
                    // agree on one wire representation. Additionally add the
                    // derived reverse `blocks` view (the issues this one blocks)
                    // — a `show`-only projection, since it needs the full loaded
                    // set; like `version`, it is read-time, not persisted.
                    project_blocked_by(m, i);
                    m.insert("blocks".into(), refs_json(&blocks_of(&issues, &i.slug)));
                }
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print_issue_detail(i);
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

/// Report Definition-of-Done completion for one issue. Routes through
/// the shared parser in `issuectl_core::body` so the output matches
/// what the schema-level DoD gate (in `transitions::evaluate_dod`)
/// would see at write time. Exits 0 when `## Acceptance Criteria` is
/// present and fully checked, 1 otherwise — agents can gate a
/// "ready to mark done" step on `issuectl ready <slug>`.
pub(crate) fn cmd_ready(json: bool, slug: &str) -> Result<()> {
    use issuectl_core::body::DodReport;
    let issues = load();
    let Some(issue) = issues.into_iter().find(|i| i.slug == slug) else {
        fail(
            json,
            1,
            "not-found",
            &format!("issue {slug} not found"),
            serde_json::Value::Null,
        );
    };
    let report = DodReport::from_body(&issue.body);
    let ready = report.acceptance.fully_checked();

    if json {
        let section_json = |s: &issuectl_core::body::SectionStatus| {
            serde_json::json!({
                "present": s.present,
                "total": s.total(),
                "checked": s.checked(),
                "unchecked_items": s.unchecked_items()
                    .into_iter()
                    .map(|c| c.text.clone())
                    .collect::<Vec<_>>(),
            })
        };
        let v = serde_json::json!({
            "slug": issue.slug,
            "status": issue.status,
            "ready": ready,
            "acceptance_criteria": section_json(&report.acceptance),
            "tests_run": section_json(&report.tests),
            "implementation_notes": serde_json::json!({
                "present": report.notes.present,
            }),
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("issue: {} ({})", issue.slug, issue.status);
        println!("ready: {ready}");
        println!(
            "  Acceptance Criteria: {} of {} checked{}",
            report.acceptance.checked(),
            report.acceptance.total(),
            if !report.acceptance.present {
                " (section missing)"
            } else {
                ""
            },
        );
        for u in report.acceptance.unchecked_items() {
            println!("    [ ] {}", u.text);
        }
        println!(
            "  Tests Run: {} of {} checked{}",
            report.tests.checked(),
            report.tests.total(),
            if !report.tests.present {
                " (section missing)"
            } else {
                ""
            },
        );
        println!(
            "  Implementation Notes: {}",
            if report.notes.present {
                "present"
            } else {
                "missing"
            },
        );
    }
    if !ready {
        std::process::exit(1);
    }
    Ok(())
}

/// Open an issue's `item.md` (or its directory with `--dir`) in an
/// editor. The issue is a real file on disk, so we just resolve the
/// path and hand it to the editor. With `--json` we print the resolved
/// path instead of launching anything — agents and scripts cannot drive
/// an interactive editor, so spawning one would only hang them.
pub(crate) fn cmd_open(json: bool, slug: &str, dir: bool, editor: Option<String>) -> Result<()> {
    let root = find_root();
    // `locate_issue` returns (folder, item.md path) where `folder` is a
    // bare name like "open"/"closed", not a path — so the issue
    // directory is the parent of item.md.
    let (_, item_md) = locate_issue(&root, slug)?;
    let target = if dir {
        item_md
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow::anyhow!("cannot determine issue directory for {slug}"))?
    } else {
        item_md
    };

    if json {
        let report = serde_json::json!({
            "slug": slug,
            "path": target.to_string_lossy(),
            // `is_dir` (not `dir`) so the key never collides with the
            // issue-directory `dir` string field used by the action
            // commands — here it is the boolean "was --dir requested".
            "is_dir": dir,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let editor = editor
        .or_else(|| {
            std::env::var("VISUAL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .ok_or_else(|| {
            anyhow::anyhow!("no editor configured; pass --editor <cmd> or set $VISUAL / $EDITOR")
        })?;

    // Hand the editor string to `sh -c` so shell quoting works the way
    // it does for git's `GIT_EDITOR` — `--editor "code -w"` or an editor
    // path containing spaces (quoted by the user) both behave correctly,
    // rather than the naive whitespace split that mangles them. The
    // target path is passed as a positional arg, not interpolated, so a
    // path with spaces or shell metacharacters is never re-parsed.
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$@\""))
        .arg("sh")
        .arg(&target)
        .status()
        .with_context(|| format!("failed to launch editor {editor:?}"))?;
    if !status.success() {
        // Propagate the editor's own exit code so callers can tell, e.g.,
        // a vim `:cq` abort from a crash.
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Copy `files` into `issues/<slug>/attachments/`. Thin shim over
/// `mutate::attach::attach_files`; collision handling, lock acquisition,
/// and the per-file outcome shape all live there.
pub(crate) fn cmd_attach(json: bool, slug: &str, files: Vec<PathBuf>) -> Result<()> {
    let root = find_root();
    let report = match mutate::attach::attach_files(&root, slug, &files) {
        Ok(r) => r,
        Err(mutate::MutateError::Validation(msg)) => {
            fail(json, 1, "validation", &msg, serde_json::Value::Null);
        }
        Err(e) => return Err(anyhow::anyhow!("{e}")),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Attached {} file(s) to @{slug}:", report.attached.len());
        for f in &report.attached {
            let rename_note = if f.renamed {
                format!(" (renamed from {})", f.original_name)
            } else {
                String::new()
            };
            println!(
                "  {} -> {}{rename_note}",
                f.source.display(),
                f.path.display()
            );
        }
    }
    Ok(())
}

pub(crate) fn cmd_search(json: bool, query_str: &str, all: bool) -> Result<()> {
    let mut q = query::parse(query_str).context("parsing search query")?;
    let me = whoami();
    query::resolve_me(&mut q, me.as_deref()).context("resolving `:me` in query")?;
    let issues = load();

    // `search` keeps the historical scope rule: open-only unless
    // `--all`. A positive `folder:`/`status:` term in the query
    // can still expand scope, but a negated one (e.g.
    // `-status:wontfix`) is exclusion, not scope expansion.
    let scope_expanded = all
        || q.has_positive_field(query::FieldName::Folder)
        || q.has_positive_field(query::FieldName::Status);

    let graph = query::build_blocked_by_graph(&issues);
    let ctx = query::MatchCtx::today(&graph);
    let mut filtered: Vec<_> = issues
        .into_iter()
        .filter(|i| {
            if !scope_expanded && i.folder != "open" {
                return false;
            }
            query::matches_with(&q, i, &ctx)
        })
        .collect();

    filtered.sort_by(|a, b| a.slug.cmp(&b.slug));

    if json {
        // Mirror `list`/`show`: attach the optimistic-concurrency
        // `version` token to each issue so a `search` hit can be fed
        // straight into a mutation without a second `show` round-trip.
        let with_version: Vec<_> = filtered
            .iter()
            .map(|i| {
                let mut v = serde_json::to_value(i).expect("Issue serializes");
                if let serde_json::Value::Object(ref mut m) = v {
                    m.insert(
                        "version".into(),
                        serde_json::Value::String(canonical::canonical_hash(i)),
                    );
                    // Same top-level `blocked_by` projection `show --json`
                    // applies, so a `jq '.blocked_by'` over list output reads
                    // the canonical array instead of `null` (the real value was
                    // buried under `.extra.blocked_by`). The derived reverse
                    // `blocks` view is `show`-only and deliberately not added
                    // here — a per-row reverse scan across the whole list is out
                    // of scope for a flat listing.
                    project_blocked_by(m, i);
                }
                v
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&with_version)?);
    } else {
        print_issue_table(&filtered);
    }

    Ok(())
}

pub(crate) fn cmd_stats(json: bool) -> Result<()> {
    let issues = load();

    let open_count = issues.iter().filter(|i| i.folder == "open").count();
    let closed_count = issues.iter().filter(|i| i.folder == "closed").count();

    if json {
        let open_issues: Vec<_> = issues.iter().filter(|i| i.folder == "open").collect();
        let out = serde_json::json!({
            "total": issues.len(),
            "open": open_count,
            "closed": closed_count,
            "by_type": count_by_json(&open_issues, |i| &i.issue_type),
            "by_status": count_by_json(&open_issues, |i| &i.status),
            "by_priority": count_by_json(&open_issues, |i| &i.priority),
            "by_assignee": count_by_json(&open_issues, |i| {
                let a = i.effective_assignee();
                if a.is_empty() { "(none)" } else { a }
            }),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!(
            "Total: {}  (open: {}, closed: {})",
            issues.len(),
            open_count,
            closed_count
        );
        println!();

        let open_issues: Vec<_> = issues.iter().filter(|i| i.folder == "open").collect();
        print_counts("By type (open):", &open_issues, |i| &i.issue_type);
        print_counts("By status (open):", &open_issues, |i| &i.status);
        print_counts("By priority (open):", &open_issues, |i| &i.priority);
        print_counts("By assignee (open):", &open_issues, |i| {
            let a = i.effective_assignee();
            if a.is_empty() {
                "(none)"
            } else {
                a
            }
        });
    }

    Ok(())
}
