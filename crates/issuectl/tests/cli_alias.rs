//! Integration tests for the CLI convenience-alias surface added in the
//! "CLI alias nippu" bundle: the `create` → `new` subcommand alias, the
//! `--body` → `--description` arg alias on `new`, the `assign <slug>
//! <user>` convenience subcommand (routing through the `set --assignee`
//! path, plus `--clear`), and the `body <slug>` unrecognized-subcommand
//! routing hint.
//!
//! These live in `tests/` (not inline) because they exercise the real
//! argv → dispatch → filesystem/exit-code surface end to end; the inline
//! `#[cfg(test)]` tests in `src/main.rs` cover the pure parse-shape and
//! `subcommand_error_hint` logic. See `AGENTS.md` (`Tests`).

use std::process::{Command, Output};

use tempfile::TempDir;

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

fn stdout_json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout not JSON: {e}\n{}", dump(out)))
}

/// Create an issue and return its slug. Uses a fixed slug for
/// determinism.
fn seed_issue(root: &std::path::Path, slug: &str) {
    let out = run(
        root,
        &["new", "--type", "task", "--title", "Seed", "--slug", slug],
    );
    assert_eq!(out.status.code(), Some(0), "seed failed: {}", dump(&out));
}

/// `create` is a visible alias for `new`: it writes exactly what `new`
/// would and reports success.
#[test]
fn create_alias_resolves_to_new() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "create", "--type", "bug", "--title", "Hello", "--slug", "ab-cd",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    assert!(
        tmp.path().join("issues/ab-cd/item.md").is_file(),
        "expected item.md to exist: {}",
        dump(&out)
    );
}

/// `--body` is an alias for `--description` on `new`: the value lands in
/// the created issue's body.
#[test]
fn body_flag_alias_populates_description() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "new",
            "--type",
            "task",
            "--title",
            "Desc",
            "--slug",
            "body-alias",
            "--body",
            "Body via alias flag.",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let body = std::fs::read_to_string(tmp.path().join("issues/body-alias/item.md")).unwrap();
    assert!(
        body.contains("Body via alias flag."),
        "body should contain the --body text, got:\n{body}"
    );
}

