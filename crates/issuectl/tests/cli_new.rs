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

/// `low` is the lowest priority value: accepted on `new`, `ls -p`, and
/// `update`, while the default stays `normal`. Guards the widened
/// `PRIORITIES` set against a regression back to two-valued.
#[test]
fn priority_low_is_accepted_end_to_end() {
    let tmp = fresh_repo();

    // `new --priority low` succeeds and persists the field.
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
    let item = std::fs::read_to_string(tmp.path().join("issues/lo-one/item.md"))
        .expect("item.md should exist");
    assert!(
        item.contains("priority: low"),
        "expected `priority: low` in frontmatter; got:\n{item}"
    );

    // `ls -p low` filters to the low-priority issue.
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

    // `update --priority low` is accepted on an existing issue.
    let out = run(
        tmp.path(),
        &[
            "new", "--type", "task", "--title", "Bump me", "--slug", "bu-mp",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let show = run(tmp.path(), &["--json", "show", "bu-mp"]);
    let version = serde_json::from_slice::<serde_json::Value>(&show.stdout)
        .expect("show stdout should be JSON")["version"]
        .as_str()
        .expect("version string")
        .to_string();
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
    let item = std::fs::read_to_string(tmp.path().join("issues/bu-mp/item.md"))
        .expect("item.md should exist");
    assert!(
        item.contains("priority: low"),
        "expected `priority: low` after update; got:\n{item}"
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
/// envelope and the generic `command-failed` code.
#[test]
fn json_error_contract_wraps_bubbled_errors() {
    let tmp = fresh_repo();
    // `update --json` without `--expected-version` is a D4=B violation
    // that bubbles up as an anyhow error from the command body.
    let out = run(
        tmp.path(),
        &["--json", "update", "ab-cd", "--status", "in-progress"],
    );
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr should be JSON");
    assert_eq!(v["error"]["code"], "command-failed", "{}", dump(&out));
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--expected-version"),
        "{}",
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
