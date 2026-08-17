use super::*;

pub(crate) fn json_error_value(
    code: &str,
    message: &str,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut err = serde_json::Map::new();
    err.insert("code".into(), serde_json::Value::String(code.to_string()));
    err.insert(
        "message".into(),
        serde_json::Value::String(message.to_string()),
    );
    if let serde_json::Value::Object(map) = extra {
        for (k, v) in map {
            err.insert(k, v);
        }
    }
    serde_json::json!({ "error": serde_json::Value::Object(err) })
}

/// Classify an anyhow error that bubbled up to `main` into a stable
/// `--json` envelope error code. Most failures are opaque and render as
/// the generic `command-failed`, but a mutate-layer `NotFound` — raised
/// on a missing slug by the eight mutate-layer write verbs (`update`,
/// `close`, `set`, `note`, `check`, `label`, `depend`, `body set`) — is
/// threaded through as the typed `MutateError` (see the `.map_err` sites
/// that call `anyhow::Error::new`) so it maps to the same `not-found`
/// code the read paths emit. Agents branch on the code instead of
/// string-matching "not found" on a generic `command-failed`. Note this
/// covers only verbs whose target is resolved in the mutate layer; verbs
/// that resolve the slug earlier at the repo layer (e.g. `triage`) still
/// surface a missing slug as `command-failed`.
///
/// We scan the whole `.chain()` rather than only the outermost error:
/// `anyhow`'s own `.context()` wrappers already preserve downcasting, but
/// searching the chain also survives an intervening non-`anyhow` wrapper
/// and states the intent — find this type anywhere it appears.
pub(crate) fn bubbled_error_code(e: &anyhow::Error) -> &'static str {
    match e
        .chain()
        .find_map(|cause| cause.downcast_ref::<mutate::MutateError>())
    {
        Some(mutate::MutateError::NotFound) => "not-found",
        // The intrinsic intake invariants (and any `transitions.yaml`
        // rule) are enforced inside the shared under-lock path, so a
        // generic `set`/`update --status` that trips one bubbles a
        // `TransitionViolation`. Classify it with the same
        // `transition-illegal` code the first-class `intake` verbs emit
        // so agents branch on one code regardless of which verb they
        // used. (The bubble path still exits 1; the `intake` surface maps
        // the refused-but-actionable case to exit 2 itself.)
        Some(mutate::MutateError::TransitionViolation(_)) => "transition-illegal",
        _ => "command-failed",
    }
}

/// Process exit code for an error bubbling to `main` under `--json`. A
/// transition violation is refused-but-actionable → `2`, matching the
/// first-class `intake` verbs; everything else is `1`. Keeps the same
/// error (e.g. an intrinsic intake invariant) from reporting a different
/// exit status depending on whether it was reached via `intake …` or a
/// generic `set`/`update --status`.
pub(crate) fn bubbled_exit_code(e: &anyhow::Error) -> i32 {
    match e
        .chain()
        .find_map(|cause| cause.downcast_ref::<mutate::MutateError>())
    {
        Some(mutate::MutateError::TransitionViolation(_)) => 2,
        _ => 1,
    }
}

/// Print the shared `--json` error object to stderr.
pub(crate) fn emit_json_error(code: &str, message: &str, extra: serde_json::Value) {
    eprintln!(
        "{}",
        serde_json::to_string_pretty(&envelope::error(code, message, extra)).unwrap_or_default()
    );
}

/// Fail a command under the unified output contract. With `--json` it
/// emits the shared `{"error":{…}}` object to stderr; otherwise it prints
/// the historical `Error: <message>` line. Exits with `code` (1 = generic
/// failure / not-found, 2 = refused-but-actionable). Used by the explicit
/// `process::exit` sites so they honour `--json` like the bubble-up path
/// in `main`.
pub(crate) fn fail(
    json: bool,
    code: i32,
    err_code: &str,
    message: &str,
    extra: serde_json::Value,
) -> ! {
    if json {
        emit_json_error(err_code, message, extra);
    } else {
        eprintln!("Error: {message}");
    }
    std::process::exit(code);
}

