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

/// Omitting the required `--as` yields a targeted usage error that names
/// `--as`, not a generic "reorder your args" failure.
#[test]
fn note_missing_as_names_the_flag() {
    let tmp = fresh_repo();
    run_ok(
        tmp.path(),
        &["new", "Anchor", "--type", "bug", "--slug", "an-chor"],
    );

    let out = run(
        tmp.path(),
        &["note", "an-chor", "orphan note", "--decision"],
    );
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--as"),
        "missing-author error must name --as; got:\n{stderr}"
    );
}
