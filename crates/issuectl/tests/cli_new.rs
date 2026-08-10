//! Golden tests for `issuectl new`'s terminal output. These lock the
//! byte-identical CLI output promise from the typed-error refactor
//! (commit c272404) end-to-end. The inline test in `src/mutate.rs`
//! (`do_new_error_to_anyhow_text_matches_per_variant`) covers the
//! `From<DoNewError> for anyhow::Error` conversion but cannot observe
//! `cmd_new`'s `println!` formatting or `main()`'s `anyhow::Error`
//! Debug rendering. These tests fill that gap.
//!
//! Assertions for project-owned text (validation/conflict/schema-violation
//! and our own framing of `cannot parse`/`cannot create`/`Caused by:`) are
//! exact byte-for-byte. Assertions on text owned by upstream crates
//! (`serde_yaml` parser diagnostics, `std::io::Error` Display) are
//! relaxed to substring/prefix matches so dep/toolchain bumps don't
//! break the suite without an underlying CLI regression.
//!
//! Convention: see `AGENTS.md` (`Tests`) for when integration tests in
//! `tests/` are warranted vs. inline `#[cfg(test)]` modules.

use std::process::{Command, Output};

use tempfile::TempDir;

/// Creates a tempdir with an empty `issues/` directory. The first
/// `issuectl new` invocation against the result will bootstrap a
/// default `.schema.yaml`. Tests that need a pre-existing schema
/// should write one directly after calling this.
fn fresh_repo() -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("issues")).expect("mkdir issues");
    tmp
}

fn run(root: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_issuectl"))
        // Strip env that mutates anyhow's Debug rendering or could
        // localize `std::io::Error` strings. Without this, a developer
        // running `RUST_BACKTRACE=1 cargo test` sees the IO test fail
        // with no underlying regression.
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        // `--root` already pins the repo, but `current_dir` neutralises
        // any cwd-walking code paths that could pick up the developer's
        // checkout state if they `cargo test` from inside an issuectl
        // working tree.
        .current_dir(root)
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .expect("spawn issuectl")
}

fn dump(out: &Output) -> String {
    format!(
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    )
}

fn assert_failure(out: &Output, expected_stderr: &str) {
    assert_eq!(out.status.code(), Some(1), "{}", dump(out));
    assert!(
        out.stdout.is_empty(),
        "expected empty stdout; {}",
        dump(out)
    );
    let actual = String::from_utf8_lossy(&out.stderr);
    assert_eq!(actual.as_ref(), expected_stderr, "{}", dump(out));
}

#[test]
fn new_success_writes_file_and_prints_created_lines() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "new", "--type", "bug", "--title", "Hello", "--slug", "ab-cd",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let item_path = tmp.path().join("issues/ab-cd/item.md");
    assert!(
        item_path.is_file(),
        "expected {} to exist",
        item_path.display()
    );
    let expected = format!("Created ab-cd: Hello\n  {}\n", item_path.display());
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    assert!(out.stderr.is_empty(), "{}", dump(&out));
}

#[test]
fn new_json_success_prints_expected_payload() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "--json", "new", "--type", "bug", "--title", "Hello", "--slug", "ab-cd",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert!(out.stderr.is_empty(), "{}", dump(&out));
    let item_path = tmp.path().join("issues/ab-cd/item.md");
    assert!(
        item_path.is_file(),
        "expected {} to exist",
        item_path.display()
    );
    let dir = tmp.path().join("issues/ab-cd");
    // Unified field vocabulary: `dir` = issue directory, `path` = item.md.
    let expected = format!(
        "{{\n  \"dir\": \"{}\",\n  \"path\": \"{}\",\n  \"slug\": \"ab-cd\",\n  \"title\": \"Hello\",\n  \"warnings\": []\n}}\n",
        dir.display(),
        item_path.display(),
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}

#[test]
fn new_with_reserved_notes_section_warns_on_stderr_but_succeeds() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "bug",
            "--title",
            "Hello",
            "--slug",
            "notes-warn",
            "--description",
            "intro\n\n## Notes\n\nlegacy",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning:") && stderr.contains("## Notes"),
        "expected a reserved-section warning on stderr; {}",
        dump(&out)
    );
    assert!(tmp.path().join("issues/notes-warn/item.md").is_file());
}

#[test]
fn new_json_with_reserved_notes_section_populates_warnings() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "--json",
            "new",
            "--type",
            "bug",
            "--title",
            "Hello",
            "--slug",
            "notes-json",
            "--description",
            "intro\n\n## Notes\n\nlegacy",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("new --json stdout should be JSON");
    let warnings = v["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1, "{}", dump(&out));
    assert!(warnings[0].as_str().unwrap().contains("## Notes"));
}

