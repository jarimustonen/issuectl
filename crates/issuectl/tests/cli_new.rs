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
        "{{\n  \"dir\": \"{}\",\n  \"path\": \"{}\",\n  \"slug\": \"ab-cd\",\n  \"title\": \"Hello\"\n}}\n",
        dir.display(),
        item_path.display(),
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
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

/// A bubble-up anyhow error under `--json` is rendered with the shared
/// envelope and the generic `command-failed` code. `update` on a slug
/// that does not exist bubbles a plain anyhow error from the command
/// body (as opposed to the explicit `not-found` classification the read
/// paths emit).
#[test]
fn json_error_contract_wraps_bubbled_errors() {
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
    assert_eq!(v["error"]["code"], "command-failed", "{}", dump(&out));
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not found"),
        "{}",
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
