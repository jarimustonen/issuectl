use super::*;

#[cfg(test)]
mod tests {
    #[test]
    fn help_document_includes_global_flags_and_new_examples() {
        let root = Cli::command();
        let new = root
            .get_subcommands()
            .find(|command| command.get_name() == "create")
            .unwrap();
        let document = help_document(
            &root,
            new,
            vec!["issuectl".to_string(), "create".to_string()],
        );

        assert!(document
            .flags
            .iter()
            .any(|flag| flag.long.as_deref() == Some("--json") && flag.global));
        assert!(document
            .flags
            .iter()
            .find(|flag| flag.long.as_deref() == Some("--type"))
            .unwrap()
            .possible_values
            .contains(&"bug".to_string()));
        assert_eq!(document.examples[0].argv[0], "issuectl");
    }

    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn create_help_describes_title_derived_slug_policy() {
        let root = Cli::command();
        let create = root
            .get_subcommands()
            .find(|command| command.get_name() == "create")
            .unwrap();
        let document = help_document(
            &root,
            create,
            vec!["issuectl".to_string(), "create".to_string()],
        );
        let description = document
            .description
            .expect("create subcommand must have a long description");

        assert!(description.contains("neither `--slug` nor `--slug-random` is supplied"));
        assert!(description.contains("derive a descriptive 2-3 word kebab slug from the title"));
        assert!(description.contains("collisions get a numeric suffix"));
        assert!(description.contains("`--slug-random` to opt into a random"));
        assert!(description.contains("Titles with no sensible slug fall back to random"));
        assert!(create
            .get_arguments()
            .any(|arg| arg.get_long() == Some("slug")));
        assert!(create
            .get_arguments()
            .any(|arg| arg.get_long() == Some("slug-random")));
    }