/// Convenience aliases we deliberately expose (or accept), mapped to the
/// canonical subcommand they resolve to. Used to enrich clap's
/// "unrecognized subcommand" errors: when a near-miss lands on one of
/// these aliases, the tip names the canonical verb the user can rely on.
pub(crate) const SUBCOMMAND_ALIASES: &[(&str, &str)] =
    &[("new", "create"), ("ls", "list"), ("dups", "duplicates")];

/// The subcommand path clap was parsing when it produced `err`, taken
/// from the error's `Usage` context line (`Usage: <bin> <sub...>
/// [OPTIONS] …`). Empty = top level; `["body"]` = inside the `body`
/// group. Derived from clap's own usage rather than argv so it is
/// immune to option-value ordering (`--root=body foo`) and to the binary
/// being renamed.
pub(crate) fn usage_command_path(err: &clap::Error) -> Vec<String> {
    use clap::error::{ContextKind, ContextValue};
    let usage = match err.get(ContextKind::Usage) {
        Some(ContextValue::StyledStr(s)) => s.to_string(),
        Some(ContextValue::String(s)) => s.clone(),
        // DisplayHelp does not carry a Usage context in clap 4.6, even
        // though its rendered output includes the same usage line.
        _ => err.to_string(),
    };
    let line = usage
        .lines()
        .find(|line| line.trim_start().starts_with("Usage:"))
        .unwrap_or("");
    let after = line.trim().trim_start_matches("Usage:").trim();
    let mut toks = after.split_whitespace();
    let _bin = toks.next(); // program name
                            // Subcommand chain runs until the first placeholder (`[OPTIONS]`,
                            // `<COMMAND>`, `[ARGS]`, …).
    toks.take_while(|t| !t.starts_with('[') && !t.starts_with('<'))
        .map(str::to_string)
        .collect()
}

/// Build a routing tip for an `unrecognized subcommand` error, or `None`
/// when the error is something else or no better form is known.
///
/// Two cases, in priority order:
///   1. `body <slug>` — a bare slug passed where a `body` sub-subcommand
///      is expected. `body` is a group; the op is `body set <slug>`. Gated
///      on the error actually originating under the `body` command (via
///      `usage_command_path`), so an option value that happens to equal
///      `body` (`--root=body foo`) never triggers it.
///   2. A *top-level* alias near-miss — clap suggested (or the user typed)
///      a known convenience alias; name the canonical subcommand it
///      resolves to. Gated to the top level so an unknown token inside a
///      subcommand (`body ls`) is never rerouted to a top-level verb.
///
/// Pure over `err` so it is unit-testable without spawning a process.
pub(crate) fn subcommand_error_hint(err: &clap::Error) -> Option<String> {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    if err.kind() != ErrorKind::InvalidSubcommand {
        return None;
    }

    let invalid = match err.get(ContextKind::InvalidSubcommand) {
        Some(ContextValue::String(s)) => Some(s.clone()),
        _ => None,
    };
    let path = usage_command_path(err);

    // Case 1: `body <invalid>` where <invalid> is a bare slug.
    if path == ["body"] {
        if let Some(inv) = &invalid {
            return Some(format!(
                "`body` is a subcommand group — did you mean `issuectl body set {inv}`?"
            ));
        }
    }

    // Case 2: a top-level near-miss (or exact alias) that maps to a
    // canonical verb. clap may offer several suggestions (e.g. `creat` →
    // 'rename', 'ready', 'create'); scan all of them, plus the invalid
    // token itself, and prefer the one that resolves to a canonical
    // subcommand.
    if path.is_empty() {
        let mut candidates: Vec<String> = Vec::new();
        match err.get(ContextKind::SuggestedSubcommand) {
            Some(ContextValue::String(s)) => candidates.push(s.clone()),
            Some(ContextValue::Strings(v)) => candidates.extend(v.iter().cloned()),
            _ => {}
        }
        if let Some(inv) = &invalid {
            candidates.push(inv.clone());
        }
        for candidate in &candidates {
            if let Some((alias, canonical)) =
                SUBCOMMAND_ALIASES.iter().find(|(a, _)| a == candidate)
            {
                return Some(format!(
                    "`{alias}` is an alias for `{canonical}` — run `issuectl {canonical} …`."
                ));
            }
        }
    }
    None
}

