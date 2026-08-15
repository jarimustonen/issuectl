//! Regression tests for the CLI papercuts closed under
//! `@cli-ux-subcommand-friction` (#2/#3/#5): a positional title on
//! `new`, an actionable built-in-list-field hint on `set`, and
//! order-insensitive `note` flags with a targeted missing-`--as` error.
//!
//! These lock the invocation surface (clap arg layout + hint wording)
//! that agents and humans reach for first, so a future refactor can't
//! silently reintroduce the friction. Convention for `tests/` vs inline
//! `#[cfg(test)]`: see `AGENTS.md` (`Tests`).

use std::process::{Command, Output};

use tempfile::TempDir;

/// Tempdir with an empty `issues/` dir; the first `new` bootstraps a
/// default `.schema.yaml`.
fn fresh_repo() -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("issues")).expect("mkdir issues");
    tmp
}

fn run(root: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        // Keep clap's stderr free of ANSI colour codes so the substring
        // assertions below match the plain flag/arg names.
        .env("NO_COLOR", "1")
        .current_dir(root)
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .expect("spawn issuectl")
}

/// `run` that asserts the command succeeded — used for test *setup* so a
/// broken seed fails loudly at its own call site instead of surfacing as
/// a confusing assertion failure later.
fn run_ok(root: &std::path::Path, args: &[&str]) -> Output {
    let out = run(root, args);
    assert_eq!(
        out.status.code(),
        Some(0),
        "setup command failed; {}",
        dump(&out)
    );
    out
}

fn dump(out: &Output) -> String {
    format!(
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    )
}

/// Reads a string `field` from a `--json show <slug>` payload.
fn show_field(root: &std::path::Path, slug: &str, field: &str) -> String {
    let show = run(root, &["--json", "show", slug]);
    assert_eq!(show.status.code(), Some(0), "{}", dump(&show));
    serde_json::from_slice::<serde_json::Value>(&show.stdout).expect("show stdout should be JSON")
        [field]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "show payload missing string field {field:?}; {}",
                dump(&show)
            )
        })
        .to_string()
}

// --- #2: positional title on `new` -------------------------------------

/// A positional title is accepted (matching how `note`/`search` take
/// positional text) and lands in the frontmatter `title`.
#[test]
fn new_accepts_positional_title() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "new",
            "Login loops",
            "--type",
            "bug",
            "--slug",
            "login-loops",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert_eq!(
        show_field(tmp.path(), "login-loops", "title"),
        "Login loops"
    );
}

/// The canonical `--title` flag still works — the positional is additive.
#[test]
fn new_title_flag_still_works() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "bug",
            "--title",
            "Flag title",
            "--slug",
            "flag-title",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert_eq!(show_field(tmp.path(), "flag-title", "title"), "Flag title");
}

/// Passing both the positional and `--title` is a clap usage error
/// (exit 2) that names the conflict.
#[test]
fn new_rejects_both_positional_and_flag_title() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &["new", "Positional", "--title", "Flag", "--type", "bug"],
    );
    // Exit code (clap usage error) is the behavioural contract; we also
    // check both title spellings appear, but deliberately do NOT assert
    // on clap's connecting grammar ("cannot be used with"), which is
    // rendering text that can change across clap versions.
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--title") && stderr.contains("TITLE"),
        "conflict error should name both title forms; got:\n{stderr}"
    );
}

/// Passing neither a positional title nor `--title` is a clap usage
/// error (exit 2) that flags the required title.
#[test]
fn new_rejects_missing_title() {
    let tmp = fresh_repo();
    let out = run(tmp.path(), &["new", "--type", "bug"]);
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("TITLE") && stderr.contains("--title"),
        "expected required-title error naming TITLE/--title; got:\n{stderr}"
    );
}