    fn fresh_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        tmp
    }

    fn new_args(t: &str, title: &str) -> NewArgs {
        NewArgs {
            issue_type: t.to_string(),
            title: title.to_string(),
            slug: None,
            slug_random: false,
            reporter: None,
            assignee: None,
            owner: None,
            priority: "normal".to_string(),
            epic: None,
            labels: vec![],
            related: vec![],
            source: None,
            description: None,
            custom_fields: vec![],
            lane: None,
            lane_seq: None,
            collision: vec![],
            status: None,
            inbox: false,
        }
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn new_alias_resolves_to_create() {
        let cli =
            Cli::try_parse_from(["issuectl", "new", "--type", "task", "--title", "x"]).unwrap();
        assert!(matches!(cli.command, Command::Create { .. }));
    }

    #[test]
    fn lane_flags_parse_into_create() {
        // `create` mirrors `update`'s lane surface so an issue can be born
        // scheduled in one call: `--lane`, `--lane-seq`, and repeatable
        // `--add-collision` all route into the `Create` variant.
        let cli = Cli::try_parse_from([
            "issuectl",
            "create",
            "--type",
            "feature",
            "--title",
            "x",
            "--lane",
            "cli-fixes",
            "--lane-seq",
            "40",
            "--add-collision",
            "crates/issuectl/src/main.rs",
            "--add-collision",
            "foo/bar.rs",
        ])
        .unwrap();
        match cli.command {
            Command::Create {
                lane,
                lane_seq,
                add_collision,
                ..
            } => {
                assert_eq!(lane.as_deref(), Some("cli-fixes"));
                assert_eq!(lane_seq, Some(40));
                assert_eq!(
                    add_collision,
                    vec!["crates/issuectl/src/main.rs", "foo/bar.rs"]
                );
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn lane_seq_parses_negative_on_create() {
        // `allow_hyphen_values = true` exists so a negative precedence key
        // parses as a value, not a dangling flag.
        let cli = Cli::try_parse_from([
            "issuectl",
            "create",
            "--type",
            "task",
            "--title",
            "x",
            "--lane-seq",
            "-5",
        ])
        .unwrap();
        match cli.command {
            Command::Create { lane_seq, .. } => assert_eq!(lane_seq, Some(-5)),
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn body_flag_is_alias_for_description_on_create() {
        let cli = Cli::try_parse_from([
            "issuectl", "create", "--type", "task", "--title", "x", "--body", "hello",
        ])
        .unwrap();
        match cli.command {
            Command::Create { description, .. } => {
                assert_eq!(description.as_deref(), Some("hello"))
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn body_file_flag_parses_into_body_file_on_create() {
        let cli = Cli::try_parse_from([
            "issuectl",
            "create",
            "--type",
            "task",
            "--title",
            "x",
            "--body-file",
            "notes.md",
        ])
        .unwrap();
        match cli.command {
            Command::Create {
                body_file,
                description,
                ..
            } => {
                assert_eq!(body_file.as_deref(), Some(Path::new("notes.md")));
                assert_eq!(description, None);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn comment_is_visible_alias_for_note() {
        // The natural guess `issuectl comment <slug> …` resolves to the
        // exact same `Command::Note` variant (and thus `cmd_note` handler)
        // as `note`.
        let cli = Cli::try_parse_from(["issuectl", "comment", "sl-ug", "--as", "u", "hi"]).unwrap();
        match cli.command {
            Command::Note { slug, message, .. } => {
                assert_eq!(slug, "sl-ug");
                assert_eq!(message.as_deref(), Some("hi"));
            }
            _ => panic!("expected Note"),
        }
    }

    #[test]
    fn message_flag_parses_into_note() {
        let cli =
            Cli::try_parse_from(["issuectl", "note", "sl-ug", "--as", "u", "--message", "hi"])
                .unwrap();
        match cli.command {
            Command::Note {
                message,
                message_flag,
                ..
            } => {
                assert_eq!(message, None);
                assert_eq!(message_flag.as_deref(), Some("hi"));
            }
            _ => panic!("expected Note"),
        }
    }

    #[test]
    fn body_and_comment_flags_are_aliases_for_message_on_note() {
        // The note family accepts both the `create --body` and `close
        // --comment` spellings, so its advertised shared vocabulary is real.
        for flag in ["--body", "--comment"] {
            let cli = Cli::try_parse_from(["issuectl", "note", "sl-ug", "--as", "u", flag, "hi"])
                .unwrap();
            match cli.command {
                Command::Note { message_flag, .. } => {
                    assert_eq!(message_flag.as_deref(), Some("hi"))
                }
                _ => panic!("expected Note"),
            }
        }
    }

    #[test]
    fn message_and_note_are_aliases_for_comment_on_close() {
        for flag in ["--message", "--note"] {
            let cli = Cli::try_parse_from(["issuectl", "close", "sl-ug", flag, "done"]).unwrap();
            match cli.command {
                Command::Close { comment, .. } => assert_eq!(comment.as_deref(), Some("done")),
                _ => panic!("expected Close"),
            }
        }
    }

    #[test]
    fn body_file_is_visible_alias_for_from_file_on_note() {
        // `--body-file` is a visible alias of `--from-file`, so it lands in
        // the same `from_file` field (matching `create --body-file`).
        let cli = Cli::try_parse_from([
            "issuectl",
            "comment",
            "sl-ug",
            "--as",
            "u",
            "--body-file",
            "note.md",
        ])
        .unwrap();
        match cli.command {
            Command::Note { from_file, .. } => {
                assert_eq!(from_file.as_deref(), Some(Path::new("note.md")));
            }
            _ => panic!("expected Note"),
        }
    }

    #[test]
    fn unknown_note_flag_is_named_by_clap() {
        let err = Cli::try_parse_from([
            "issuectl",
            "note",
            "sl-ug",
            "--as",
            "u",
            "--not-a-note-flag",
            "hi",
        ])
        .map(|_| ())
        .unwrap_err();
        assert!(
            err.to_string().contains("--not-a-note-flag"),
            "usage error must name the offending flag: {err}"
        );
    }

    #[test]
    fn note_positional_and_message_flag_conflict() {
        // The `note_body` arg group makes clap reject two body sources at
        // once — here the positional plus `--message`.
        // `Cli` is not `Debug`, so map the Ok arm away before `unwrap_err`.
        let err = Cli::try_parse_from([
            "issuectl",
            "note",
            "sl-ug",
            "--as",
            "u",
            "hi",
            "--message",
            "hi",
        ])
        .map(|_| ())
        .unwrap_err();
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::ArgumentConflict,
            "expected an argument-conflict usage error, got {err}"
        );
    }

    #[test]
    fn note_positional_and_body_file_conflict() {
        // Positional plus `--body-file` is likewise rejected by the group.
        let err = Cli::try_parse_from([
            "issuectl",
            "note",
            "sl-ug",
            "--as",
            "u",
            "hi",
            "--body-file",
            "note.md",
        ])
        .map(|_| ())
        .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn body_file_accepts_stdin_dash() {
        let cli = Cli::try_parse_from([
            "issuectl",
            "new",
            "--type",
            "task",
            "--title",
            "x",
            "--body-file",
            "-",
        ])
        .unwrap();
        match cli.command {
            Command::Create { body_file, .. } => {
                assert_eq!(body_file.as_deref(), Some(Path::new("-")));
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn body_file_conflicts_with_description() {
        // Mutual exclusion is a clap `conflicts_with`, so combining the two
        // body sources is a usage error caught before any I/O (it maps to
        // the `usage-error` envelope in `fn main`).
        let err = Cli::try_parse_from([
            "issuectl",
            "new",
            "--type",
            "task",
            "--title",
            "x",
            "--body-file",
            "notes.md",
            "--description",
            "inline",
        ])
        .err()
        .unwrap();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn body_file_conflicts_with_body_alias() {
        // The `--body` visible alias shares `description`'s arg id, so the
        // conflict fires against it too.
        let err = Cli::try_parse_from([
            "issuectl",
            "new",
            "--type",
            "task",
            "--title",
            "x",
            "--body-file",
            "notes.md",
            "--body",
            "inline",
        ])
        .err()
        .unwrap();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn read_body_file_arg_strips_only_trailing_whitespace() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notes.md");
        fs::write(&path, "## Notes\n\nsome markdown body\n\n").unwrap();
        let got = read_body_file_arg(&path).unwrap();
        // Trailing newlines gone, no other change.
        assert_eq!(got, "## Notes\n\nsome markdown body");
    }

    #[test]
    fn read_body_file_arg_preserves_leading_whitespace() {
        // A body is a whole document: a file that opens with a 4-space
        // indented code block must survive verbatim (only trailing
        // whitespace is stripped), matching `body set --from-file` and
        // NOT the leading-and-trailing `trim()` the first draft used.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("code.md");
        fs::write(&path, "    let x = 1;\n\nprose\n").unwrap();
        let got = read_body_file_arg(&path).unwrap();
        assert_eq!(got, "    let x = 1;\n\nprose");
    }

    #[test]
    fn read_body_file_arg_rejects_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("blank.md");
        fs::write(&path, "\n\n  \n").unwrap();
        let err = read_body_file_arg(&path).unwrap_err();
        assert!(err.to_string().contains("empty"), "got: {err}");
    }

    #[test]
    fn read_body_file_arg_missing_path_errors_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope.md");
        // A missing path must surface as a clean error (not a panic); the
        // envelope classifies it downstream.
        let err = read_body_file_arg(&missing).unwrap_err();
        assert!(err.to_string().contains("cannot read body"), "got: {err}");
    }

    #[test]
    fn assign_parses_user_and_clear() {
        let cli = Cli::try_parse_from(["issuectl", "assign", "some-slug", "alice"]).unwrap();
        match cli.command {
            Command::Assign {
                slug, user, clear, ..
            } => {
                assert_eq!(slug, "some-slug");
                assert_eq!(user.as_deref(), Some("alice"));
                assert!(!clear);
            }
            _ => panic!("expected Assign"),
        }

        let cli = Cli::try_parse_from(["issuectl", "assign", "some-slug", "--clear"]).unwrap();
        match cli.command {
            Command::Assign { user, clear, .. } => {
                assert!(user.is_none());
                assert!(clear);
            }
            _ => panic!("expected Assign"),
        }

        // A user is required unless --clear is given.
        assert!(Cli::try_parse_from(["issuectl", "assign", "some-slug"]).is_err());
        // --clear conflicts with an explicit user.
        assert!(
            Cli::try_parse_from(["issuectl", "assign", "some-slug", "alice", "--clear"]).is_err()
        );
    }

    #[test]
    fn body_slug_error_hints_body_set() {
        let err = Cli::try_parse_from(["issuectl", "body", "some-slug"])
            .err()
            .expect("expected a parse error");
        let hint = subcommand_error_hint(&err).expect("expected a routing hint");
        assert!(
            hint.contains("body set some-slug"),
            "hint should point at `body set`, was: {hint}"
        );
    }

    #[test]
    fn body_hint_survives_interleaved_global_flag() {
        // `body --json some-slug`: the global `--json` sits between the
        // subcommand and the bad token. A raw argv-adjacency scan would
        // miss it; the usage-context path still fires because clap reports
        // the error as originating under `body`.
        let err = Cli::try_parse_from(["issuectl", "body", "--json", "some-slug"])
            .err()
            .expect("expected a parse error");
        let hint = subcommand_error_hint(&err).expect("expected a routing hint");
        assert!(
            hint.contains("body set some-slug"),
            "hint should point at `body set`, was: {hint}"
        );
    }

    #[test]
    fn body_hint_not_triggered_by_option_value() {
        // `--root=body some-slug`: here `body` is the *value* of `--root`
        // and `some-slug` is the (unknown) top-level subcommand. The hint
        // must NOT claim this is the `body` group — an argv-adjacency scan
        // would have false-positived here.
        let err = Cli::try_parse_from(["issuectl", "--root=body", "some-slug"])
            .err()
            .expect("expected a parse error");
        let hint = subcommand_error_hint(&err);
        assert!(
            hint.as_deref()
                .map(|h| !h.contains("body set"))
                .unwrap_or(true),
            "must not emit a body-set hint for a `--root` value, was: {hint:?}"
        );
    }

    #[test]
    fn near_miss_inside_subcommand_is_not_rerouted() {
        // `body ls`: `ls` is unknown *under* `body`. It must not be
        // rerouted to the top-level `list` alias — that would discard the
        // user's `body` context.
        let err = Cli::try_parse_from(["issuectl", "body", "ls"])
            .err()
            .expect("expected a parse error");
        let hint = subcommand_error_hint(&err);
        assert!(
            hint.as_deref()
                .map(|h| !h.contains("is an alias"))
                .unwrap_or(true),
            "must not reroute an in-`body` token to a top-level alias, was: {hint:?}"
        );
    }

    #[test]
    fn alias_near_miss_routes_to_canonical_verb() {
        // `neew` is a near miss for the visible `new` alias. Its routing
        // hint must name both the alias and its canonical `create` target.
        let err = Cli::try_parse_from(["issuectl", "neew"])
            .err()
            .expect("expected a parse error");
        let hint = subcommand_error_hint(&err)
            .expect("`neew` should route through visible `new` to canonical `create`");
        assert!(
            hint.contains("new"),
            "hint should mention alias, was: {hint}"
        );
        assert!(
            hint.contains("issuectl create"),
            "hint should name canonical `create`, was: {hint}"
        );
    }

    #[test]
    fn unrelated_bad_subcommand_has_no_hint() {
        let err = Cli::try_parse_from(["issuectl", "zzzzzzzzzz"])
            .err()
            .expect("expected a parse error");
        assert!(subcommand_error_hint(&err).is_none());
    }

    /// Guards against `SUBCOMMAND_ALIASES` drifting from the actual clap
    /// wiring: every entry must be a real alias (visible or hidden) of its
    /// named canonical subcommand. Without this, the near-miss tip could
    /// advertise an alias the CLI does not actually accept.
    #[test]
    fn subcommand_aliases_are_all_wired() {
        use clap::CommandFactory;
        let cmd = Cli::command();
        for (alias, canonical) in SUBCOMMAND_ALIASES {
            let sub = cmd
                .get_subcommands()
                .find(|s| s.get_name() == *canonical)
                .unwrap_or_else(|| panic!("no subcommand named `{canonical}`"));
            let wired = sub.get_all_aliases().any(|a| a == *alias);
            assert!(
                wired,
                "`{alias}` is listed in SUBCOMMAND_ALIASES → `{canonical}` but is not a clap alias of it"
            );
        }
    }

    /// clap's own internal-consistency check: catches invalid arg IDs,
    /// duplicate names, and — critically for `create` — a `title_input`
    /// group that references a renamed/removed field. This is the
    /// build-time backstop for the `dispatch` arm that merges
    /// `title_pos`/`title_flag` and errors if neither is present.
    #[test]
    fn cli_definition_is_internally_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    /// The `create` title group must stay `required` (so "neither" is
    /// rejected) and mutually exclusive (so "both" is rejected). A
    /// refactor that drops `.required(true)` would compile and pass the
    /// happy-path tests while letting a title-less `create` reach the
    /// `dispatch` merge; this pins the wiring the merge relies on.
    #[test]
    fn new_title_input_group_is_required_and_exclusive() {
        use clap::CommandFactory;
        let cmd = Cli::command();
        let new_sub = cmd
            .get_subcommands()
            .find(|s| s.get_name() == "create")
            .expect("`create` subcommand present");
        // `is_multiple` is a `&mut self` builder getter, so work on an
        // owned clone.
        let mut group = new_sub
            .get_groups()
            .find(|g| g.get_id() == "title_input")
            .expect("`title_input` group present")
            .clone();
        assert!(group.is_required_set(), "title_input must be required");
        assert!(
            !group.is_multiple(),
            "title_input must be mutually exclusive (multiple=false)"
        );
        let members: Vec<_> = group.get_args().map(|id| id.as_str()).collect();
        assert!(
            members.contains(&"title_pos") && members.contains(&"title_flag"),
            "title_input must contain both title_pos and title_flag; got {members:?}"
        );
    }

    #[test]
    fn truncate_handles_non_ascii_at_boundary() {
        // Regression: byte-index slicing panicked at non-char boundary
        // for Finnish titles like "Käyttäjän kirjautuminen rikki".
        let title = "Käyttäjän kirjautuminen rikki sisäänkirjautumisessa";
        // Should not panic for any max_len <= char count.
        for n in 1..=title.chars().count() {
            let _ = truncate(title, n);
        }
        // Truncated output should be a valid string ending in ellipsis
        // when truncation actually happens.
        let out = truncate(title, 10);
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().count(), 10);
    }

    #[test]
    fn truncate_keeps_short_text_unchanged() {
        assert_eq!(truncate("ä", 5), "ä");
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_max_len_zero_returns_empty() {
        // Without the guard, `max_len = 0` would push `…` and return
        // a single-character string, violating the contract.
        assert_eq!(truncate("anything", 0), "");
        assert_eq!(truncate("", 0), "");
    }

    #[test]
    fn new_with_reserved_notes_section_warns_without_blocking() {
        let tmp = fresh_repo();
        let mut a = new_args("task", "Reserved");
        a.slug = Some("reserved-notes".into());
        a.description = Some("intro\n\n## Notes\n\nlegacy content".into());
        let n = do_new(tmp.path(), a).unwrap();
        // Write still happened.
        assert!(n.item_path.exists());
        // ...and a reserved-section warning was surfaced.
        assert_eq!(n.warnings.len(), 1, "warnings={:?}", n.warnings);
        assert!(n.warnings[0].contains("## Notes"));
        assert!(n.warnings[0].contains("## Comments"));
    }

    #[test]
    fn new_warns_when_reserved_section_follows_horizontal_rule() {
        // Regression: the raw body carries a Markdown horizontal rule
        // (`---` + blank line) *before* `## Notes`. An earlier version
        // stripped frontmatter by splitting `render` on `---\n\n`, which
        // this body would truncate — silently dropping the warning.
        let tmp = fresh_repo();
        let mut a = new_args("task", "Reserved");
        a.slug = Some("notes-after-rule".into());
        a.description = Some("intro\n\n---\n\n## Notes\n\nlegacy content".into());
        let n = do_new(tmp.path(), a).unwrap();
        assert_eq!(n.warnings.len(), 1, "warnings={:?}", n.warnings);
        assert!(n.warnings[0].contains("## Notes"));
    }

    #[test]
    fn new_with_clean_body_has_no_warnings() {
        let tmp = fresh_repo();
        let mut a = new_args("task", "Clean");
        a.slug = Some("clean-body".into());
        a.description = Some("intro\n\n## Comments\n\nfine".into());
        let n = do_new(tmp.path(), a).unwrap();
        assert!(n.warnings.is_empty(), "warnings={:?}", n.warnings);
    }

    #[test]
    fn body_set_with_reserved_notes_section_warns_without_blocking() {
        let tmp = fresh_repo();
        let mut a = new_args("task", "Body target");
        a.slug = Some("body-target".into());
        let n = do_new(tmp.path(), a).unwrap();
        let outcome = mutate::update_body(
            tmp.path(),
            &n.slug,
            None,
            "fresh body\n\n## Notes\n\nlegacy".into(),
            false,
        )
        .unwrap();
        assert_eq!(outcome.warnings.len(), 1, "warnings={:?}", outcome.warnings);
        assert!(outcome.warnings[0].contains("## Notes"));
        // The body was written despite the warning.
        assert!(read(&n.item_path).contains("## Notes"));
    }

    #[test]
    fn body_set_with_clean_body_has_no_warnings() {
        let tmp = fresh_repo();
        let mut a = new_args("task", "Body clean");
        a.slug = Some("body-clean".into());
        let n = do_new(tmp.path(), a).unwrap();
        let outcome = mutate::update_body(
            tmp.path(),
            &n.slug,
            None,
            "fresh body\n\n## Comments\n\nfine".into(),
            false,
        )
        .unwrap();
        assert!(
            outcome.warnings.is_empty(),
            "warnings={:?}",
            outcome.warnings
        );
    }

    #[test]
    fn update_sets_status_and_bumps_updated() {
        let tmp = fresh_repo();
        let mut a = new_args("bug", "Bug");
        a.slug = Some("my-test-slug".into());
        a.reporter = Some("rep".into());
        a.assignee = Some("ass".into());
        let n = do_new(tmp.path(), a).unwrap();
        do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                status: Some("in-progress".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(content.contains("status: in-progress"));
    }

    #[test]
    fn update_with_closing_status_does_not_move_directory() {
        let tmp = fresh_repo();
        let mut a = new_args("bug", "Bug");
        a.slug = Some("close-me".into());
        a.reporter = Some("r".into());
        a.assignee = Some("a".into());
        let n = do_new(tmp.path(), a).unwrap();
        let outcome = do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                status: Some("fixed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(outcome.moved_to_closed);
        // Flat layout: the directory does not change on close.
        assert_eq!(outcome.final_dir, n.item_path.parent().unwrap());
        let content = read(&n.item_path);
        assert!(content.contains("status: fixed"));
        assert!(content.contains("closed:"));
    }

    #[test]
    fn update_set_epic_replaces_value() {
        let tmp = fresh_repo();
        let mut a = new_args("task", "T");
        a.slug = Some("task-x".into());
        a.epic = Some("api-v2".into());
        let n = do_new(tmp.path(), a).unwrap();
        do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                epic: Some("api-v3".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(content.contains("epic: api-v3"));
    }

    #[test]
    fn update_lane_seq_sets_and_clears() {
        // End-to-end mutate/CLI path: `--lane-seq` writes an *unquoted*
        // YAML integer (so the parser lifts it back into the typed slot),
        // and `--no-lane-seq` removes it.
        let tmp = fresh_repo();
        let mut a = new_args("task", "T");
        a.slug = Some("seq-x".into());
        let n = do_new(tmp.path(), a).unwrap();

        do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                lane_seq: Some(20),
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(
            content.contains("lane_seq: 20"),
            "lane_seq must be an unquoted integer; got: {content}"
        );
        let parsed = issuectl_core::parser::parse_item_md_text_with_warnings(
            &content,
            &n.slug,
            "open",
            Path::new("x"),
        );
        assert_eq!(
            parsed.issue.lane_seq,
            Some(20),
            "written lane_seq must lift back into the typed field"
        );

        do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                no_lane_seq: true,
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(
            !content.contains("lane_seq"),
            "--no-lane-seq must remove the key; got: {content}"
        );
    }

    #[test]
    fn update_no_epic_clears_field() {
        let tmp = fresh_repo();
        let mut a = new_args("task", "T");
        a.slug = Some("task-y".into());
        a.epic = Some("api-v2".into());
        let n = do_new(tmp.path(), a).unwrap();
        do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                no_epic: true,
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        assert!(!content.contains("epic:"));
    }

    #[test]
    fn update_add_blocked_by_writes_and_normalizes_at_sigil() {
        // `update --add-blocked-by @foo bar` (mixed sigil) round-trips
        // through the same flock/schema write path as `--add-related`
        // and lands the raw `extra.blocked_by` list. Normalization
        // rewrites every ref to the canonical `@slug` form on write (the
        // same value that folds into `canonical_hash`), the same as a
        // `depend add` or `--add-related`.
        let tmp = fresh_repo();
        let mut a = new_args("task", "Subject");
        a.slug = Some("blk-subject".into());
        let n = do_new(tmp.path(), a).unwrap();
        for dep in ["blk-one", "blk-two"] {
            let mut da = new_args("task", "Dep");
            da.slug = Some(dep.into());
            do_new(tmp.path(), da).unwrap();
        }
        do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                add_blocked_by: vec!["@blk-one".into(), "blk-two".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        let issue = issuectl_core::parser::parse_item_md_text_with_warnings(
            &content,
            &n.slug,
            "open",
            &n.item_path,
        )
        .issue;
        assert_eq!(
            issue.blocked_by(),
            vec!["blk-one".to_string(), "blk-two".to_string()],
            "both blockers must round-trip (bare, sorted): {content}"
        );
        // Assert the RAW stored value, not just `blocked_by()` (which
        // re-normalizes on read): the mixed-sigil input must be rewritten
        // to the canonical `@slug` form on disk (insertion order), so the
        // value folded into `canonical_hash` is the canonical one.
        assert_eq!(
            issue.extra.get("blocked_by"),
            Some(&serde_json::json!(["@blk-one", "@blk-two"])),
            "raw extra.blocked_by must be canonical @-form: {content}"
        );
    }

    #[test]
    fn update_blocked_by_add_remove_round_trip() {
        // Add two blockers, then remove one: the surviving edge stays and
        // the dropped one is gone. Mirrors `--add-related`/`--remove-related`.
        let tmp = fresh_repo();
        let mut a = new_args("task", "Subject");
        a.slug = Some("rt-subject".into());
        let n = do_new(tmp.path(), a).unwrap();
        for dep in ["rt-one", "rt-two"] {
            let mut da = new_args("task", "Dep");
            da.slug = Some(dep.into());
            do_new(tmp.path(), da).unwrap();
        }
        do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                add_blocked_by: vec!["rt-one".into(), "rt-two".into()],
                ..Default::default()
            },
        )
        .unwrap();
        do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                remove_blocked_by: vec!["@rt-one".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        let issue = issuectl_core::parser::parse_item_md_text_with_warnings(
            &content,
            &n.slug,
            "open",
            &n.item_path,
        )
        .issue;
        assert_eq!(
            issue.blocked_by(),
            vec!["rt-two".to_string()],
            "removed edge must be gone, survivor kept: {content}"
        );
    }

    #[test]
    fn update_blocked_by_json_projection_matches_show() {
        // The `--json` projection (`project_blocked_by`) must surface a
        // sorted/deduped/`@`-prefixed top-level `blocked_by` array and
        // strip the raw `extra.blocked_by` copy — the same wire shape
        // `show`/`ls`/`search --json` emit. Feed unsorted + duplicate
        // adds and confirm the projection canonicalizes them.
        let tmp = fresh_repo();
        let mut a = new_args("task", "Subject");
        a.slug = Some("proj-subject".into());
        let n = do_new(tmp.path(), a).unwrap();
        for dep in ["proj-zeta", "proj-alpha"] {
            let mut da = new_args("task", "Dep");
            da.slug = Some(dep.into());
            do_new(tmp.path(), da).unwrap();
        }
        do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                // Unsorted, with a duplicate (mixed sigil) to force dedup.
                add_blocked_by: vec!["@proj-zeta".into(), "proj-alpha".into(), "proj-zeta".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let content = read(&n.item_path);
        let issue = issuectl_core::parser::parse_item_md_text_with_warnings(
            &content,
            &n.slug,
            "open",
            &n.item_path,
        )
        .issue;
        let mut v = serde_json::to_value(&issue).expect("Issue serializes");
        let m = v.as_object_mut().unwrap();
        project_blocked_by(m, &issue);
        assert_eq!(
            m.get("blocked_by").unwrap(),
            &serde_json::json!(["@proj-alpha", "@proj-zeta"]),
            "top-level projection must be sorted/deduped/@-prefixed"
        );
        // The raw `extra.blocked_by` copy is stripped so there is exactly
        // one wire representation.
        let extra_blocked = m
            .get("extra")
            .and_then(|e| e.as_object())
            .and_then(|e| e.get("blocked_by"));
        assert!(
            extra_blocked.is_none(),
            "raw extra.blocked_by must be stripped: {m:?}"
        );
        // The duplicate `proj-zeta` add must not survive in RAW storage
        // either — `write::add_to_string_list` dedups on insert — so the
        // canonical hash never folds in a phantom duplicate that the
        // read-time projection would otherwise paper over. Inspect the raw
        // `extra.blocked_by` value directly (insertion order, canonical
        // `@`-form, deduped) rather than trusting the deduping reader.
        assert_eq!(
            issue.extra.get("blocked_by"),
            Some(&serde_json::json!(["@proj-zeta", "@proj-alpha"])),
            "raw blocked_by must be deduped in storage, not just in projection: {content}"
        );
    }

    #[test]
    fn update_add_blocked_by_self_is_rejected() {
        // The CLI surface must surface the core self-block guard as an
        // error (→ `--json` error envelope + non-zero exit in `main`).
        let tmp = fresh_repo();
        let mut a = new_args("task", "Subject");
        a.slug = Some("self-blk".into());
        let n = do_new(tmp.path(), a).unwrap();
        let err = do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                add_blocked_by: vec!["@self-blk".into()],
                ..Default::default()
            },
        )
        .err()
        .expect("self-block must be rejected");
        assert!(
            err.to_string().contains("cannot block itself"),
            "self-block must be rejected: {err}"
        );
    }

    #[test]
    fn update_blocked_by_add_and_remove_same_slug_is_rejected() {
        // `--add-blocked-by @x --remove-blocked-by x` is conflicting
        // intent (caught after normalization, so the mixed sigil still
        // collides) — must error, not silently pick one.
        let tmp = fresh_repo();
        let mut a = new_args("task", "Subject");
        a.slug = Some("conflict-subj".into());
        let n = do_new(tmp.path(), a).unwrap();
        let err = do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                add_blocked_by: vec!["@dep-x".into()],
                remove_blocked_by: vec!["dep-x".into()],
                ..Default::default()
            },
        )
        .err()
        .expect("add+remove overlap must be rejected");
        assert!(
            err.to_string()
                .contains("add_blocked_by and remove_blocked_by"),
            "add+remove overlap must be rejected: {err}"
        );
    }

    #[test]
    fn update_add_blocked_by_malformed_ref_is_rejected() {
        // A malformed ref must fail validation (→ non-zero exit +
        // `--json` error envelope), per the issue's contract.
        let tmp = fresh_repo();
        let mut a = new_args("task", "Subject");
        a.slug = Some("malformed-subj".into());
        let n = do_new(tmp.path(), a).unwrap();
        let err = do_update(
            tmp.path(),
            UpdateArgs {
                slug: n.slug.clone(),
                add_blocked_by: vec!["not a slug!".into()],
                ..Default::default()
            },
        )
        .err()
        .expect("malformed ref must be rejected");
        assert!(
            err.to_string()
                .contains("must be @slug or a kebab-case slug"),
            "malformed ref must be rejected: {err}"
        );
    }

    #[test]
    fn close_defaults_to_fixed_for_bug() {
        let tmp = fresh_repo();
        let mut a = new_args("bug", "Bug");
        a.slug = Some("bug-slug".into());
        a.reporter = Some("r".into());
        a.assignee = Some("a".into());
        let n = do_new(tmp.path(), a).unwrap();
        let outcome = do_close(tmp.path(), &n.slug, None, None, None, vec![], None).unwrap();
        assert!(outcome.moved_to_closed);
        let content = read(&outcome.final_dir.join("item.md"));
        assert!(content.contains("status: fixed"));
    }

    #[test]
    fn close_defaults_to_done_for_task() {
        let tmp = fresh_repo();
        let mut a = new_args("task", "Task");
        a.slug = Some("task-slug".into());
        a.reporter = Some("r".into());
        a.assignee = Some("a".into());
        let n = do_new(tmp.path(), a).unwrap();
        let outcome = do_close(tmp.path(), &n.slug, None, None, None, vec![], None).unwrap();
        let content = read(&outcome.final_dir.join("item.md"));
        assert!(content.contains("status: done"));
    }

    #[test]
    fn close_rejects_already_closed() {
        let tmp = fresh_repo();
        let mut a = new_args("bug", "Bug");
        a.slug = Some("once-only".into());
        a.reporter = Some("r".into());
        a.assignee = Some("a".into());
        let n = do_new(tmp.path(), a).unwrap();
        do_close(tmp.path(), &n.slug, None, None, None, vec![], None).unwrap();
        assert!(do_close(tmp.path(), &n.slug, None, None, None, vec![], None).is_err());
    }

    #[test]
    fn locate_issue_finds_flat() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/foo-bar")).unwrap();
        fs::write(
            tmp.path().join("issues/foo-bar/item.md"),
            "---\nstatus: open\n---\n",
        )
        .unwrap();
        let (folder, _) = locate_issue(tmp.path(), "foo-bar").unwrap();
        assert_eq!(folder, "open");
        assert!(locate_issue(tmp.path(), "missing").is_err());
    }

    #[test]
    fn locate_issue_finds_legacy_path() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/closed/old-fox-here")).unwrap();
        fs::write(
            tmp.path().join("issues/closed/old-fox-here/item.md"),
            "---\nstatus: fixed\n---\n",
        )
        .unwrap();
        let (folder, item) = locate_issue(tmp.path(), "old-fox-here").unwrap();
        assert_eq!(folder, "closed");
        assert!(item
            .to_string_lossy()
            .contains("issues/closed/old-fox-here"));
    }

    #[test]
    fn parse_days_accepts_bare_and_suffix() {
        assert_eq!(parse_days("90").unwrap(), 90);
        assert_eq!(parse_days("90d").unwrap(), 90);
        assert_eq!(parse_days("0").unwrap(), 0);
        assert!(parse_days("-5").is_err());
        assert!(parse_days("7days").is_err());
        assert!(parse_days("d").is_err());
    }

    #[test]
    fn parse_commit_spec_basic() {
        assert_eq!(
            parse_commit_spec("abc123:fix login").unwrap(),
            ("abc123".to_string(), "fix login".to_string())
        );
    }

    #[test]
    fn parse_commit_spec_rejects_no_colon() {
        assert!(parse_commit_spec("abc123 fix").is_err());
    }
    #[test]
    fn parse_non_empty_rejects_empty_and_padded() {
        assert!(parse_non_empty("").is_err());
        assert!(parse_non_empty("  ").is_err());
        assert!(parse_non_empty(" a").is_err());
    }

    #[test]
    fn parse_custom_field_rejects_built_in_keys() {
        // Built-in keys must use their dedicated flags so we don't
        // shadow validation done by clap (e.g. `--field type=garbage`).
        for k in ["type", "title", "slug", "status", "priority"] {
            let s = format!("{k}=foo");
            assert!(parse_custom_field(&s).is_err(), "{k} must be rejected");
        }
    }
    #[test]
    fn parse_custom_field_message_points_at_real_flag() {
        // Round-2 review: previous message hardcoded `--<key>` for keys
        // (`commits`, `closed`) that have no matching flag. The hint
        // table now points at the real flag or behavior.
        let err = parse_custom_field("commits=foo").unwrap_err();
        assert!(
            err.contains("--add-commit"),
            "expected --add-commit hint, got {err:?}"
        );
        let err = parse_custom_field("closed=foo").unwrap_err();
        assert!(
            err.contains("status") || err.contains("closing"),
            "expected status/closing hint, got {err:?}"
        );
    }

    #[test]
    fn parse_custom_field_accepts_kebab_and_underscore() {
        assert!(parse_custom_field("team=payments").is_ok());
        assert!(parse_custom_field("team-name=payments").is_ok());
        assert!(parse_custom_field("severity_level=p1").is_ok());
        assert!(parse_custom_field("=payments").is_err());
        assert!(parse_custom_field("team=").is_err());
        assert!(parse_custom_field("team:payments").is_err());
    }

    #[test]
    fn parse_custom_field_rejects_padded_input() {
        // Aligns with `parse_non_empty`'s reject-padding policy so
        // `--field` and `--clear-field` don't silently strip whitespace
        // the user did not intend.
        assert!(parse_custom_field(" team=payments").is_err());
        assert!(parse_custom_field("team =payments").is_err());
        assert!(parse_custom_field("team= payments").is_err());
        assert!(parse_custom_field("team=payments ").is_err());
    }

    #[test]
    fn parse_custom_field_key_accepts_valid_keys_and_rejects_built_ins() {
        assert!(parse_custom_field_key("team").is_ok());
        assert!(parse_custom_field_key("team-name").is_ok());
        assert!(parse_custom_field_key("severity_level").is_ok());

        for (k, _) in mutate::RESERVED_CUSTOM_FIELD_KEYS {
            assert!(
                parse_custom_field_key(k).is_err(),
                "{k} must be rejected as built-in"
            );
        }

        assert!(parse_custom_field_key("").is_err());
        assert!(parse_custom_field_key(" team").is_err(), "padded key");
        assert!(parse_custom_field_key("team ").is_err(), "padded key");
        assert!(parse_custom_field_key("bad key").is_err());
        assert!(parse_custom_field_key("team:name").is_err());
    }

    fn write_raw_issue(root: &Path, slug: &str, fm: &str, body: &str) {
        let dir = root.join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), format!("---\n{fm}---\n{body}")).unwrap();
    }

    #[test]
    fn ls_query_filters_by_status_and_label() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: in-progress\npriority: high\nassignee: alice\nlabels: [frontend]\n",
            "# Login is broken\n",
        );
        write_raw_issue(
            tmp.path(),
            "calm-bright-newt",
            "type: feature\nstatus: open\npriority: normal\nassignee: bob\nlabels: [wontfix]\n",
            "# Add export\n",
        );

        let issues = repo::load_issues(tmp.path());
        let q = query::parse("status:in-progress assignee:alice").unwrap();
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| query::matches(&q, i))
            .map(|i| i.slug.clone())
            .collect();
        assert_eq!(hits, vec!["amber-loud-fox".to_string()]);

        let q = query::parse("-label:wontfix").unwrap();
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| query::matches(&q, i))
            .map(|i| i.slug.clone())
            .collect();
        assert_eq!(hits, vec!["amber-loud-fox".to_string()]);
    }

    /// Folder-scope decision for `list` (`list_folder_filter`). Locks
    /// the `list-status-done` contract: a positively-pinned status
    /// disables the implicit open-only default so `ls -s done`/`-s fixed`
    /// reach closed/archived issues, while bare `list` stays open-only
    /// and explicit `--all`/`--closed` remain authoritative (a pinned
    /// status must NOT silently drop `--closed`).
    #[test]
    fn list_folder_filter_scope_rules() {
        let status_q = |v: &str| {
            let mut q = query::Query::default();
            q.push(query::Term::Field {
                field: query::FieldName::Status,
                m: query::FieldMatch::Equals(v.to_string()),
                negated: false,
            });
            q
        };
        let bare = query::Query::default();

        // Bare `list`: open-only default preserved.
        assert_eq!(list_folder_filter(&bare, false, false, false), Some("open"));
        // `-s fixed` (no --all/--closed): default steps aside → all folders.
        assert_eq!(
            list_folder_filter(&status_q("fixed"), false, false, false),
            None,
            "pinned status must lift the open-only default"
        );
        // `--closed -s done`: --closed stays authoritative, not dropped.
        assert_eq!(
            list_folder_filter(&status_q("done"), false, true, false),
            Some("closed"),
            "--closed must survive a pinned status"
        );
        // `--all -s done`: no folder restriction.
        assert_eq!(
            list_folder_filter(&status_q("done"), true, false, false),
            None
        );
        // Negated status alone does NOT scope-expand (still open-only)…
        let neg = query::parse("-status:wontfix").unwrap();
        assert_eq!(
            list_folder_filter(&neg, false, false, false),
            Some("open"),
            "a lone negated status is exclusion, not scope"
        );
        // …but supplying it positionally (has_query) does, like any
        // positional query.
        assert_eq!(list_folder_filter(&neg, false, false, true), None);
    }

    /// `search -status:wontfix` should remain scoped to open. A
    /// negated status term is exclusion, not scope expansion.
    #[test]
    fn search_negated_status_does_not_expand_scope() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\n",
            "# Open\n",
        );
        write_raw_issue(
            tmp.path(),
            "calm-bright-newt",
            "type: bug\nstatus: done\nclosed: 2026-05-01\n",
            "# Done\n",
        );

        let issues = repo::load_issues(tmp.path());
        let q = query::parse("-status:wontfix").unwrap();

        let scope_expanded = q.has_positive_field(query::FieldName::Folder)
            || q.has_positive_field(query::FieldName::Status);
        assert!(!scope_expanded, "negation must not expand scope");

        let hits: Vec<_> = issues
            .iter()
            .filter(|i| {
                if !scope_expanded && i.folder != "open" {
                    return false;
                }
                query::matches(&q, i)
            })
            .map(|i| i.slug.clone())
            .collect();
        assert_eq!(hits, vec!["amber-loud-fox".to_string()]);
    }

    #[test]
    fn search_query_combines_text_and_field() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\n",
            "# Login deadlock\n\nUser hits flock contention.",
        );
        write_raw_issue(
            tmp.path(),
            "calm-bright-newt",
            "type: feature\nstatus: open\n",
            "# Just a deadlock-themed feature note.",
        );

        let issues = repo::load_issues(tmp.path());
        let q = query::parse("deadlock text:flock").unwrap();
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| query::matches(&q, i))
            .map(|i| i.slug.clone())
            .collect();
        assert_eq!(hits, vec!["amber-loud-fox".to_string()]);
    }

    // ── parse_apply_patch (round-2 #4) ────────────────────────────

    #[test]
    fn parse_apply_patch_rejects_null_expected_version_under_json() {
        let yaml = "slug: some-issue\nexpected_version: null\npriority: high\n";
        let err = parse_apply_patch(yaml, true).unwrap_err();
        assert!(
            err.to_string().contains("expected_version"),
            "expected expected_version error, got {err}"
        );
    }

    #[test]
    fn parse_apply_patch_rejects_missing_expected_version_under_json() {
        let yaml = "slug: some-issue\npriority: high\n";
        let err = parse_apply_patch(yaml, true).unwrap_err();
        assert!(err.to_string().contains("expected_version"));
    }

    #[test]
    fn parse_apply_patch_rejects_empty_and_padded_expected_version_under_json() {
        for v in [
            "expected_version: \"\"",
            "expected_version: \"   \"",
            "expected_version: \" sha256:abc \"",
        ] {
            let yaml = format!("slug: some-issue\n{v}\npriority: high\n");
            let err = parse_apply_patch(&yaml, true).unwrap_err();
            assert!(
                err.to_string().contains("expected_version"),
                "expected expected_version error for {v:?}, got {err}"
            );
        }
    }

    #[test]
    fn parse_apply_patch_rejects_user_supplied_dry_run_field() {
        let yaml = "slug: some-issue\ndry_run: true\npriority: high\n";
        let err = parse_apply_patch(yaml, false).unwrap_err();
        assert!(
            err.to_string().contains("dry_run") && err.to_string().contains("CLI flag"),
            "expected dry_run CLI-flag error, got {err}"
        );
    }

    #[test]
    fn parse_apply_patch_accepts_well_formed_json_patch() {
        let yaml = "slug: well-formed-issue\nexpected_version: sha256:abc123\npriority: high\n";
        let (slug, req) = parse_apply_patch(yaml, true).unwrap();
        assert_eq!(slug, "well-formed-issue");
        assert_eq!(req.expected_version.as_deref(), Some("sha256:abc123"));
    }

    #[test]
    fn parse_apply_patch_allows_missing_expected_version_when_not_json() {
        // Non-JSON callers may opt into blind clobber: `flock` still
        // serializes writes, but no version check is required.
        let yaml = "slug: some-issue\npriority: high\n";
        let (slug, req) = parse_apply_patch(yaml, false).unwrap();
        assert_eq!(slug, "some-issue");
        assert!(req.expected_version.is_none());
    }

    // ── bulk ──────────────────────────────────────────────────────

    fn bulk_spec(set: &[(&str, &str)], add_labels: &[&str]) -> BulkSpec {
        BulkSpec {
            set: set
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            add_labels: add_labels.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn status_of(root: &Path, slug: &str) -> String {
        repo::load_issues(root)
            .into_iter()
            .find(|i| i.slug == slug)
            .unwrap_or_else(|| panic!("issue {slug} not found"))
            .status
    }

    #[test]
    fn parse_bulk_set_accepts_built_ins_and_custom() {
        assert_eq!(
            parse_bulk_set("status=done").unwrap(),
            ("status".to_string(), "done".to_string())
        );
        assert_eq!(
            parse_bulk_set("team=payments").unwrap(),
            ("team".to_string(), "payments".to_string())
        );
        assert!(parse_bulk_set("status").is_err());
        assert!(parse_bulk_set("status=").is_err());
        assert!(parse_bulk_set("=done").is_err());
        assert!(parse_bulk_set(" status=done").is_err());
        assert!(parse_bulk_set("status =done").is_err());
        assert!(parse_bulk_set("bad key=done").is_err());
    }

    #[test]
    fn parse_bulk_set_rejects_unroutable_built_ins_with_hint() {
        // List-shaped and auto-managed built-ins can't go through --set;
        // the error points at the right flag instead of landing in the
        // custom-field slot and erroring late.
        let err = parse_bulk_set("labels=foo").unwrap_err();
        assert!(err.contains("--add-label"), "got {err:?}");
        let err = parse_bulk_set("related=foo").unwrap_err();
        assert!(err.contains("--add-related"), "got {err:?}");
        for k in ["title", "slug", "commits", "closed", "created"] {
            assert!(
                parse_bulk_set(&format!("{k}=foo")).is_err(),
                "{k} must be rejected"
            );
        }
        // Routable built-ins and genuine custom fields still pass.
        assert!(parse_bulk_set("priority=high").is_ok());
        assert!(parse_bulk_set("team=payments").is_ok());
    }

    #[test]
    fn parse_bulk_clear_rejects_unroutable_built_ins() {
        assert!(parse_bulk_clear_key("labels").is_err());
        assert!(parse_bulk_clear_key("title").is_err());
        assert!(parse_bulk_clear_key("epic").is_ok());
        assert!(parse_bulk_clear_key("team").is_ok());
    }

    #[test]
    fn validate_bulk_spec_rejects_empty_and_dups() {
        assert!(validate_bulk_spec(&BulkSpec::default()).is_err());
        let dup_set = BulkSpec {
            set: vec![
                ("priority".into(), "high".into()),
                ("priority".into(), "low".into()),
            ],
            ..Default::default()
        };
        assert!(validate_bulk_spec(&dup_set).is_err());
        let overlap = BulkSpec {
            set: vec![("epic".into(), "some-epic".into())],
            clear: vec!["epic".into()],
            ..Default::default()
        };
        assert!(validate_bulk_spec(&overlap).is_err());
        assert!(validate_bulk_spec(&bulk_spec(&[("priority", "high")], &[])).is_ok());
    }

    #[test]
    fn bulk_applies_set_to_every_match() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: normal\nassignee: alice\n",
            "# One\n",
        );
        write_raw_issue(
            tmp.path(),
            "calm-bright-newt",
            "type: bug\nstatus: open\npriority: normal\nassignee: alice\n",
            "# Two\n",
        );
        write_raw_issue(
            tmp.path(),
            "eager-silent-mole",
            "type: feature\nstatus: open\npriority: normal\nassignee: bob\n",
            "# Three\n",
        );

        let q = query::parse("assignee:alice").unwrap();
        let spec = bulk_spec(&[("priority", "high")], &[]);
        let results = bulk_apply(tmp.path(), &q, &spec, false).unwrap();

        let mut slugs: Vec<_> = results.iter().map(|r| r.slug.clone()).collect();
        slugs.sort();
        assert_eq!(slugs, vec!["amber-loud-fox", "calm-bright-newt"]);

        let issues = repo::load_issues(tmp.path());
        let by = |s: &str| {
            issues
                .iter()
                .find(|i| i.slug == s)
                .unwrap()
                .priority
                .clone()
        };
        assert_eq!(by("amber-loud-fox"), "high");
        assert_eq!(by("calm-bright-newt"), "high");
        // The non-matching issue is untouched.
        assert_eq!(by("eager-silent-mole"), "normal");
    }

    #[test]
    fn bulk_set_status_routes_through_typed_slot() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            // `feature`, so `done` is the type-compatible completion (the
            // code-level type × status invariant reserves `done` for
            // non-bug work — a bug completes as `fixed`).
            "type: feature\nstatus: open\npriority: normal\nassignee: alice\n",
            "# One\n",
        );
        let q = query::parse("status:open").unwrap();
        let spec = bulk_spec(&[("status", "done")], &[]);
        bulk_apply(tmp.path(), &q, &spec, false).unwrap();
        assert_eq!(status_of(tmp.path(), "amber-loud-fox"), "done");
        // A closing status routes through the typed slot, so `closed:`
        // is stamped (and the issue lands in the closed folder).
        let issue = repo::load_issues(tmp.path())
            .into_iter()
            .find(|i| i.slug == "amber-loud-fox")
            .unwrap();
        assert_eq!(issue.folder, "closed");
        assert!(issue.closed.is_some());
    }

    #[test]
    fn bulk_dry_run_writes_nothing_and_returns_diffs() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: normal\n",
            "# One\n",
        );
        let q = query::parse("status:open").unwrap();
        let spec = bulk_spec(&[("priority", "high")], &[]);
        let results = bulk_apply(tmp.path(), &q, &spec, true).unwrap();
        assert_eq!(results.len(), 1);
        let diff = results[0].diff.as_deref().unwrap();
        assert!(diff.contains("priority"), "diff should mention the change");
        // Nothing written: on-disk priority is unchanged.
        let issues = repo::load_issues(tmp.path());
        assert_eq!(issues[0].priority, "normal");
    }

    #[test]
    fn bulk_dry_run_status_change_writes_nothing_but_shows_diff() {
        // Flat layout: the directory is `issues/<slug>/` regardless of
        // status, so a status change shows up in the diff (frontmatter +
        // a stamped `closed:`), not as a directory move. Dry-run must
        // write nothing.
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            // `feature`, so `done` is the type-compatible completion.
            "type: feature\nstatus: open\npriority: normal\nassignee: alice\n",
            "# One\n",
        );
        let q = query::parse("status:open").unwrap();
        let spec = bulk_spec(&[("status", "done")], &[]);
        let results = bulk_apply(tmp.path(), &q, &spec, true).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0]
            .final_dir
            .to_string_lossy()
            .ends_with("issues/amber-loud-fox"));
        let diff = results[0].diff.as_deref().unwrap();
        assert!(diff.contains("status: done"), "diff: {diff}");
        assert!(diff.contains("closed:"), "diff should stamp closed: {diff}");
        // Still a dry run: on-disk status is unchanged.
        assert_eq!(status_of(tmp.path(), "amber-loud-fox"), "open");
    }

    #[test]
    fn bulk_no_match_is_empty_not_error() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\n",
            "# One\n",
        );
        let q = query::parse("assignee:nobody").unwrap();
        let spec = bulk_spec(&[("priority", "high")], &[]);
        let results = bulk_apply(tmp.path(), &q, &spec, false).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn bulk_preflight_aborts_all_on_one_invalid_target() {
        // Two issues match; the priority value is invalid, so the
        // dry-run pre-flight must reject before any write lands.
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: normal\n",
            "# One\n",
        );
        write_raw_issue(
            tmp.path(),
            "calm-bright-newt",
            "type: bug\nstatus: open\npriority: normal\n",
            "# Two\n",
        );
        let q = query::parse("status:open").unwrap();
        let spec = bulk_spec(&[("priority", "bogus")], &[]);
        let err = bulk_apply(tmp.path(), &q, &spec, false).unwrap_err();
        assert!(
            err.to_string().contains("priority"),
            "expected a priority validation error, got {err}"
        );
        // No file was rewritten — both keep their original priority.
        let issues = repo::load_issues(tmp.path());
        for i in &issues {
            assert_eq!(i.priority, "normal", "{} must be untouched", i.slug);
        }
    }

    #[test]
    fn bulk_adds_label_to_matches() {
        let tmp = fresh_repo();
        write_raw_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: normal\nlabels: [frontend]\n",
            "# One\n",
        );
        let q = query::parse("status:open").unwrap();
        let spec = bulk_spec(&[], &["triaged"]);
        bulk_apply(tmp.path(), &q, &spec, false).unwrap();
        let issue = repo::load_issues(tmp.path())
            .into_iter()
            .find(|i| i.slug == "amber-loud-fox")
            .unwrap();
        let labels = issue.labels.unwrap_or_default();
        assert!(labels.contains(&"triaged".to_string()));
        assert!(labels.contains(&"frontend".to_string()));
    }

    #[test]
    fn read_message_arg_prefers_positional() {
        let got = read_message_arg(Some("hello".into()), None, false, None).unwrap();
        assert_eq!(got, "hello");
    }

    #[test]
    fn read_message_arg_rejects_two_inline_sources() {
        let err = read_message_arg(Some("positional".into()), Some("flag".into()), false, None)
            .unwrap_err();
        assert!(err.to_string().contains("note_body"), "got: {err}");
    }

    #[test]
    fn read_message_arg_reads_message_flag() {
        // The `--message`/`--body` flag is a first-class body source
        // alongside the positional.
        let got = read_message_arg(None, Some("via flag".into()), false, None).unwrap();
        assert_eq!(got, "via flag");
    }

    #[test]
    fn read_message_arg_reads_from_file() {
        // `--from-file` and its `--body-file` visible alias share this one
        // `from_file` source.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("note.md");
        fs::write(&path, "from a file\n").unwrap();
        let got = read_message_arg(None, None, false, Some(path)).unwrap();
        assert_eq!(got, "from a file");
    }

    #[test]
    fn read_message_arg_requires_a_source() {
        assert!(read_message_arg(None, None, false, None).is_err());
    }

    #[test]
    fn read_message_arg_rejects_blank_text() {
        assert!(read_message_arg(Some("   \n".into()), None, false, None).is_err());
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("blank.md");
        fs::write(&path, "\n\n").unwrap();
        assert!(read_message_arg(None, None, false, Some(path)).is_err());
    }

    // Small limit so cap tests stay sub-millisecond instead of allocating
    // the real 10 MiB bound.
    const TEST_LIMIT: u64 = 16;

    #[test]
    fn read_capped_accepts_input_at_the_limit() {
        let data = vec![b'a'; TEST_LIMIT as usize];
        let got = read_capped(data.as_slice(), TEST_LIMIT, "note", "test").unwrap();
        assert_eq!(got.len(), TEST_LIMIT as usize);
    }

    #[test]
    fn read_capped_rejects_input_over_the_limit() {
        let data = vec![b'a'; TEST_LIMIT as usize + 1];
        let err = read_capped(data.as_slice(), TEST_LIMIT, "note", "test").unwrap_err();
        assert!(err.to_string().contains("exceeds"), "got: {err}");
    }

    #[test]
    fn read_capped_short_circuits_an_unbounded_source() {
        // `io::repeat` is an infinite reader (a `/dev/zero` stand-in). If the
        // `take(limit + 1)` guard regressed to reading everything, this test
        // would hang / OOM instead of returning a prompt "exceeds" error.
        let err = read_capped(std::io::repeat(b'a'), TEST_LIMIT, "body", "test").unwrap_err();
        assert!(err.to_string().contains("exceeds"), "got: {err}");
    }

    #[test]
    fn read_capped_rejects_invalid_utf8_with_offset() {
        let data: &[u8] = &[b'o', b'k', 0xff, 0xfe];
        let err = read_capped(data, TEST_LIMIT, "body", "test").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("UTF-8"), "got: {msg}");
        assert!(msg.contains("offset 2"), "got: {msg}");
    }

    #[test]
    fn read_capped_file_reads_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("body.md");
        fs::write(&path, "hello body\n").unwrap();
        let got = read_capped_file(&path, "body").unwrap();
        assert_eq!(got, "hello body\n");
    }

    #[test]
    fn read_capped_file_missing_path_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist.md");
        assert!(read_capped_file(&missing, "body").is_err());
    }
}