/// Return examples for the commands agents most often use. The argv arrays are
/// intentionally shell-independent: callers can pass them directly to a
/// process launcher without reparsing a shell string.
pub(crate) fn help_examples(path: &[String]) -> Vec<help::HelpExample> {
    let examples = match path.get(1).map(String::as_str) {
        None => vec![
            ("List open issues", &["issuectl", "list"][..]),
            (
                "Show an issue as JSON",
                &["issuectl", "show", "login-loop", "--json"][..],
            ),
            (
                "Create a bug",
                &[
                    "issuectl",
                    "create",
                    "--type",
                    "bug",
                    "--title",
                    "Login loop",
                ][..],
            ),
            (
                "Update an issue status",
                &[
                    "issuectl",
                    "update",
                    "login-loop",
                    "--status",
                    "in-progress",
                ][..],
            ),
        ],
        Some("create") => vec![(
            "Create a bug with a descriptive title",
            &[
                "issuectl",
                "create",
                "--type",
                "bug",
                "--title",
                "Login loop",
            ][..],
        )],
        Some("list") => vec![(
            "List open bugs as JSON",
            &["issuectl", "list", "--type", "bug", "--json"][..],
        )],
        Some("show") => vec![(
            "Show one issue as JSON",
            &["issuectl", "show", "login-loop", "--json"][..],
        )],
        Some("update") => vec![(
            "Set an issue status",
            &[
                "issuectl",
                "update",
                "login-loop",
                "--status",
                "in-progress",
            ][..],
        )],
        Some("close") => vec![(
            "Close an issue as fixed",
            &["issuectl", "close", "login-loop", "--status", "fixed"][..],
        )],
        Some("search") => vec![("Search issue text", &["issuectl", "search", "login"][..])],
        Some("config") => vec![(
            "Show effective configuration",
            &["issuectl", "config", "show", "--json"][..],
        )],
        Some("skill") => vec![(
            "List bundled skills",
            &["issuectl", "skill", "list", "--json"][..],
        )],
        _ => Vec::new(),
    };
    examples
        .into_iter()
        .map(|(description, argv)| help::HelpExample {
            description: description.to_string(),
            argv: argv.iter().map(|part| (*part).to_string()).collect(),
        })
        .collect()
}

pub(crate) fn help_argument(arg: &clap::Arg) -> help::HelpArgument {
    let takes_values = arg.get_action().takes_values();
    help::HelpArgument {
        name: arg.get_id().to_string(),
        short: arg.get_short().map(|short| format!("-{short}")),
        long: arg.get_long().map(|long| format!("--{long}")),
        required: arg.is_required_set(),
        global: arg.is_global_set(),
        description: arg
            .get_long_help()
            .or_else(|| arg.get_help())
            .map(ToString::to_string),
        value_names: if takes_values {
            arg.get_value_names()
                .unwrap_or_default()
                .iter()
                .map(ToString::to_string)
                .collect()
        } else {
            Vec::new()
        },
        default: if takes_values {
            arg.get_default_values()
                .iter()
                .map(|value| value.to_string_lossy().into_owned())
                .collect()
        } else {
            Vec::new()
        },
        possible_values: if takes_values {
            arg.get_possible_values()
                .iter()
                .map(|value| value.get_name().to_string())
                .collect()
        } else {
            Vec::new()
        },
        env: arg
            .get_env()
            .map(|value| value.to_string_lossy().into_owned()),
    }
}