#[test]
fn new_with_body_file_reserved_section_warns() {
    let tmp = fresh_repo();
    let body_path = tmp.path().join("body.md");
    std::fs::write(&body_path, "intro\n\n## Notes\n\nlegacy\n").expect("write body file");
    let out = run(
        tmp.path(),
        &[
            "--json",
            "new",
            "--type",
            "bug",
            "--title",
            "Hello",
            "--slug",
            "bf-notes",
            "--body-file",
            body_path.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("new --json stdout should be JSON");
    let warnings = v["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1, "{}", dump(&out));
    assert!(warnings[0].as_str().unwrap().contains("## Notes"));
}

#[test]
fn body_set_with_reserved_section_warns_on_stderr_and_json() {
    let tmp = fresh_repo();
    // Create the target issue first (clean body, no warning).
    let created = run(
        tmp.path(),
        &[
            "new", "--type", "bug", "--title", "Target", "--slug", "bs-notes",
        ],
    );
    assert_eq!(created.status.code(), Some(0), "{}", dump(&created));

    let body_path = tmp.path().join("newbody.md");
    std::fs::write(&body_path, "fresh\n\n## Notes\n\nlegacy\n").expect("write body file");

    // Human mode: warning on stderr, write succeeds.
    let human = run(
        tmp.path(),
        &[
            "body",
            "set",
            "bs-notes",
            "--from-file",
            body_path.to_str().unwrap(),
        ],
    );
    assert_eq!(human.status.code(), Some(0), "{}", dump(&human));
    let stderr = String::from_utf8_lossy(&human.stderr);
    assert!(
        stderr.contains("warning:") && stderr.contains("## Notes"),
        "expected reserved-section warning on stderr; {}",
        dump(&human)
    );

    // JSON mode: warnings array populated.
    let json = run(
        tmp.path(),
        &[
            "--json",
            "body",
            "set",
            "bs-notes",
            "--from-file",
            body_path.to_str().unwrap(),
        ],
    );
    assert_eq!(json.status.code(), Some(0), "{}", dump(&json));
    let v: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("body set --json stdout should be JSON");
    let warnings = v["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1, "{}", dump(&json));
    assert!(warnings[0].as_str().unwrap().contains("## Notes"));
}

/// Reads `field` from an `issuectl --json show <slug>` payload.
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

/// `low` is the lowest priority value: accepted on `new`, `ls -p`, and
/// `update`, while the default stays `normal`. Guards the widened
/// `PRIORITIES` set against a regression back to two-valued (and against
/// the default silently shifting off `normal`).
#[test]
fn priority_low_is_accepted_end_to_end() {
    let tmp = fresh_repo();

    // `new --priority low` succeeds and persists the field (verified
    // through the machine-facing `show` payload, not raw file text).
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "bug",
            "--title",
            "Low one",
            "--slug",
            "lo-one",
            "--priority",
            "low",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert_eq!(show_field(tmp.path(), "lo-one", "priority"), "low");

    // A second issue with the default priority proves `-p low` actually
    // filters — without it, `ls -p low` returning `[lo-one]` would pass
    // even if the filter were ignored. It also asserts the default is
    // still `normal`, not `low`.
    let out = run(
        tmp.path(),
        &[
            "new", "--type", "task", "--title", "Bump me", "--slug", "bu-mp",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert_eq!(show_field(tmp.path(), "bu-mp", "priority"), "normal");

    // `ls -p low` returns only the low issue, excluding the normal one.
    let out = run(tmp.path(), &["--json", "ls", "-p", "low"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("ls stdout should be JSON");
    let slugs: Vec<&str> = v
        .as_array()
        .expect("ls returns an array")
        .iter()
        .map(|i| i["slug"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(slugs, vec!["lo-one"], "{}", dump(&out));

    // `update --priority low` is accepted and changes the persisted value.
    let version = show_field(tmp.path(), "bu-mp", "version");
    let out = run(
        tmp.path(),
        &[
            "--json",
            "update",
            "bu-mp",
            "--priority",
            "low",
            "--expected-version",
            &version,
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert_eq!(show_field(tmp.path(), "bu-mp", "priority"), "low");
}

/// A priority outside the widened set is still rejected, and the clap
/// usage error advertises exactly `[low, normal, high]`. Guards against
/// the parser being loosened (e.g. swapped to a bare `String`) when the
/// accepted set changed.
#[test]
fn priority_outside_set_is_rejected_with_possible_values() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "task",
            "--title",
            "Nope",
            "--slug",
            "no-pe",
            "--priority",
            "medium",
        ],
    );
    // clap usage errors exit 2 (not the app's 1).
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("[possible values: low, normal, high]"),
        "expected possible-values list in usage error; got:\n{stderr}"
    );
}

/// The unified `--json` error contract: a failing command under `--json`
/// emits `{"error":{"code","message"}}` to stderr (not the bare
/// `Error: …` line) and leaves stdout empty.
#[test]
fn json_error_contract_emits_structured_error() {
    let tmp = fresh_repo();
    let out = run(tmp.path(), &["--json", "show", "does-not-exist"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr should be JSON");
    assert_eq!(v["error"]["code"], "not-found", "{}", dump(&out));
    assert_eq!(
        v["error"]["message"],
        "issue does-not-exist not found",
        "{}",
        dump(&out)
    );
}

/// A clap usage error (unknown flag) under `--json` is caught in `main`
/// and re-emitted as the shared envelope with `code:"usage-error"` and
/// exit 1 — not clap's plain-text stderr + exit 2.
#[test]
fn json_error_contract_wraps_clap_usage_errors() {
    let tmp = fresh_repo();
    let out = run(tmp.path(), &["--json", "show", "ab-cd", "--bogus-flag"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr should be JSON");
    assert_eq!(v["error"]["code"], "usage-error", "{}", dump(&out));
}

/// A bubble-up `MutateError::NotFound` under `--json` is classified with
/// the stable `not-found` code — the same code the read paths emit — not
/// the generic `command-failed`. Every write verb (`update`/`close`/
/// `set`/`note`/`check`/`label`/`depend`/`body set`) raises `NotFound` on
/// a missing slug; `main` downcasts the typed error rather than
/// string-matching flattened anyhow text, so an agent branches on the
/// code instead of grepping the message. Reserve `command-failed` for a
/// genuinely opaque failure — see `json_write_wrong_expected_version_still_conflicts`.
#[test]
fn json_error_contract_classifies_write_not_found() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "--json",
            "update",
            "no-such-issue",
            "--status",
            "in-progress",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr should be JSON");
    assert_eq!(v["error"]["code"], "not-found", "{}", dump(&out));
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not found"),
        "{}",
        dump(&out)
    );
}

/// Every mutate-layer write verb — not just `update` — must classify a
/// missing slug as `not-found`, and the `--json` `message` must be the
/// exact `MutateError::NotFound` Display string (`issue not found`). The
/// eight verbs route through distinct mutate functions
/// (`update_issue`/`close_issue`/`toggle_checkbox`/`update_body`/note
/// append), so a single site regressing to a string-flattened
/// `anyhow!("{e}")` would silently escape classification back to
/// `command-failed`. Pinning the exact message also guards the byte
/// parity that holds only because `MutateError` exposes no `source()`.
/// `body set` reads stdin, so it takes a separate spawn path.
#[test]
fn json_all_write_verbs_classify_missing_issue_as_not_found() {
    // (label, argv) for the seven verbs that need no stdin. A missing
    // slug must reach the mutate layer and bubble `NotFound`.
    let cases: &[(&str, &[&str])] = &[
        (
            "update",
            &[
                "--json",
                "update",
                "no-such-issue",
                "--status",
                "in-progress",
            ],
        ),
        (
            "close",
            &["--json", "close", "no-such-issue", "--status", "fixed"],
        ),
        (
            "set",
            &["--json", "set", "no-such-issue", "priority", "high"],
        ),
        (
            "note",
            &[
                "--json",
                "note",
                "no-such-issue",
                "--as",
                "tester",
                "a note",
            ],
        ),
        ("check", &["--json", "check", "no-such-issue", "a task"]),
        (
            "label",
            &["--json", "label", "no-such-issue", "add", "mylabel"],
        ),
        (
            "depend",
            &[
                "--json",
                "depend",
                "add",
                "no-such-issue",
                "--blocked-by",
                "other-issue",
            ],
        ),
    ];
    for (label, args) in cases {
        let tmp = fresh_repo();
        let out = run(tmp.path(), args);
        assert_eq!(out.status.code(), Some(1), "{label}: {}", dump(&out));
        assert!(out.stdout.is_empty(), "{label}: {}", dump(&out));
        let v: serde_json::Value = serde_json::from_slice(&out.stderr)
            .unwrap_or_else(|_| panic!("{label}: stderr should be JSON:\n{}", dump(&out)));
        assert_eq!(v["error"]["code"], "not-found", "{label}: {}", dump(&out));
        assert_eq!(
            v["error"]["message"],
            "issue not found",
            "{label}: {}",
            dump(&out)
        );
    }

    // `body set` reads the new body from stdin, so it can't go through
    // `run`. The missing slug still bubbles `NotFound` after the body is
    // read.
    let tmp = fresh_repo();
    let out = Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env("LC_ALL", "C")
        .current_dir(tmp.path())
        .arg("--root")
        .arg(tmp.path())
        .args(["--json", "body", "set", "no-such-issue", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(b"# rewritten\n")?;
            child.wait_with_output()
        })
        .expect("spawn body set");
    assert_eq!(out.status.code(), Some(1), "body set: {}", dump(&out));
    assert!(out.stdout.is_empty(), "body set: {}", dump(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr)
        .unwrap_or_else(|_| panic!("body set: stderr should be JSON:\n{}", dump(&out)));
    assert_eq!(v["error"]["code"], "not-found", "body set: {}", dump(&out));
    assert_eq!(
        v["error"]["message"],
        "issue not found",
        "body set: {}",
        dump(&out)
    );
}

/// `--expected-version` is OPTIONAL on `--json` writes (opt-in
/// compare-and-swap, superseding design D4=B). A `--json` `update`
/// without a token now SUCCEEDS — symmetric with the human path — and
/// its result carries the new canonical `version` at the top level,
/// matching `show --json`. Guards against a regression back to the
/// mandatory-token surface that tripped agent callers who added `--json`
/// only for parseable output.
#[test]
fn json_update_without_expected_version_succeeds() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "new", "--type", "task", "--title", "Optional", "--slug", "op-ti",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    let out = run(
        tmp.path(),
        &["--json", "update", "op-ti", "--priority", "low"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("update stdout should be JSON");
    assert!(
        v["version"].as_str().is_some_and(|s| !s.is_empty()),
        "update result should carry a top-level `version`; {}",
        dump(&out)
    );
    assert_eq!(show_field(tmp.path(), "op-ti", "priority"), "low");
    // The reported version matches the persisted canonical version.
    assert_eq!(
        v["version"].as_str().unwrap(),
        show_field(tmp.path(), "op-ti", "version"),
        "{}",
        dump(&out)
    );
}

/// `--json` `close` without `--expected-version` succeeds and carries a
/// top-level `version`, the same top-level key `show --json` exposes.
#[test]
fn json_close_without_expected_version_succeeds() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "new", "--type", "bug", "--title", "Closes", "--slug", "cl-os",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    let out = run(
        tmp.path(),
        &["--json", "close", "cl-os", "--status", "fixed"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("close stdout should be JSON");
    assert!(
        v["version"].as_str().is_some_and(|s| !s.is_empty()),
        "close result should carry a top-level `version`; {}",
        dump(&out)
    );
    assert_eq!(v["moved_to_closed"], true, "{}", dump(&out));
}

/// The compare-and-swap remains fully honored when a token IS passed
/// (opt-in): a WRONG `--expected-version` still fails the conflict path
/// (exit 1), while the CORRECT token succeeds. Removing the *requirement*
/// to pass a token must not remove the CAS check itself.
#[test]
fn json_write_wrong_expected_version_still_conflicts() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &["new", "--type", "task", "--title", "Cas", "--slug", "ca-sw"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    // Wrong token → conflict (exit 1), write refused. Asserting the
    // error envelope (not merely exit 1) proves the CAS comparison
    // actually ran: a mismatch surfaces as `command-failed` carrying the
    // `version mismatch` message, not some incidental failure.
    let out = run(
        tmp.path(),
        &[
            "--json",
            "update",
            "ca-sw",
            "--priority",
            "high",
            "--expected-version",
            "sha256:v1:deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let err: serde_json::Value =
        serde_json::from_slice(&out.stderr).expect("conflict stderr should be JSON");
    assert_eq!(err["error"]["code"], "command-failed", "{}", dump(&out));
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("version mismatch"),
        "conflict must report a version mismatch; {}",
        dump(&out)
    );
    assert_eq!(show_field(tmp.path(), "ca-sw", "priority"), "normal");

    // Correct token → success.
    let version = show_field(tmp.path(), "ca-sw", "version");
    let out = run(
        tmp.path(),
        &[
            "--json",
            "update",
            "ca-sw",
            "--priority",
            "high",
            "--expected-version",
            &version,
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert_eq!(show_field(tmp.path(), "ca-sw", "priority"), "high");
}

/// The requirement was dropped from *eight* command handlers, each with
/// its own argument parsing and mutation helper. `update`/`close`/`assign`
/// are covered above; this exercises the remaining machine-write verbs
/// (`set`, `note`, `label`, `depend`, `body set`) so a stray `bail!`
/// reintroduced into any one of them fails a test. Each: a `--json` write
/// WITHOUT a token succeeds and its result carries the top-level `version`
/// matching `show --json`.
#[test]
fn json_remaining_verbs_tokenless_writes_succeed_with_version() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "new", "--type", "task", "--title", "Verbs", "--slug", "ve-rb",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    // A second issue to serve as a `depend` blocker.
    let out = run(
        tmp.path(),
        &[
            "new", "--type", "task", "--title", "Block", "--slug", "bl-ok",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    // Each entry is a tokenless `--json` write. `body` is the only two-word
    // verb; the rest are single subcommands.
    let writes: &[&[&str]] = &[
        &["--json", "set", "ve-rb", "priority", "high"],
        &["--json", "note", "ve-rb", "--as", "tester", "a note"],
        &["--json", "label", "ve-rb", "add", "backend"],
        &["--json", "depend", "add", "ve-rb", "--blocked-by", "bl-ok"],
    ];
    for args in writes {
        let out = run(tmp.path(), args);
        assert_eq!(out.status.code(), Some(0), "args {args:?}: {}", dump(&out));
        let v: serde_json::Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|_| panic!("stdout should be JSON for {args:?}: {}", dump(&out)));
        assert_eq!(
            v["version"].as_str().unwrap_or_default(),
            show_field(tmp.path(), "ve-rb", "version"),
            "args {args:?} must report the persisted top-level version; {}",
            dump(&out)
        );
    }

    // `body set` reads from stdin, so it takes a separate path.
    let out = Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env("LC_ALL", "C")
        .current_dir(tmp.path())
        .arg("--root")
        .arg(tmp.path())
        .args(["--json", "body", "set", "ve-rb", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"# Verbs\n\nrewritten body\n")?;
            child.wait_with_output()
        })
        .expect("spawn body set");
    assert_eq!(out.status.code(), Some(0), "body set: {}", dump(&out));
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("body set stdout should be JSON");
    assert_eq!(
        v["version"].as_str().unwrap_or_default(),
        show_field(tmp.path(), "ve-rb", "version"),
        "body set must report the persisted top-level version; {}",
        dump(&out)
    );

    // `check` needs a checkbox in the body; the `body set` above installed
    // none, so add one, then toggle it tokenless.
    let out = Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env("LC_ALL", "C")
        .current_dir(tmp.path())
        .arg("--root")
        .arg(tmp.path())
        .args(["--json", "body", "set", "ve-rb", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"# Verbs\n\n- [ ] finish the task\n")?;
            child.wait_with_output()
        })
        .expect("spawn body set for checkbox");
    assert_eq!(
        out.status.code(),
        Some(0),
        "body set checkbox: {}",
        dump(&out)
    );

    let out = run(tmp.path(), &["--json", "check", "ve-rb", "finish the task"]);
    assert_eq!(out.status.code(), Some(0), "check: {}", dump(&out));
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("check stdout should be JSON");
    assert_eq!(
        v["version"].as_str().unwrap_or_default(),
        show_field(tmp.path(), "ve-rb", "version"),
        "check must report the persisted top-level version; {}",
        dump(&out)
    );
}

/// End-to-end: with no `--slug`, the built binary derives the slug from
/// the title (the flipped default). Observed through the `--json` payload
/// and the on-disk directory the process created — neither visible to an
/// inline test.
#[test]
fn new_without_slug_derives_from_title() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "--json",
            "new",
            "--type",
            "bug",
            "--title",
            "Login redirect loops on safari",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("new stdout should be JSON");
    assert_eq!(v["slug"], "login-redirect-loops", "{}", dump(&out));
    assert!(
        tmp.path()
            .join("issues/login-redirect-loops/item.md")
            .is_file(),
        "{}",
        dump(&out)
    );
}

/// `--slug-random` and `--slug` are mutually exclusive — a clap-level
/// conflict observable only through the built binary's arg parser (exit 2,
/// "cannot be used with"). An explicit slug always wins, so forcing random
/// at the same time is a usage error rather than a silent precedence rule.
#[test]
fn new_slug_random_conflicts_with_slug() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "bug",
            "--title",
            "x",
            "--slug",
            "ab-cd",
            "--slug-random",
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot be used with"),
        "stderr was: {stderr:?}"
    );
}

#[test]
fn new_validation_owner_on_non_epic_fails() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &["new", "--type", "bug", "--title", "x", "--owner", "alice"],
    );
    assert_failure(&out, "Error: --owner is only valid with --type epic\n");
}

#[test]
fn new_conflict_slug_already_taken_fails() {
    let tmp = fresh_repo();
    let taken = tmp.path().join("issues/taken-slug");
    std::fs::create_dir_all(&taken).unwrap();
    std::fs::write(taken.join("item.md"), "---\nstatus: open\n---\n").unwrap();

    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "bug",
            "--title",
            "x",
            "--slug",
            "taken-slug",
        ],
    );
    let expected = format!(
        "Error: slug \"taken-slug\" already exists at {}; retry with a different --slug \
         or omit --slug to get a random auto-generated one\n",
        taken.display()
    );
    assert_failure(&out, &expected);
}

#[test]
fn new_schema_violation_missing_required_field_fails() {
    let tmp = fresh_repo();
    std::fs::write(
        tmp.path().join("issues/.schema.yaml"),
        "version: 1\nfields:\n  team:\n    required: true\n",
    )
    .unwrap();

    let out = run(tmp.path(), &["new", "--type", "bug", "--title", "x"]);
    assert_failure(&out, "Error: schema: missing required field \"team\"\n");
}

#[test]
fn new_schema_config_malformed_yaml_fails() {
    // Locks our `Error: cannot parse <path>: ` framing exactly. The
    // serde_yaml diagnostic wording that follows is third-party text;
    // a substring check tolerates dep bumps without losing coverage of
    // the project-owned framing.
    let tmp = fresh_repo();
    let schema_path = tmp.path().join("issues/.schema.yaml");
    std::fs::write(&schema_path, "version: 1\nfields: : :\n").unwrap();

    let out = run(tmp.path(), &["new", "--type", "bug", "--title", "x"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let expected_prefix = format!("Error: cannot parse {}: ", schema_path.display());
    assert!(
        stderr.starts_with(&expected_prefix),
        "stderr should start with {expected_prefix:?}, got {stderr:?}"
    );
    assert!(
        stderr.contains("mapping values"),
        "stderr should mention `mapping values` (serde_yaml diagnostic), got {stderr:?}"
    );
    assert!(
        stderr.ends_with('\n'),
        "stderr should end with newline, got {stderr:?}"
    );
}

#[cfg(unix)]
#[test]
fn new_io_failure_chmod_readonly_fails() {
    // Mirrors `mutate::tests::new_issue_io_failure_returns_typed_error`:
    // pre-seed a `.schema.yaml` (so we get past `ensure_default_written`),
    // then chmod 0o500 on `issues/` so `fs::create_dir(<root>/issues/<slug>)`
    // fails with EACCES. RAII guard restores perms on every exit so the
    // tempdir cleanup never inherits a 0o500 directory.
    //
    // Setup writes the schema file directly rather than seeding via a
    // successful `new` run, so this test's failure mode is independent
    // of regressions in the success path.
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    struct PermGuard {
        path: PathBuf,
        original: std::fs::Permissions,
    }
    impl Drop for PermGuard {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.path, self.original.clone());
        }
    }

    let tmp = fresh_repo();
    let issues_dir = tmp.path().join("issues");
    // Minimal valid schema. Existence alone is enough to short-circuit
    // `ensure_default_written`; contents only need to parse.
    std::fs::write(issues_dir.join(".schema.yaml"), "version: 1\n").unwrap();

    let original = std::fs::metadata(&issues_dir).unwrap().permissions();
    // Install the guard BEFORE chmod so a panic in the (vanishingly
    // small) window between `set_permissions` and `let _guard` cannot
    // leak a 0o500 directory into tempdir cleanup.
    let _guard = PermGuard {
        path: issues_dir.clone(),
        original: original.clone(),
    };
    let mut readonly = original.clone();
    readonly.set_mode(0o500);
    std::fs::set_permissions(&issues_dir, readonly).unwrap();

    // chmod 0o500 has no effect for uid 0; skip with a visible log
    // when a probe write still succeeds (CI containers occasionally
    // run as root). Mirrors the uid-0 escape hatch in the inline test.
    let probe = issues_dir.join(".io-probe");
    let chmod_enforced = std::fs::write(&probe, b"x").is_err();
    let _ = std::fs::remove_file(&probe);
    if !chmod_enforced {
        eprintln!(
            "skipping new_io_failure_chmod_readonly: chmod 0500 did not prevent writes to {} \
             (likely running as root)",
            issues_dir.display()
        );
        return;
    }

    let target = issues_dir.join("io-fail-slug");
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "bug",
            "--title",
            "x",
            "--slug",
            "io-fail-slug",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Lock our framing and anyhow's multi-line Caused-by rendering.
    let expected_prefix = format!(
        "Error: cannot create {}\n\nCaused by:\n    ",
        target.display()
    );
    assert!(
        stderr.starts_with(&expected_prefix),
        "stderr should start with {expected_prefix:?}, got {stderr:?}"
    );
    // `Permission denied` is `std::io::Error` Display text — substring
    // match tolerates the libc/Rust formatting of the trailing
    // `(os error N)` decoration.
    assert!(
        stderr.contains("Permission denied"),
        "stderr should mention `Permission denied`, got {stderr:?}"
    );
    assert!(
        stderr.ends_with('\n'),
        "stderr should end with newline, got {stderr:?}"
    );
}

/// Reads the `body` field from an `issuectl --json show <slug>` payload.
fn show_body(root: &std::path::Path, slug: &str) -> String {
    let show = run(root, &["--json", "show", slug]);
    assert_eq!(show.status.code(), Some(0), "{}", dump(&show));
    serde_json::from_slice::<serde_json::Value>(&show.stdout).expect("show stdout should be JSON")
        ["body"]
        .as_str()
        .expect("body field")
        .to_string()
}

#[test]
fn new_body_file_writes_markdown_below_heading() {
    let tmp = fresh_repo();
    let notes = tmp.path().join("notes.md");
    std::fs::write(&notes, "First paragraph.\n\nSecond paragraph.\n").expect("write notes");
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "feature",
            "--title",
            "From a file",
            "--slug",
            "bf-file",
            "--body-file",
            notes.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let body = show_body(tmp.path(), "bf-file");
    // Structural, not just substring: the file markdown must land below
    // the `# <title>` heading, under the `## Description` section the
    // shared renderer emits, in order — proving it flowed through the
    // same write path as an inline `--description` rather than being
    // dropped into frontmatter or before the title.
    let title = body.find("# From a file").expect("title heading");
    let desc = body.find("## Description").expect("description heading");
    let first = body.find("First paragraph.").expect("first paragraph");
    let second = body.find("Second paragraph.").expect("second paragraph");
    assert!(
        title < desc && desc < first && first < second,
        "body out of order: {body:?}"
    );
}

#[test]
fn new_body_file_preserves_leading_indentation() {
    // End-to-end guard for the trim_end (not trim) contract: a file that
    // opens with a 4-space indented code block must survive into the
    // stored body verbatim, so the rendered issue is a valid Markdown
    // code block — the leading whitespace is the author's intent.
    let tmp = fresh_repo();
    let notes = tmp.path().join("code.md");
    std::fs::write(&notes, "    let x = 1;\n\nprose after.\n").expect("write notes");
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "feature",
            "--title",
            "Indented",
            "--slug",
            "bf-indent",
            "--body-file",
            notes.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let body = show_body(tmp.path(), "bf-indent");
    assert!(
        body.contains("    let x = 1;"),
        "leading indentation lost: {body:?}"
    );
}

#[test]
fn new_body_file_dash_reads_stdin() {
    use std::io::Write;
    use std::process::Stdio;

    let tmp = fresh_repo();
    let mut child = Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .current_dir(tmp.path())
        .arg("--root")
        .arg(tmp.path())
        .args([
            "new",
            "--type",
            "feature",
            "--title",
            "From stdin",
            "--slug",
            "bf-stdin",
            "--body-file",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn issuectl");
    child
        .stdin
        .take()
        .expect("stdin handle")
        .write_all(b"Body piped in via stdin.\n")
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let body = show_body(tmp.path(), "bf-stdin");
    assert!(body.contains("# From stdin"), "body was: {body:?}");
    assert!(
        body.contains("Body piped in via stdin."),
        "body was: {body:?}"
    );
}

#[test]
fn new_body_file_conflicts_with_body_plain_is_clap_usage_error() {
    let tmp = fresh_repo();
    let notes = tmp.path().join("notes.md");
    std::fs::write(&notes, "x\n").expect("write notes");
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "feature",
            "--title",
            "Conflict",
            "--body-file",
            notes.to_str().unwrap(),
            "--body",
            "inline",
        ],
    );
    // clap conflict → clap's own usage error, its default exit code 2,
    // nothing created.
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot be used with"),
        "stderr was: {stderr:?}"
    );
}

#[test]
fn new_body_file_conflicts_with_body_json_is_usage_error_envelope() {
    let tmp = fresh_repo();
    let notes = tmp.path().join("notes.md");
    std::fs::write(&notes, "x\n").expect("write notes");
    let out = run(
        tmp.path(),
        &[
            "--json",
            "new",
            "--type",
            "feature",
            "--title",
            "Conflict",
            "--body-file",
            notes.to_str().unwrap(),
            "--body",
            "inline",
        ],
    );
    // Under `--json` the conflict is re-emitted as the shared
    // `usage-error` envelope on stderr at exit 1 (AGENTS.md contract).
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let v: serde_json::Value =
        serde_json::from_slice(&out.stderr).expect("stderr should be a JSON error envelope");
    assert_eq!(v["error"]["code"], "usage-error", "{}", dump(&out));
}

#[test]
fn new_body_file_missing_path_errors_cleanly() {
    let tmp = fresh_repo();
    let missing = tmp.path().join("does-not-exist.md");
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "feature",
            "--title",
            "Missing",
            "--body-file",
            missing.to_str().unwrap(),
        ],
    );
    // A missing body file is a clean error envelope, not a panic.
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot read body"),
        "stderr was: {stderr:?}"
    );
}

#[test]
fn new_body_file_missing_path_json_is_clean_envelope() {
    // The AI-first `--json` contract: a missing body file must produce
    // the shared error envelope on stderr with empty stdout and a stable
    // string `code`, so an agent branches on the envelope rather than
    // crashing. (A missing input file classifies as `command-failed`,
    // like `note --from-file`; the point tested here is a well-formed
    // envelope, not the specific code.)
    let tmp = fresh_repo();
    let missing = tmp.path().join("does-not-exist.md");
    let out = run(
        tmp.path(),
        &[
            "--json",
            "new",
            "--type",
            "feature",
            "--title",
            "Missing",
            "--slug",
            "bf-missing",
            "--body-file",
            missing.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let v: serde_json::Value =
        serde_json::from_slice(&out.stderr).expect("stderr should be a JSON error envelope");
    assert!(v["error"]["code"].is_string(), "{}", dump(&out));
    // No partial issue was created.
    let show = run(tmp.path(), &["--json", "show", "bf-missing"]);
    assert_ne!(show.status.code(), Some(0), "{}", dump(&show));
}

/// Regression: `--json show` must surface `blocked_by` as a top-level key
/// after `depend add --blocked-by`, not bury it inside `extra`. Before the
/// fix the key was absent from the object entirely (`jq .blocked_by` →
/// null) even though the frontmatter carried it. Also pins the derived
/// reverse `blocks` view on the blocker side, and that both keys are
/// present (as empty arrays) when an issue has no dependants.
#[test]
fn show_json_exposes_blocked_by_and_derived_blocks() {
    let tmp = fresh_repo();
    // Two issues: `dp-end` will be blocked by `bl-ocker`.
    for (title, slug) in [("Dependent", "dp-end"), ("Blocker", "bl-ocker")] {
        let out = run(
            tmp.path(),
            &["new", "--type", "task", "--title", title, "--slug", slug],
        );
        assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    }
    let out = run(
        tmp.path(),
        &[
            "--json",
            "depend",
            "add",
            "dp-end",
            "--blocked-by",
            "bl-ocker",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    // Dependent side: `blocked_by` present as a top-level array of
    // `@`-prefixed refs (matching the frontmatter), `blocks` empty.
    let show = run(tmp.path(), &["--json", "show", "dp-end"]);
    assert_eq!(show.status.code(), Some(0), "{}", dump(&show));
    let v: serde_json::Value =
        serde_json::from_slice(&show.stdout).expect("show stdout should be JSON");
    assert!(
        v.get("blocked_by").is_some(),
        "blocked_by key must be present, not absent; {}",
        dump(&show)
    );
    assert_eq!(
        v["blocked_by"],
        serde_json::json!(["@bl-ocker"]),
        "{}",
        dump(&show)
    );
    assert_eq!(v["blocks"], serde_json::json!([]), "{}", dump(&show));

    // Blocker side: empty `blocked_by`, and the derived reverse `blocks`
    // view names the dependent.
    let show = run(tmp.path(), &["--json", "show", "bl-ocker"]);
    assert_eq!(show.status.code(), Some(0), "{}", dump(&show));
    let v: serde_json::Value =
        serde_json::from_slice(&show.stdout).expect("show stdout should be JSON");
    assert_eq!(v["blocked_by"], serde_json::json!([]), "{}", dump(&show));
    assert_eq!(
        v["blocks"],
        serde_json::json!(["@dp-end"]),
        "{}",
        dump(&show)
    );

    // The projection is the *only* copy on the wire: the raw nested
    // `extra.blocked_by` (which serde would otherwise emit) is stripped, so
    // consumers can't read a second, potentially divergent representation.
    let show = run(tmp.path(), &["--json", "show", "dp-end"]);
    let v: serde_json::Value =
        serde_json::from_slice(&show.stdout).expect("show stdout should be JSON");
    assert!(
        v.get("extra").and_then(|e| e.get("blocked_by")).is_none(),
        "extra.blocked_by must be stripped from show output; {}",
        dump(&show)
    );
}

/// The top-level `blocked_by` is a *canonical* projection, not a raw
/// frontmatter echo: hand-edited scalar/unsorted/unprefixed/duplicate refs
/// are coerced to a sorted, deduped, `@`-prefixed array. Guards the
/// normalization contract the fix promises (the `depend`-driven test above
/// only ever sees well-formed input).
#[test]
fn show_json_canonicalizes_raw_blocked_by_frontmatter() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &["new", "--type", "task", "--title", "Raw", "--slug", "ra-w"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    // Hand-edit the frontmatter to a messy shape: unsorted, mixed sigils,
    // a duplicate, and surrounding whitespace. `Issue::blocked_by()` must
    // normalize all of it.
    let item = tmp.path().join("issues/ra-w/item.md");
    let text = std::fs::read_to_string(&item).expect("read item.md");
    let text = text.replacen(
        "---\n",
        "---\nblocked_by: ['@zz-later', 'aa-first', '@aa-first', ' @mm-middle ']\n",
        1,
    );
    std::fs::write(&item, text).expect("write item.md");

    let show = run(tmp.path(), &["--json", "show", "ra-w"]);
    assert_eq!(show.status.code(), Some(0), "{}", dump(&show));
    let v: serde_json::Value =
        serde_json::from_slice(&show.stdout).expect("show stdout should be JSON");
    assert_eq!(
        v["blocked_by"],
        serde_json::json!(["@aa-first", "@mm-middle", "@zz-later"]),
        "sorted, deduped, @-prefixed; {}",
        dump(&show)
    );
}

/// Regression: `--json ls` must surface `blocked_by` as a top-level key
/// for every row, not bury it inside `extra` — the same contract
/// `--json show` already honours. Before the fix `jq '.[].blocked_by'`
/// over `ls` output read `null` even though the frontmatter (and the real
/// value under `.extra.blocked_by`) carried the link, silently fooling any
/// programmatic consumer that walked the listing. Pins both the populated
/// dependent row and that the raw `extra.blocked_by` copy is stripped, so
/// there is exactly one representation on the wire.
#[test]
fn ls_json_exposes_blocked_by_and_strips_extra_copy() {
    let tmp = fresh_repo();
    // Two issues: `dp-end` will be blocked by `bl-ocker`.
    for (title, slug) in [("Dependent", "dp-end"), ("Blocker", "bl-ocker")] {
        let out = run(
            tmp.path(),
            &["new", "--type", "task", "--title", title, "--slug", slug],
        );
        assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    }
    let out = run(
        tmp.path(),
        &[
            "--json",
            "depend",
            "add",
            "dp-end",
            "--blocked-by",
            "bl-ocker",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    let ls = run(tmp.path(), &["--json", "ls"]);
    assert_eq!(ls.status.code(), Some(0), "{}", dump(&ls));
    let rows: serde_json::Value =
        serde_json::from_slice(&ls.stdout).expect("ls stdout should be JSON");
    let rows = rows.as_array().expect("ls output is a JSON array");

    // Every row carries a top-level `blocked_by` (present, never `null`)
    // and never leaks the raw `extra.blocked_by`.
    for row in rows {
        assert!(
            row.get("blocked_by").map(|b| !b.is_null()).unwrap_or(false),
            "every ls row must carry a non-null top-level blocked_by; {}",
            dump(&ls)
        );
        assert!(
            row.get("extra").and_then(|e| e.get("blocked_by")).is_none(),
            "extra.blocked_by must be stripped from ls output; {}",
            dump(&ls)
        );
    }

    // The dependent row names its blocker as a canonical `@`-prefixed ref;
    // the blocker row has an empty list (not `null`).
    let find = |slug: &str| {
        rows.iter()
            .find(|r| r["slug"] == serde_json::json!(slug))
            .unwrap_or_else(|| panic!("row {slug} not found; {}", dump(&ls)))
    };
    assert_eq!(
        find("dp-end")["blocked_by"],
        serde_json::json!(["@bl-ocker"]),
        "{}",
        dump(&ls)
    );
    assert_eq!(
        find("bl-ocker")["blocked_by"],
        serde_json::json!([]),
        "{}",
        dump(&ls)
    );

    // `blocked_by` was the dependent's only unknown-frontmatter key, so
    // stripping it must drop the whole `extra` object (not leave `{}`) —
    // matching `Issue::extra`'s `skip_serializing_if` contract. Asserts the
    // empty-`extra` branch of `project_blocked_by`, which `extra.blocked_by
    // is None` alone would not (that also holds for `"extra": {}`).
    assert!(
        find("dp-end").get("extra").is_none(),
        "extra must be omitted once its only key (blocked_by) is stripped; {}",
        dump(&ls)
    );
}

/// `ls --json` applies the *canonical* `blocked_by` projection per row
/// (sorted, deduped, `@`-prefixed — coercing hand-edited scalar/unsorted/
/// unprefixed/duplicate frontmatter) exactly like `show --json`, and the
/// strip touches only `blocked_by`: an unrelated `extra` key must survive.
/// Guards both the normalization contract and the other-key branch of
/// `project_blocked_by` on the listing path (the `depend`-driven test above
/// only sees a single well-formed ref and a blocked_by-only `extra`).
#[test]
fn ls_json_canonicalizes_blocked_by_and_preserves_other_extra() {
    let tmp = fresh_repo();
    for (title, slug) in [("Messy array", "ma-ss"), ("Scalar", "sc-al")] {
        let out = run(
            tmp.path(),
            &["new", "--type", "task", "--title", title, "--slug", slug],
        );
        assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    }

    // `ma-ss`: a messy array (unsorted, mixed sigils, a duplicate,
    // whitespace) alongside an unrelated custom key that must be preserved.
    let item = tmp.path().join("issues/ma-ss/item.md");
    let text = std::fs::read_to_string(&item).expect("read ma-ss item.md");
    let text = text.replacen(
        "---\n",
        "---\ntriage: alice\nblocked_by: ['@zz-later', 'aa-first', '@aa-first', ' @mm-middle ']\n",
        1,
    );
    std::fs::write(&item, text).expect("write ma-ss item.md");

    // `sc-al`: `blocked_by` as a bare scalar string (hand-edited form
    // `Issue::blocked_by()` tolerates), which must still project to an array.
    let item = tmp.path().join("issues/sc-al/item.md");
    let text = std::fs::read_to_string(&item).expect("read sc-al item.md");
    let text = text.replacen("---\n", "---\nblocked_by: '@bl-ocker'\n", 1);
    std::fs::write(&item, text).expect("write sc-al item.md");

    let ls = run(tmp.path(), &["--json", "ls"]);
    assert_eq!(ls.status.code(), Some(0), "{}", dump(&ls));
    let rows: serde_json::Value =
        serde_json::from_slice(&ls.stdout).expect("ls stdout should be JSON");
    let rows = rows.as_array().expect("ls output is a JSON array");
    let find = |slug: &str| {
        rows.iter()
            .find(|r| r["slug"] == serde_json::json!(slug))
            .unwrap_or_else(|| panic!("row {slug} not found; {}", dump(&ls)))
    };

    // Canonicalized: sorted, deduped, `@`-prefixed.
    assert_eq!(
        find("ma-ss")["blocked_by"],
        serde_json::json!(["@aa-first", "@mm-middle", "@zz-later"]),
        "sorted, deduped, @-prefixed on ls; {}",
        dump(&ls)
    );
    // The unrelated `extra` key survives; only `blocked_by` is stripped.
    assert_eq!(
        find("ma-ss")["extra"]["triage"],
        serde_json::json!("alice"),
        "unrelated extra key must survive the blocked_by strip; {}",
        dump(&ls)
    );
    assert!(
        find("ma-ss")
            .get("extra")
            .and_then(|e| e.get("blocked_by"))
            .is_none(),
        "extra.blocked_by must be stripped even when other extra keys remain; {}",
        dump(&ls)
    );
    // A scalar `blocked_by` projects to a single-element array, not a string.
    assert_eq!(
        find("sc-al")["blocked_by"],
        serde_json::json!(["@bl-ocker"]),
        "scalar blocked_by must project to an array on ls; {}",
        dump(&ls)
    );
}

/// Regression: `--json search` shares `ls`'s serialization block, so it
/// must surface the same top-level `blocked_by` projection (not `null`
/// under `extra`). Symmetric to `ls_json_exposes_blocked_by_and_strips_extra_copy`.
#[test]
fn search_json_exposes_blocked_by_and_strips_extra_copy() {
    let tmp = fresh_repo();
    for (title, slug) in [("Dependent", "dp-end"), ("Blocker", "bl-ocker")] {
        let out = run(
            tmp.path(),
            &["new", "--type", "task", "--title", title, "--slug", slug],
        );
        assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    }
    let out = run(
        tmp.path(),
        &[
            "--json",
            "depend",
            "add",
            "dp-end",
            "--blocked-by",
            "bl-ocker",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    // A positive `status:` term keeps both open issues in scope.
    let search = run(tmp.path(), &["--json", "search", "status:open"]);
    assert_eq!(search.status.code(), Some(0), "{}", dump(&search));
    let rows: serde_json::Value =
        serde_json::from_slice(&search.stdout).expect("search stdout should be JSON");
    let rows = rows.as_array().expect("search output is a JSON array");

    for row in rows {
        assert!(
            row.get("blocked_by").map(|b| !b.is_null()).unwrap_or(false),
            "every search row must carry a non-null top-level blocked_by; {}",
            dump(&search)
        );
        assert!(
            row.get("extra").and_then(|e| e.get("blocked_by")).is_none(),
            "extra.blocked_by must be stripped from search output; {}",
            dump(&search)
        );
    }

    let find = |slug: &str| {
        rows.iter()
            .find(|r| r["slug"] == serde_json::json!(slug))
            .unwrap_or_else(|| panic!("row {slug} not found; {}", dump(&search)))
    };
    assert_eq!(
        find("dp-end")["blocked_by"],
        serde_json::json!(["@bl-ocker"]),
        "{}",
        dump(&search)
    );
    assert_eq!(
        find("bl-ocker")["blocked_by"],
        serde_json::json!([]),
        "{}",
        dump(&search)
    );
}
