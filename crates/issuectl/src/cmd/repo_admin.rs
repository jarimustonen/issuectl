use super::*;

pub(crate) fn cmd_fmt(json: bool, slugs: Vec<String>, check: bool, diff: bool) -> Result<()> {
    let mode = if check {
        fmt::FormatMode::Check
    } else if diff {
        fmt::FormatMode::Diff
    } else {
        fmt::FormatMode::Write
    };
    let root = find_root();
    let results = fmt::format_repo(&root, &slugs, mode)?;
    let any_changed = results
        .iter()
        .any(|r| r.status == fmt::FormatStatus::Changed);

    if json {
        let entries: Vec<_> = results
            .iter()
            .map(|r| {
                let mut o = serde_json::json!({
                    "path": r.path.to_string_lossy(),
                    "status": match r.status {
                        fmt::FormatStatus::Unchanged => "unchanged",
                        fmt::FormatStatus::Changed => "changed",
                    },
                });
                // Include the diff when --diff requested so JSON
                // consumers don't lose what the human pretty-printer
                // would have shown (M6).
                if let (Some(d), serde_json::Value::Object(map)) = (&r.diff, &mut o) {
                    map.insert("diff".into(), serde_json::Value::String(d.clone()));
                }
                o
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        for r in &results {
            match r.status {
                fmt::FormatStatus::Unchanged => {}
                fmt::FormatStatus::Changed => match mode {
                    fmt::FormatMode::Write => println!("formatted: {}", r.path.display()),
                    fmt::FormatMode::Check => println!("would format: {}", r.path.display()),
                    fmt::FormatMode::Diff => {
                        if let Some(d) = &r.diff {
                            print!("{d}");
                        }
                    }
                },
            }
        }
        if !any_changed && mode != fmt::FormatMode::Diff {
            println!("All {} file(s) already formatted.", results.len());
        }
    }

    if check && any_changed {
        std::process::exit(1);
    }
    Ok(())
}

pub(crate) fn cmd_context(json: bool, slug: &str, write: bool) -> Result<()> {
    let root = find_root();
    let bundle = context::build(&root, slug)?;
    let (filename, content) = if json {
        ("context.json", context::render_json(&bundle)?)
    } else {
        ("context.md", context::render_markdown(&bundle))
    };
    if write {
        let path = context::write_artifact(&root, slug, &[filename], &content)?;
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "path": path.to_string_lossy(),
                    "slug": slug,
                }))?
            );
        } else {
            println!("wrote {}", path.display());
        }
    } else {
        print!("{content}");
    }
    Ok(())
}

pub(crate) fn cmd_prompt(json: bool, template: &str, slug: &str, write: bool) -> Result<()> {
    let root = find_root();
    let bundle = context::build(&root, slug)?;
    let tpl = context::load_template(&root, template)?;
    let rendered = context::render_prompt(&tpl, &bundle);
    if write {
        let segments = context::prompt_cache_segments(template)?;
        let segs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
        let path = context::write_artifact(&root, slug, &segs, &rendered)?;
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "path": path.to_string_lossy(),
                    "slug": slug,
                    "template": template,
                    "rendered": rendered,
                }))?
            );
        } else {
            println!("wrote {}", path.display());
        }
    } else if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "slug": slug,
                "template": template,
                "rendered": rendered,
            }))?
        );
    } else {
        print!("{rendered}");
    }
    Ok(())
}

pub(crate) fn cmd_config(json: bool, action: ConfigAction) -> Result<()> {
    let root = find_root();
    match action {
        ConfigAction::Path => {
            let path = config::path(&root);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({ "path": path.to_string_lossy() })
                    )?
                );
            } else {
                println!("{}", path.display());
            }
        }
        ConfigAction::Show => {
            let report = config::show(&root)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let file_state = if report.exists {
                    ""
                } else {
                    " (missing; built-in defaults apply)"
                };
                println!("{}{file_state}", report.path);
                for (key, resolved) in report.values {
                    let value = serde_json::to_string_pretty(&resolved.value)?;
                    if value.contains('\n') {
                        println!(
                            "{key} [{}]:\n  {}",
                            resolved.source.as_str(),
                            value.replace('\n', "\n  ")
                        );
                    } else {
                        println!("{key} [{}]: {value}", resolved.source.as_str());
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn find_root() -> PathBuf {
    if let Some(Some(p)) = ROOT_OVERRIDE.get() {
        if !p.join("issues").is_dir() {
            eprintln!(
                "Error: --root {} does not contain an issues/ directory",
                p.display()
            );
            std::process::exit(1);
        }
        return p.clone();
    }
    repo::find_repo_root(None)
}

pub(crate) fn load() -> Vec<models::Issue> {
    let root = find_root();
    repo::load_issues(&root)
}

/// The implicit folder scope shared by `list` and `export`: open issues
/// only by default, unless `--all` (no filter), `--closed`, or a
/// positional query (caller opts into scoping it themselves) is given.
pub(crate) fn folder_default_filter(
    all: bool,
    closed: bool,
    has_query: bool,
) -> Option<&'static str> {
    if all {
        None
    } else if closed {
        Some("closed")
    } else if has_query {
        None
    } else {
        Some("open")
    }
}

/// Folder scope for `list`. Explicit `--all`/`--closed` stay
/// authoritative (so `list --closed --status done` still restricts to
/// the closed folder rather than silently dropping the flag). Absent
/// those, a positional query OR a positively-pinned `status:`/`folder:`
/// term disables the implicit open-only default — otherwise
/// `list --status done` would AND `folder:open` against a closing
/// status and match nothing (the `list-status-done` bug). A positional
/// query already disables the default regardless of its contents (a
/// *negated* `-status:wontfix` term does not scope-expand on its own,
/// but supplying it positionally does, same as any positional query).
/// This routes through [`folder_default_filter`] so `--all`/`--closed`
/// precedence is encoded in exactly one place.
pub(crate) fn list_folder_filter(
    q: &query::Query,
    all: bool,
    closed: bool,
    has_query: bool,
) -> Option<&'static str> {
    let scope_pinned = q.has_positive_field(query::FieldName::Status)
        || q.has_positive_field(query::FieldName::Folder);
    folder_default_filter(all, closed, has_query || scope_pinned)
}