/// Convert clap's command metadata into the core-owned serializable model.
pub(crate) fn help_document(
    root: &ClapCommand,
    command: &ClapCommand,
    path: Vec<String>,
) -> help::HelpDocument {
    let mut document = help::HelpDocument::new(path, command.get_name().to_string());
    document.description = command
        .get_long_about()
        .or_else(|| command.get_about())
        .map(ToString::to_string);
    document.subcommands = command
        .get_subcommands()
        .filter(|subcommand| !subcommand.is_hide_set())
        .map(|subcommand| help::HelpSubcommand {
            name: subcommand.get_name().to_string(),
            aliases: subcommand
                .get_name_and_visible_aliases()
                .into_iter()
                .skip(1)
                .map(str::to_string)
                .collect(),
            description: subcommand
                .get_long_about()
                .or_else(|| subcommand.get_about())
                .map(ToString::to_string),
        })
        .collect();
    for arg in command.get_arguments().filter(|arg| !arg.is_hide_set()) {
        if arg.is_positional() {
            document.args.push(help_argument(arg));
        } else {
            document.flags.push(help_argument(arg));
        }
    }
    if command.get_name() != root.get_name() {
        document.flags.extend(
            root.get_arguments()
                .filter(|arg| arg.is_global_set() && !arg.is_hide_set())
                .map(help_argument),
        );
    }
    document.examples = help_examples(&document.path);
    document
}

/// Print structured help after clap has already recognized a help request.
/// The path comes from clap's usage context, so option values, aliases, and
/// nested subcommands are interpreted by clap rather than by a second parser.
pub(crate) fn print_json_help(err: &clap::Error) {
    let root = Cli::command();
    let command_path = usage_command_path(err);
    let mut selected = &root;
    for segment in &command_path {
        let Some(subcommand) = selected
            .get_subcommands()
            .find(|candidate| candidate.get_name() == segment)
        else {
            break;
        };
        selected = subcommand;
    }
    let mut path = vec![root.get_name().to_string()];
    path.extend(command_path);
    println!(
        "{}",
        help::render_json(&help_document(&root, selected, path))
            .expect("machine-readable help document must serialize")
    );
}

/// Whether `--json` was supplied as an option, rather than as a positional
/// value after clap's `--` separator.
pub(crate) fn argv_has_json_flag() -> bool {
    std::env::args()
        .skip(1)
        .take_while(|arg| arg != "--")
        .any(|arg| arg == "--json")
}

pub(crate) fn run() -> Result<()> {
    JSON_OUTPUT.store(argv_has_json_flag(), std::sync::atomic::Ordering::Relaxed);
    // Parse manually instead of `Cli::parse()` so clap's own usage
    // errors honour the `--json` contract too. By default clap prints
    // free-form text to stderr and exits 2 — an agent that always passes
    // `--json` would get un-parseable output and an exit code that
    // collides with our "refused-but-actionable" 2. We prescan argv for
    // `--json` (the flag itself can't have been parsed yet) and, when
    // set, render usage errors as the shared error envelope with exit 1,
    // reserving 2 for genuine refused-but-actionable outcomes. Help and
    // version requests are not errors — let clap print them normally.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind;
            let wants_json = argv_has_json_flag();
            if wants_json && e.kind() == ErrorKind::DisplayHelp {
                print_json_help(&e);
                return Ok(());
            }
            // Route unrecognized-subcommand errors to a form the user can
            // actually run (e.g. `body <slug>` → `body set <slug>`, or an
            // alias near-miss → its canonical verb). See
            // `subcommand_error_hint`.
            let hint = subcommand_error_hint(&e);
            if wants_json
                && !matches!(
                    e.kind(),
                    ErrorKind::DisplayHelp
                        | ErrorKind::DisplayVersion
                        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                )
            {
                let mut message = e.to_string().trim_end().to_string();
                if let Some(h) = &hint {
                    message.push_str("\n\ntip: ");
                    message.push_str(h);
                }
                emit_json_error("usage-error", &message, serde_json::Value::Null);
                std::process::exit(1);
            }
            if let Some(h) = hint {
                // Print clap's own rendered error first (preserving its
                // usage block), then append our routing tip. `hint` is
                // only ever `Some` for `InvalidSubcommand`, whose exit code
                // is clap's usage code (2) — mirror `e.exit()` so scripts
                // see the same code they did before the tip existed.
                let _ = e.print();
                eprintln!("\ntip: {h}");
                std::process::exit(e.exit_code());
            }
            e.exit();
        }
    };
    let json_output = cli.json;
    JSON_OUTPUT.store(json_output, std::sync::atomic::Ordering::Relaxed);
    ROOT_OVERRIDE.set(cli.root).ok();

    // Unified `--json` error contract: any error that bubbles up to
    // `main` is rendered as the shared `{"error":{code,message}}` object
    // on stderr (exit 1) so agents parse one shape regardless of which
    // command failed. Without `--json` we return the error unchanged so
    // anyhow's default `Error: …` rendering (and existing tests) are
    // preserved byte-for-byte.
    let result = dispatch(cli.command, json_output);
    if json_output {
        if let Err(e) = result {
            emit_json_error(
                bubbled_error_code(&e),
                &format!("{e:#}"),
                serde_json::Value::Null,
            );
            std::process::exit(bubbled_exit_code(&e));
        }
        return Ok(());
    }
    result
}

