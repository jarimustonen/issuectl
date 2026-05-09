//! Golden tests for `issuectl new`'s terminal output.
//!
//! These exist because the typed-error refactor (commit c272404)
//! promises that the human-readable error text from `cmd_new` stays
//! byte-identical to the pre-refactor output. The unit test in
//! `mutate.rs` (`do_new_error_to_anyhow_text_matches_per_variant`)
//! locks the `From<DoNewError> for anyhow::Error` conversion. That
//! does NOT cover:
//!
//! - `cmd_new`'s own `println!` formatting (success / JSON branches),
//! - anyhow's `Debug` rendering used by `main()`'s `Result<()>`
//!   (multi-line "Caused by:" chains),
//! - drift introduced anywhere upstream of `cmd_new` that still
//!   changes the final terminal output.
//!
//! So we exercise the actual binary via `CARGO_BIN_EXE_issuectl` and
//! lock the exit code + stderr.
//!
//! Convention note: this is the project's first integration test.
//! See `AGENTS.md` (`Tests` section) for when to use `tests/` vs.
//! inline `#[cfg(test)]` modules.

use std::process::{Command, Output};

use tempfile::TempDir;

fn fresh_repo() -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("issues")).expect("mkdir issues");
    tmp
}

fn run(root: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .expect("spawn issuectl")
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("stderr utf-8")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("stdout utf-8")
}

#[test]
fn new_success_prints_created_lines() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &["new", "--type", "bug", "--title", "Hello", "--slug", "ab-cd"],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let item_path = tmp.path().join("issues/ab-cd/item.md");
    let expected = format!("Created ab-cd: Hello\n  {}\n", item_path.display());
    assert_eq!(stdout(&out), expected);
    assert_eq!(stderr(&out), "");
}

#[test]
fn new_validation_owner_on_non_epic() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "new", "--type", "bug", "--title", "x", "--owner", "alice",
        ],
    );
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        "Error: --owner is only valid with --type epic\n"
    );
}

#[test]
fn new_conflict_slug_already_taken() {
    let tmp = fresh_repo();
    let taken = tmp.path().join("issues/taken-slug");
    std::fs::create_dir_all(&taken).unwrap();
    std::fs::write(taken.join("item.md"), "---\nstatus: open\n---\n").unwrap();

    let out = run(
        tmp.path(),
        &[
            "new", "--type", "bug", "--title", "x", "--slug", "taken-slug",
        ],
    );
    assert!(!out.status.success());
    let expected = format!("Error: target directory already exists: {}\n", taken.display());
    assert_eq!(stderr(&out), expected);
}

#[test]
fn new_schema_violation_missing_required_field() {
    let tmp = fresh_repo();
    std::fs::write(
        tmp.path().join("issues/.schema.yaml"),
        "version: 1\nfields:\n  team:\n    required: true\n",
    )
    .unwrap();

    let out = run(tmp.path(), &["new", "--type", "bug", "--title", "x"]);
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        "Error: schema: missing required field \"team\"\n"
    );
}

#[test]
fn new_schema_config_malformed_yaml() {
    let tmp = fresh_repo();
    let schema_path = tmp.path().join("issues/.schema.yaml");
    std::fs::write(&schema_path, "version: 1\nfields: : :\n").unwrap();

    let out = run(tmp.path(), &["new", "--type", "bug", "--title", "x"]);
    assert!(!out.status.success());
    let expected = format!(
        "Error: cannot parse {}: mapping values are not allowed in this context at line 2 column 9\n",
        schema_path.display()
    );
    assert_eq!(stderr(&out), expected);
}

#[cfg(unix)]
#[test]
fn new_io_failure_chmod_readonly() {
    // Mirrors `mutate::tests::new_issue_io_failure_returns_typed_error`:
    // pre-seed the default schema (so we get past `ensure_default_written`),
    // then chmod 0o500 on `issues/` so `fs::create_dir(<root>/issues/<slug>)`
    // fails with EACCES. RAII guard restores perms on every exit so the
    // tempdir cleanup never inherits a 0o500 directory.
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
    // Seed default schema by running a successful `new` first; this
    // matches the inline test's call to `schema::ensure_default_written`
    // but goes through the binary so the harness stays end-to-end.
    let seed = run(
        tmp.path(),
        &["new", "--type", "bug", "--title", "seed", "--slug", "seed-issue"],
    );
    assert!(seed.status.success(), "seed failed: {}", stderr(&seed));

    let issues_dir = tmp.path().join("issues");
    let original = std::fs::metadata(&issues_dir).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_mode(0o500);
    std::fs::set_permissions(&issues_dir, readonly).unwrap();
    let _guard = PermGuard {
        path: issues_dir.clone(),
        original: original.clone(),
    };

    // chmod 0o500 has no effect for uid 0; skip the assertion when a
    // probe write still succeeds (CI containers occasionally run as
    // root). Mirrors the uid-0 escape hatch in the inline test.
    let probe = issues_dir.join(".io-probe");
    let chmod_enforced = std::fs::write(&probe, b"x").is_err();
    let _ = std::fs::remove_file(&probe);
    if !chmod_enforced {
        return;
    }

    let target = issues_dir.join("io-fail-slug");
    let out = run(
        tmp.path(),
        &[
            "new", "--type", "bug", "--title", "x", "--slug", "io-fail-slug",
        ],
    );
    assert!(!out.status.success());
    let expected = format!(
        "Error: cannot create {}\n\nCaused by:\n    Permission denied (os error 13)\n",
        target.display()
    );
    assert_eq!(stderr(&out), expected);
}