/// `assign <slug> <user>` sets `assignee`, equivalent to `set <slug>
/// assignee <user>`.
#[test]
fn assign_sets_assignee() {
    let tmp = fresh_repo();
    seed_issue(tmp.path(), "assign-me");
    let out = run(tmp.path(), &["assign", "assign-me", "alice"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    let show = run(tmp.path(), &["--json", "show", "assign-me"]);
    assert_eq!(stdout_json(&show)["assignee"], "alice", "{}", dump(&show));
}

/// `assign <slug> --clear` unassigns, mirroring `set --clear`.
#[test]
fn assign_clear_unassigns() {
    let tmp = fresh_repo();
    seed_issue(tmp.path(), "clear-me");
    let set = run(tmp.path(), &["assign", "clear-me", "bob"]);
    assert_eq!(set.status.code(), Some(0), "{}", dump(&set));

    let clear = run(tmp.path(), &["assign", "clear-me", "--clear"]);
    assert_eq!(clear.status.code(), Some(0), "{}", dump(&clear));

    let show = run(tmp.path(), &["--json", "show", "clear-me"]);
    assert_eq!(
        stdout_json(&show)["assignee"],
        serde_json::Value::Null,
        "{}",
        dump(&show)
    );
}

/// `assign --json <user> --expected-version <v>` succeeds and writes the
/// assignee — proving the wrapper mirrors `set`'s full write contract, not
/// only its rejection path.
#[test]
fn assign_json_success_with_expected_version() {
    let tmp = fresh_repo();
    seed_issue(tmp.path(), "ev-ok");
    let show = run(tmp.path(), &["--json", "show", "ev-ok"]);
    let version = stdout_json(&show)["version"]
        .as_str()
        .expect("version string")
        .to_string();

    let out = run(
        tmp.path(),
        &[
            "--json",
            "assign",
            "ev-ok",
            "dave",
            "--expected-version",
            &version,
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    let after = run(tmp.path(), &["--json", "show", "ev-ok"]);
    assert_eq!(stdout_json(&after)["assignee"], "dave", "{}", dump(&after));
}

/// `assign` and `set <slug> assignee <user>` are behaviourally identical:
/// run in two fresh repos, the resulting on-disk assignee matches.
#[test]
fn assign_matches_set_assignee_path() {
    let via_assign = fresh_repo();
    seed_issue(via_assign.path(), "parity-check");
    assert_eq!(
        run(via_assign.path(), &["assign", "parity-check", "erin"])
            .status
            .code(),
        Some(0)
    );

    let via_set = fresh_repo();
    seed_issue(via_set.path(), "parity-check");
    assert_eq!(
        run(via_set.path(), &["set", "parity-check", "assignee", "erin"])
            .status
            .code(),
        Some(0)
    );

    let a = run(via_assign.path(), &["--json", "show", "parity-check"]);
    let s = run(via_set.path(), &["--json", "show", "parity-check"]);
    assert_eq!(
        stdout_json(&a)["assignee"],
        stdout_json(&s)["assignee"],
        "assign and set assignee should yield the same assignee"
    );
    assert_eq!(stdout_json(&a)["assignee"], "erin");
}

/// `assign --dry-run` writes nothing (the assignee stays unset), matching
/// `set`'s plan-only contract.
#[test]
fn assign_dry_run_writes_nothing() {
    let tmp = fresh_repo();
    seed_issue(tmp.path(), "dry-run-it");
    let out = run(tmp.path(), &["assign", "dry-run-it", "frank", "--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    let show = run(tmp.path(), &["--json", "show", "dry-run-it"]);
    assert_eq!(
        stdout_json(&show)["assignee"],
        serde_json::Value::Null,
        "dry-run must not persist the assignee: {}",
        dump(&show)
    );
}

/// `assign` honours the same `--json` optimistic-concurrency contract as
/// `set`: without `--expected-version` it is refused.
#[test]
fn assign_json_requires_expected_version() {
    let tmp = fresh_repo();
    seed_issue(tmp.path(), "needs-ev");
    let out = run(tmp.path(), &["--json", "assign", "needs-ev", "carol"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
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

/// A user is required unless `--clear` is given (parse-time error, exit 2).
#[test]
fn assign_requires_user_or_clear() {
    let tmp = fresh_repo();
    let out = run(tmp.path(), &["assign", "some-slug"]);
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
}

/// `body <slug>` (a bare slug where a `body` sub-subcommand is expected)
/// exits 2 and prints a routing tip pointing at `body set <slug>`.
#[test]
fn body_slug_prints_routing_hint() {
    let tmp = fresh_repo();
    let out = run(tmp.path(), &["body", "some-slug"]);
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("body set some-slug"),
        "stderr should hint `body set some-slug`, got:\n{stderr}"
    );
}

/// Under `--json`, the `body <slug>` usage error is wrapped in the shared
/// envelope (exit 1, `code:"usage-error"`) with the tip folded into the
/// message.
#[test]
fn body_slug_hint_json_envelope() {
    let tmp = fresh_repo();
    let out = run(tmp.path(), &["--json", "body", "some-slug"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "usage-error", "{}", dump(&out));
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("body set some-slug"),
        "{}",
        dump(&out)
    );
}

/// A top-level near-miss for the `create` alias prints a routing tip
/// naming the canonical `new` — plain path exits 2 with the tip on stderr,
/// and `--json` folds it into the shared usage-error envelope (exit 1).
#[test]
fn near_miss_alias_hint_plain_and_json() {
    let tmp = fresh_repo();

    let plain = run(tmp.path(), &["creat"]);
    assert_eq!(plain.status.code(), Some(2), "{}", dump(&plain));
    let stderr = String::from_utf8_lossy(&plain.stderr);
    assert!(
        stderr.contains("alias for `new`"),
        "plain stderr should route to `new`, got:\n{stderr}"
    );

    let json = run(tmp.path(), &["--json", "creat"]);
    assert_eq!(json.status.code(), Some(1), "{}", dump(&json));
    assert!(json.stdout.is_empty(), "{}", dump(&json));
    let v: serde_json::Value = serde_json::from_slice(&json.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "usage-error", "{}", dump(&json));
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("alias for `new`"),
        "{}",
        dump(&json)
    );
}