pub(crate) fn cmd_version(json: bool) -> Result<()> {
    let payload = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "commit": option_env!("ISSUECTL_GIT_COMMIT"),
        "schema_version": envelope::CLI_SCHEMA_VERSION,
        "supported_schemas": [schema::SUPPORTED_SCHEMA_VERSION],
        "skills": skill::skill_versions(),
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("issuectl {}", env!("CARGO_PKG_VERSION"));
    }
    Ok(())
}

pub(crate) fn dispatch(command: Command, json_output: bool) -> Result<()> {
    match command {
        Command::Version => cmd_version(json_output),
        Command::Config { action } => cmd_config(json_output, action),
        Command::List {
            query,
            assignee,
            issue_type,
            priority,
            status,
            epic,
            label,
            all,
            closed,
        } => cmd_list(
            json_output,
            query,
            assignee,
            issue_type,
            priority,
            status,
            epic,
            label,
            all,
            closed,
        ),
        Command::Show { slug } => cmd_show(json_output, &slug),
        Command::Ready { slug } => cmd_ready(json_output, &slug),
        Command::Open { slug, dir, editor } => cmd_open(json_output, &slug, dir, editor),
        Command::Attach { slug, files } => cmd_attach(json_output, &slug, files),
        Command::Search { query, all } => cmd_search(json_output, &query, all),
        Command::Stats => cmd_stats(json_output),
        Command::Duplicates {
            slug,
            threshold,
            all,
        } => cmd_duplicates(json_output, slug.as_deref(), threshold, all),
        Command::Create {
            issue_type,
            title_pos,
            title_flag,
            slug,
            slug_random,
            reporter,
            assignee,
            owner,
            priority,
            epic,
            labels,
            related,
            lane,
            lane_seq,
            add_collision,
            source,
            description,
            body_file,
            custom_fields,
            check_duplicates,
            inbox,
        } => {
            // `--body-file` is a body source that conflicts with
            // `--description`/`--body` at the clap layer, so at most one
            // is set. Reading the file (or stdin for `-`) here — before
            // `do_new` — keeps all I/O + the input cap in the CLI layer
            // and lets the resolved markdown flow through the same
            // flock/schema write path as an inline `--description`.
            let description = match body_file {
                Some(path) => Some(read_body_file_arg(&path)?),
                None => description,
            };
            cmd_new(
                json_output,
                NewArgs {
                    issue_type,
                    // The clap `title_input` group (required + mutually
                    // exclusive) guarantees exactly one of these at parse
                    // time; the `ok_or_else` is a defensive net so a future
                    // group-wiring regression surfaces as an error, not a
                    // panic (the `Cli::command().debug_assert()` test also
                    // guards the wiring at build time).
                    title: title_pos.or(title_flag).ok_or_else(|| {
                        anyhow::anyhow!(
                            "internal: clap `title_input` group did not enforce a title source"
                        )
                    })?,
                    slug,
                    slug_random,
                    reporter,
                    assignee,
                    owner,
                    priority,
                    epic,
                    labels,
                    related,
                    lane,
                    lane_seq,
                    collision: add_collision,
                    source,
                    description,
                    custom_fields,
                    // `create` never files into a reception state — status is
                    // fixed at `open`. Intake filing goes through
                    // `issuectl intake file`.
                    status: None,
                    inbox,
                },
                check_duplicates,
            )
        }
        Command::Update {
            slug,
            status,
            issue_type,
            assignee,
            no_reporter,
            no_assignee,
            owner,
            no_owner,
            priority,
            epic,
            no_epic,
            lane,
            no_lane,
            lane_seq,
            no_lane_seq,
            add_collision,
            remove_collision,
            add_labels,
            remove_labels,
            add_related,
            remove_related,
            add_blocked_by,
            remove_blocked_by,
            add_commits,
            custom_fields,
            clear_fields,
            description,
            body_file,
            expected_version,
        } => {
            // `--body-file` conflicts with `--description`/`--body` at the
            // clap layer, so at most one is set. Resolve the file (or stdin
            // for `-`) here — before `cmd_update` — so all body I/O + the
            // input cap stay in the CLI layer and the resolved markdown
            // flows through the same flock/schema write path as an inline
            // `--description`. Mirrors the `create` command's body handling.
            let set_body = match body_file {
                Some(path) => Some(read_body_file_arg(&path)?),
                None => description,
            };
            cmd_update(
                json_output,
                UpdateArgs {
                    slug,
                    status,
                    issue_type,
                    assignee,
                    no_reporter,
                    no_assignee,
                    owner,
                    no_owner,
                    priority,
                    epic,
                    no_epic,
                    lane,
                    no_lane,
                    lane_seq,
                    no_lane_seq,
                    add_collision,
                    remove_collision,
                    add_labels,
                    remove_labels,
                    add_related,
                    remove_related,
                    add_blocked_by,
                    remove_blocked_by,
                    add_commits,
                    custom_fields,
                    clear_fields,
                    set_body,
                    expected_version,
                },
            )
        }
        Command::Close {
            slug,
            status,
            author,
            comment,
            commits,
            stamp,
            expected_version,
        } => cmd_close(
            json_output,
            &slug,
            status,
            author,
            comment,
            commits,
            stamp,
            expected_version,
        ),
        Command::Rename {
            old_slug,
            new_slug,
            dry_run,
        } => cmd_rename(json_output, &old_slug, &new_slug, dry_run),
        Command::Stale { days } => cmd_stale(json_output, days),
        Command::Archive {
            older_than,
            dry_run,
        } => cmd_archive(json_output, older_than, dry_run),
        Command::Note {
            slug,
            author,
            message,
            message_flag,
            stdin,
            from_file,
            decision,
            agent_run,
            dry_run,
            expected_version,
        } => cmd_note(
            json_output,
            &slug,
            &author,
            message,
            message_flag,
            stdin,
            from_file,
            decision,
            agent_run,
            dry_run,
            expected_version,
        ),
        Command::Set {
            slug,
            field,
            value,
            clear,
            dry_run,
            expected_version,
        } => cmd_set(
            json_output,
            &slug,
            &field,
            value,
            clear,
            dry_run,
            expected_version,
        ),
        // Convenience wrapper: `assign <slug> <user>` is exactly
        // `set <slug> assignee <user>` (and `--clear` mirrors
        // `set --clear`). Route through the same handler so validation,
        // idempotency, and the `--json`/`--expected-version` contract are
        // identical — no new mutation verb or storage semantics.
        Command::Assign {
            slug,
            user,
            clear,
            dry_run,
            expected_version,
        } => cmd_set(
            json_output,
            &slug,
            "assignee",
            user,
            clear,
            dry_run,
            expected_version,
        ),
        Command::Check {
            slug,
            task,
            dry_run,
            expected_version,
        } => cmd_check(json_output, &slug, &task, dry_run, expected_version),
        Command::Label {
            slug,
            op,
            label,
            add,
            remove,
            dry_run,
            expected_version,
        } => {
            let (op, label) = resolve_label_target(op, label, add, remove)?;
            cmd_label(json_output, &slug, op, &label, dry_run, expected_version)
        }
        Command::Apply { patch, dry_run } => cmd_apply(json_output, &patch, dry_run),
        Command::Bulk {
            query,
            set,
            clear,
            add_labels,
            remove_labels,
            add_related,
            remove_related,
            dry_run,
        } => cmd_bulk(
            json_output,
            &query,
            BulkSpec {
                set,
                clear,
                add_labels,
                remove_labels,
                add_related,
                remove_related,
            },
            dry_run,
        ),
        Command::Body { action } => match action {
            BodyAction::Set {
                slug,
                stdin,
                from_file,
                expected_version,
            } => cmd_body_set(json_output, &slug, stdin, from_file, expected_version),
        },
        Command::Init {
            agent,
            with_hooks,
            with_merge_driver,
            force,
        } => {
            let root = find_root();
            let opts = init_cmd::InitOptions {
                agent: agent.into(),
                with_hooks,
                with_merge_driver,
                force,
                // Dual-home Claude skills into pi.dev's skill dir; None = no HOME.
                pi_root: skill::pi_skills_root(),
            };
            init_cmd::run(&root, opts, json_output)
        }
        Command::Doctor { fix, verbose } => doctor::run(&find_root(), fix, json_output, verbose),
        Command::Hooks { action } => match action {
            HooksAction::Install { uninstall, force } => hooks::run(&find_root(), uninstall, force),
        },
        Command::SyncCommits {
            range,
            no_branch_fallback,
            dry_run,
        } => cmd_sync_commits(json_output, range, no_branch_fallback, dry_run),
        Command::Agents { action } => match action {
            AgentsAction::Init { force } => agents::run_init(&find_root(), force, json_output),
        },
        Command::Skill { action } => match action {
            SkillAction::List => cmd_skill_list(json_output),
            SkillAction::Install { agent, force } => cmd_skill_install(&agent, force),
            SkillAction::Print { agent } => cmd_skill_print(&agent),
            SkillAction::PiStatus => cmd_skill_pi_status(json_output),
            SkillAction::PiPrune { force } => cmd_skill_pi_prune(json_output, force),
        },
        Command::Fmt { slugs, check, diff } => cmd_fmt(json_output, slugs, check, diff),
        Command::MergeDriver {
            base,
            ours,
            theirs,
            output,
        } => {
            let code = merge_driver::run(&merge_driver::MergeArgs {
                base,
                ours,
                theirs,
                output,
            })?;
            std::process::exit(code);
        }
        Command::Context { slug, write } => cmd_context(json_output, &slug, write),
        Command::Prompt {
            template,
            slug,
            write,
        } => cmd_prompt(json_output, &template, &slug, write),
        Command::InstallMergeDriver { apply } => {
            let root = find_root();
            merge_driver::install(&root, apply)
        }
        Command::Export {
            format,
            query,
            all,
            closed,
        } => cmd_export(json_output, format, query, all, closed),
        Command::Depend { action } => match action {
            DependAction::Add {
                slug,
                blocked_by,
                expected_version,
            } => cmd_depend(json_output, &slug, blocked_by, true, expected_version),
            DependAction::Remove {
                slug,
                blocked_by,
                expected_version,
            } => cmd_depend(json_output, &slug, blocked_by, false, expected_version),
        },
        Command::Dag { reservations } => cmd_dag(json_output, reservations),
        Command::Epic { action } => match action {
            EpicAction::Tree { slug } => cmd_epic_tree(json_output, slug.as_deref()),
        },
        Command::Workload => cmd_workload(json_output),
        Command::Burndown { cycle } => cmd_burndown(json_output, &cycle),
        Command::Cycle { action } => match action {
            CycleAction::Current => cmd_cycle_current(json_output),
            CycleAction::Plan { name, all, closed } => {
                cmd_cycle_plan(json_output, &name, all, closed)
            }
            CycleAction::Status { name, all } => {
                cmd_cycle_status(json_output, name.as_deref(), all)
            }
        },
        Command::Schedule { action } => match action {
            ScheduleAction::List => cmd_schedule_list(json_output),
            ScheduleAction::Run { dry_run } => cmd_schedule_run(json_output, dry_run),
        },
        Command::Import { source } => match source {
            ImportSource::Json { file, default_type } => {
                cmd_import_json(json_output, &file, &default_type)
            }
            ImportSource::Github {
                repo,
                state,
                limit,
                default_type,
            } => cmd_import_github(json_output, &repo, &state, limit, &default_type),
        },
        Command::Triage { slug } => cmd_triage(json_output, slug),
        Command::Pick { query, all, first } => cmd_pick(json_output, query, all, first),
        Command::Completions { shell } => cmd_completions(shell),
        Command::CompleteValues { kind } => cmd_complete_values(kind),
        Command::ScanTodos { create_inbox } => cmd_scan_todos(json_output, create_inbox),
        Command::Activity { since, limit } => cmd_activity(json_output, since, limit),
        Command::Timeline { slug } => cmd_timeline(json_output, &slug),
        Command::Changelog { range } => cmd_changelog(json_output, &range),
        Command::Metrics { since } => cmd_metrics(json_output, since),
        Command::Intake { action } => dispatch_intake(json_output, action),
    }
}