/// The positional title is order-insensitive relative to flags: giving
/// it *after* `--type`/`--slug` works just as well as before.
#[test]
fn new_positional_title_after_flags() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "bug",
            "--slug",
            "order-x",
            "Trailing title",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert_eq!(show_field(tmp.path(), "order-x", "title"), "Trailing title");
}

/// The `create` visible alias accepts the positional title too — it
/// resolves to the same `New` variant, so the ergonomic form must work
/// on both spellings of the verb.
#[test]
fn create_alias_accepts_positional_title() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "create",
            "Via create",
            "--type",
            "bug",
            "--slug",
            "via-create",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert_eq!(show_field(tmp.path(), "via-create", "title"), "Via create");
}

/// A bare positional title beginning with `-` is (intentionally) NOT
/// accepted: `allow_hyphen_values` is left off so a mistyped flag like
/// `new -p high` yields a clean "title required" error instead of being
/// swallowed as a title. The escape hatches for a genuinely
/// hyphen-leading title are `--title=<...>` and the `--` separator; both
/// are pinned here so a future `allow_hyphen_values` flip is a conscious
/// choice, not a silent regression.
#[test]
fn new_leading_hyphen_title_needs_an_escape() {
    let tmp = fresh_repo();

    // Bare positional starting with `-` is a clap usage error.
    let out = run(
        tmp.path(),
        &["new", "-Fix login", "--type", "bug", "--slug", "hy-bare"],
    );
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));

    // `--title=<value>` escapes it.
    let out = run(
        tmp.path(),
        &[
            "new",
            "--title=-Fix login",
            "--type",
            "bug",
            "--slug",
            "hy-eq",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert_eq!(show_field(tmp.path(), "hy-eq", "title"), "-Fix login");

    // The `--` separator escapes it positionally.
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "bug",
            "--slug",
            "hy-dd",
            "--",
            "-Fix login",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert_eq!(show_field(tmp.path(), "hy-dd", "title"), "-Fix login");
}

// --- #3: built-in list-field hint on `set` -----------------------------

/// `set <slug> related <ref>` for a built-in *list* field must not
/// suggest the non-working `--related (repeatable)`; it must name the
/// flags that actually work — and re-running with that flag must succeed.
#[test]
fn set_related_hint_names_working_flags() {
    let tmp = fresh_repo();
    run_ok(
        tmp.path(),
        &["new", "Anchor", "--type", "bug", "--slug", "an-chor"],
    );
    run_ok(
        tmp.path(),
        &["new", "Other", "--type", "bug", "--slug", "oth-er"],
    );

    let out = run(tmp.path(), &["set", "an-chor", "related", "oth-er"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--add-related") && stderr.contains("--remove-related"),
        "hint must name the working add/remove flags; got:\n{stderr}"
    );
    // The misleading pre-fix wording must be gone.
    assert!(
        !stderr.contains("--related (repeatable)"),
        "hint must not name the non-working `--related (repeatable)`; got:\n{stderr}"
    );

    // BOTH named flags work verbatim from the same slug — execute add
    // then remove, so a typo in either flag name fails the test.
    let out = run(
        tmp.path(),
        &["update", "an-chor", "--add-related", "oth-er"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let out = run(
        tmp.path(),
        &["update", "an-chor", "--remove-related", "oth-er"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
}

/// Same contract for the other built-in list field, `labels`.
#[test]
fn set_labels_hint_names_working_flags() {
    let tmp = fresh_repo();
    run_ok(
        tmp.path(),
        &["new", "Anchor", "--type", "bug", "--slug", "an-chor"],
    );

    let out = run(tmp.path(), &["set", "an-chor", "labels", "urgent"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--add-label") && stderr.contains("--remove-label"),
        "hint must name the working add/remove flags; got:\n{stderr}"
    );
    assert!(
        !stderr.contains("--label (repeatable)"),
        "hint must not name the non-working `--label (repeatable)`; got:\n{stderr}"
    );

    let out = run(tmp.path(), &["update", "an-chor", "--add-label", "urgent"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let out = run(
        tmp.path(),
        &["update", "an-chor", "--remove-label", "urgent"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
}

// --- #5: `note` flag ordering + required `--as` ------------------------

/// `note <slug> "text" --decision --as <author>` — flags trailing the
/// positionals — must parse; the layout can't force a specific order.
#[test]
fn note_flag_order_is_insensitive() {
    let tmp = fresh_repo();
    run_ok(
        tmp.path(),
        &["new", "Anchor", "--type", "bug", "--slug", "an-chor"],
    );

    // The originally-friction order: positionals first, flags last.
    let out = run(
        tmp.path(),
        &[
            "note",
            "an-chor",
            "A decision",
            "--decision",
            "--as",
            "alice",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    // The order the user had to discover also works — proving neither
    // is privileged.
    let out = run(
        tmp.path(),
        &["note", "--as", "alice", "--decision", "an-chor", "Another"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
}

/// Omitting the required `--as` yields clap's *specific* missing-argument
/// diagnostic — an `error:` line naming `--as` — not the bare generic
/// "For more information, try '--help'." fallback that a custom
/// clap-error remap could otherwise leave behind. Regression guard for
/// @note-missing-as-generic-error.
#[test]
fn note_missing_as_names_the_flag() {
    let tmp = fresh_repo();
    run_ok(
        tmp.path(),
        &["new", "Anchor", "--type", "bug", "--slug", "an-chor"],
    );

    let out = run(tmp.path(), &["note", "an-chor", "orphan note"]);
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Two conditions, both of which the buggy "For more information, try
    // '--help'." fallback fails: it must be a rendered `error:` (clap's
    // stable diagnostic prefix — the help/usage fallback carries none),
    // and it must name the missing `--as` flag. We match the durable
    // `error:` prefix + flag name rather than clap's full English
    // sentence, whose exact wording clap can revise across versions (see
    // the note in `new_rejects_both_positional_and_flag_title`).
    assert!(
        stderr.contains("error:") && stderr.contains("--as"),
        "missing-author error must be a clap `error:` naming --as, not the generic help fallback; got:\n{stderr}"
    );
}

/// The same missing-`--as` failure under `--json` renders the unified
/// `{"error":{code:"usage-error"}}` envelope (exit 1) on stderr, with
/// clap's diagnostic preserved inside `message` — the `--json` output
/// contract must not degrade to the generic help line either.
#[test]
fn note_missing_as_json_emits_usage_error_envelope() {
    let tmp = fresh_repo();
    run_ok(
        tmp.path(),
        &["new", "Anchor", "--type", "bug", "--slug", "an-chor"],
    );

    let out = run(tmp.path(), &["--json", "note", "an-chor", "orphan note"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    let envelope: serde_json::Value =
        serde_json::from_slice(&out.stderr).expect("json usage error on stderr");
    assert_eq!(
        envelope["error"]["code"],
        "usage-error",
        "expected usage-error envelope; got:\n{}",
        dump(&out)
    );
    let message = envelope["error"]["message"]
        .as_str()
        .expect("error.message string");
    // Same durable signal as the human-mode test: the envelope must wrap
    // clap's `error:` diagnostic naming `--as`, not the generic help line.
    assert!(
        message.contains("error:") && message.contains("--as"),
        "envelope message must preserve clap's error diagnostic naming --as; got:\n{message}"
    );
}

// --- @intake-bug-issuectl-d6947128f6c9: label flag-form + --json ------
//
// `label` historically took the operation ONLY as a positional
// (`label <slug> add|remove <label>`). Reaching for the `--add`/`--remove`
// flag style the rest of the CLI uses failed with a bare clap `Usage:`
// line, and — the core bug — under `--json` clap's usage error escaped the
// envelope contract entirely on some builds, reading as a silent no-op
// (empty stdout, exit non-zero, mutation skipped). We now (a) accept the
// flag form as an alias for the positional op, and (b) route EVERY
// malformed `label` invocation through clap's `label_target` arg group so
// it is a uniform `usage-error` — exit 2 in human mode, the `usage-error`
// envelope (empty stdout, exit 1, mutation skipped) under `--json` — the
// same code every other clap usage error carries, never a silent skip and
// never the generic `command-failed` a late handler check would have
// produced.
//
// `expect_usage_error_json` pins the full `--json` contract for one
// malformed invocation; the table test below drives every form-shape
// through it so a future clap refactor can't reintroduce the split
// between clap-level and handler-level rejection.

/// Reads the `labels` array of an issue as a `Vec<String>` (empty when the
/// frontmatter has no labels — the `--json` echo renders that as `null`).
fn labels_of(root: &std::path::Path, slug: &str) -> Vec<String> {
    let show = run(root, &["--json", "show", slug]);
    assert_eq!(show.status.code(), Some(0), "{}", dump(&show));
    let v: serde_json::Value =
        serde_json::from_slice(&show.stdout).expect("show stdout should be JSON");
    match &v["labels"] {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|i| i.as_str().expect("label is a string").to_string())
            .collect(),
        _ => Vec::new(),
    }
}

fn seed_issue(root: &std::path::Path, slug: &str) {
    run_ok(
        root,
        &["new", "--type", "task", "--title", slug, "--slug", slug],
    );
}

/// The `--add`/`--remove` flag form is accepted as an alias for the
/// positional operation and mutates the label set the same way.
#[test]
fn label_flag_form_adds_and_removes() {
    let tmp = fresh_repo();
    let r = tmp.path();
    seed_issue(r, "lbl-flag");

    run_ok(r, &["label", "lbl-flag", "--add", "infra"]);
    assert_eq!(labels_of(r, "lbl-flag"), vec!["infra".to_string()]);

    run_ok(r, &["label", "lbl-flag", "--remove", "infra"]);
    assert!(labels_of(r, "lbl-flag").is_empty());
}

/// The flag form routes through the same idempotent mutate path as the
/// positional form: a repeated `--add` is a no-op (no double entry) and
/// `--dry-run` writes nothing.
#[test]
fn label_flag_form_is_idempotent_and_honors_dry_run() {
    let tmp = fresh_repo();
    let r = tmp.path();
    seed_issue(r, "lbl-idem");

    run_ok(r, &["label", "lbl-idem", "--add", "infra"]);
    run_ok(r, &["label", "lbl-idem", "--add", "infra"]);
    assert_eq!(
        labels_of(r, "lbl-idem"),
        vec!["infra".to_string()],
        "repeated --add must not double the label"
    );

    // --dry-run reports success (exit 0) but must not touch the file.
    run_ok(r, &["label", "lbl-idem", "--remove", "infra", "--dry-run"]);
    assert_eq!(
        labels_of(r, "lbl-idem"),
        vec!["infra".to_string()],
        "--dry-run on the flag form must not mutate"
    );
}

/// The exact invocation from the bug report — flag-style `--remove` under
/// `--json` — now succeeds, removes the label, and echoes the resulting
/// (empty) label set rather than silently no-op'ing.
#[test]
fn label_flag_remove_json_applies_and_echoes() {
    let tmp = fresh_repo();
    let r = tmp.path();
    seed_issue(r, "lbl-jsonflag");
    run_ok(r, &["label", "lbl-jsonflag", "add", "needs-triage"]);

    let out = run(
        r,
        &[
            "label",
            "lbl-jsonflag",
            "--remove",
            "needs-triage",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json stdout");
    // Post-mutation the label set is empty (echoed as `null`), and disk agrees.
    assert!(v["labels"].is_null() || v["labels"].as_array().is_some_and(|a| a.is_empty()));
    assert!(labels_of(r, "lbl-jsonflag").is_empty());
}

/// Runs `label <slug-args> --json`, seeding `slug` with the label `keep`
/// first, and asserts the FULL malformed-`--json` contract: exit 1, empty
/// stdout, a `usage-error` envelope on stderr, and — the heart of the bug
/// — the issue's labels untouched (no silent mutation).
fn expect_usage_error_json(label_args: &[&str], slug: &str) {
    let tmp = fresh_repo();
    let r = tmp.path();
    seed_issue(r, slug);
    run_ok(r, &["label", slug, "add", "keep"]);

    let mut args = vec!["label"];
    args.extend_from_slice(label_args);
    args.push("--json");
    let out = run(r, &args);

    assert_eq!(
        out.status.code(),
        Some(1),
        "malformed label under --json must exit 1; {}",
        dump(&out)
    );
    assert!(
        out.stdout.is_empty(),
        "stdout must be empty under --json on failure; {}",
        dump(&out)
    );
    let envelope: serde_json::Value =
        serde_json::from_slice(&out.stderr).expect("json error envelope on stderr");
    assert_eq!(
        envelope["error"]["code"],
        "usage-error",
        "every malformed label invocation must carry the shared usage-error \
         code, not command-failed; {}",
        dump(&out)
    );
    // The heart of the bug: the mutation must NOT have been applied.
    assert_eq!(
        labels_of(r, slug),
        vec!["keep".to_string()],
        "labels must be untouched on a rejected invocation; {}",
        dump(&out)
    );
}

/// CORE REGRESSION: every malformed shape of `label … --json` — no
/// operation, an operation with no label, a bad enum value, a flag with no
/// value, an empty flag value, or any mix of the positional and flag forms
/// (both orderings) — MUST emit the `usage-error` envelope with empty
/// stdout and leave the labels untouched. The silent-no-op the bug
/// described must never recur, and the code must never regress to the
/// generic `command-failed`.
#[test]
fn label_json_malformed_variants_emit_usage_error_and_skip_mutation() {
    // Each case is a distinct malformed shape; the tail slug keeps the
    // seeded repos isolated. `--json` is appended by the helper.
    let cases: &[(&[&str], &str)] = &[
        (&["lbl-bare"], "lbl-bare"),                     // no op at all
        (&["lbl-noarg", "add"], "lbl-noarg"),            // op, no <label>
        (&["lbl-noarg2", "remove"], "lbl-noarg2"),       // op, no <label>
        (&["lbl-badenum", "frobnicate"], "lbl-badenum"), // bad enum value
        (&["lbl-flagnoval", "--add"], "lbl-flagnoval"),  // flag, no value
        (&["lbl-empty", "--add", ""], "lbl-empty"),      // empty flag value
        (&["lbl-mix1", "add", "x", "--remove", "y"], "lbl-mix1"),
        (&["lbl-mix2", "remove", "x", "--add", "y"], "lbl-mix2"),
        (&["lbl-mix3", "--add", "x", "--remove", "y"], "lbl-mix3"),
        (&["lbl-mix4", "--add", "x", "remove", "y"], "lbl-mix4"),
    ];
    for (args, slug) in cases {
        expect_usage_error_json(args, slug);
    }
}

/// The same malformed shapes in HUMAN mode stay clap usage errors (exit 2),
/// unchanged from before the flag form existed — making the positionals
/// optional at the clap layer must not silently downgrade the exit code to
/// a runtime failure (exit 1).
#[test]
fn label_malformed_human_mode_keeps_usage_exit_code() {
    let tmp = fresh_repo();
    let r = tmp.path();
    seed_issue(r, "lbl-exit");
    let cases: &[&[&str]] = &[
        &["label", "lbl-exit"],
        &["label", "lbl-exit", "add"],
        &["label", "lbl-exit", "frobnicate"],
        &["label", "lbl-exit", "add", "x", "--remove", "y"],
        &["label", "lbl-exit", "--add", "x", "--remove", "y"],
    ];
    for args in cases {
        let out = run(r, args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`{args:?}` must exit 2 (clap usage error); {}",
            dump(&out)
        );
    }
}