/// Thin router for the `intake` subcommand group. Each arm builds args
/// and delegates to a domain function in `mutate::intake`, then renders
/// the outcome under the shared `--json` contract.
pub(crate) fn dispatch_intake(json: bool, action: IntakeAction) -> Result<()> {
    match action {
        IntakeAction::File {
            issue_type,
            title,
            body,
            body_file,
            reporter,
            provenance,
            provenance_detail,
            source_ref,
            priority,
            slug,
            labels,
            fields,
        } => {
            let body = match body_file {
                Some(path) => Some(read_body_file_arg(&path)?),
                None => body,
            };
            cmd_intake_file(
                json,
                mutate::intake::FileRequest {
                    issue_type,
                    title,
                    body,
                    reporter,
                    provenance,
                    provenance_detail,
                    source_ref,
                    priority,
                    slug,
                    labels,
                    fields,
                },
            )
        }
        IntakeAction::Queue {
            issue_type,
            provenance,
            needs_analysis,
            state,
        } => cmd_intake_queue(json, issue_type, provenance, needs_analysis, state),
        IntakeAction::Show { slug } => cmd_intake_show(json, &slug),
        IntakeAction::Accept {
            slug,
            assignee,
            priority,
        } => intake_render(
            json,
            &slug,
            mutate::intake::accept(&find_root(), &slug, assignee, priority),
        ),
        IntakeAction::Defer {
            slug,
            reason,
            until,
        } => intake_render(
            json,
            &slug,
            mutate::intake::defer(&find_root(), &slug, &reason, until),
        ),
        IntakeAction::NeedInfo { slug, reason } => intake_render(
            json,
            &slug,
            mutate::intake::need_info(&find_root(), &slug, &reason),
        ),
        IntakeAction::Reject { slug, reason, kind } => intake_render(
            json,
            &slug,
            mutate::intake::reject(&find_root(), &slug, kind.into(), &reason),
        ),
        IntakeAction::CannotReproduce { slug, reason } => intake_render(
            json,
            &slug,
            mutate::intake::cannot_reproduce(&find_root(), &slug, &reason),
        ),
        IntakeAction::Duplicate { slug, of } => intake_render(
            json,
            &slug,
            mutate::intake::duplicate(&find_root(), &slug, &of),
        ),
        IntakeAction::Obsolete {
            slug,
            reason,
            superseded_by,
        } => intake_render(
            json,
            &slug,
            mutate::intake::obsolete(&find_root(), &slug, &reason, superseded_by),
        ),
        IntakeAction::Retype { slug, to } => intake_render(
            json,
            &slug,
            mutate::intake::retype(&find_root(), &slug, &to),
        ),
        IntakeAction::Reopen { slug, to, reason } => intake_render(
            json,
            &slug,
            mutate::intake::reopen(&find_root(), &slug, to, &reason),
        ),
        IntakeAction::Withdraw { slug, reason } => intake_render(
            json,
            &slug,
            mutate::intake::withdraw(&find_root(), &slug, &reason),
        ),
        IntakeAction::Migrate { apply } => cmd_intake_migrate(json, apply),
    }
}

/// Render a first-class intake transition outcome. On error, maps the
/// `IntakeError` onto its stable `--json` code + exit code via the shared
/// `fail` sink (`-> !`).
pub(crate) fn intake_render(
    json: bool,
    slug: &str,
    r: Result<mutate::UpdateOutcome, mutate::intake::IntakeError>,
) -> Result<()> {
    match r {
        Ok(out) => {
            if json {
                let report = serde_json::json!({
                    "slug": slug,
                    "status": out.issue.status,
                    "dir": out.issue_dir.to_string_lossy(),
                    "version": out.version,
                });
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{slug} → {}", out.issue.status);
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
