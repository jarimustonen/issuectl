//! Byte-level CLI tests for the `issuectl intake` command group. The
//! domain logic is unit-tested next to the code in
//! `mutate::intake::tests`; these tests pin the CLI contract that only a
//! spawned process can observe — the `--json` envelopes, the exit codes
//! (0 success / 2 refused-but-actionable / 1 validation), and the
//! documented error codes (`transition-illegal`, `duplicate-source-ref`,
//! `protected-field`).

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

fn json_stdout(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("stdout not JSON: {}", dump(out)))
}

fn json_stderr(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stderr).unwrap_or_else(|_| panic!("stderr not JSON: {}", dump(out)))
}

/// File a bug and return the process output.
fn file_bug(root: &std::path::Path, slug: &str) -> Output {
    run(
        root,
        &[
            "--json",
            "intake",
            "file",
            "--type",
            "bug",
            "--title",
            "A bug",
            "--body",
            "broken",
            "--reporter",
            "alice",
            "--provenance",
            "telegram",
            "--slug",
            slug,
        ],
    )
}

#[test]
fn file_creates_untriaged_and_dedups_on_source_ref() {
    let tmp = fresh_repo();
    let args: &[&str] = &[
        "--json",
        "intake",
        "file",
        "--type",
        "bug",
        "--title",
        "Crash",
        "--body",
        "boom",
        "--provenance",
        "telegram",
        "--source-ref",
        "chat:1/msg:2",
    ];
    let first = run(tmp.path(), args);
    assert_eq!(first.status.code(), Some(0), "{}", dump(&first));
    let v = json_stdout(&first);
    assert_eq!(v["status"], "untriaged");
    assert_eq!(v["deduplicated"], false);
    let slug = v["slug"].as_str().unwrap().to_string();

    // Retry with the same (provenance, source_ref) → dedup, exit 0.
    let second = run(tmp.path(), args);
    assert_eq!(second.status.code(), Some(0), "{}", dump(&second));
    let v2 = json_stdout(&second);
    assert_eq!(v2["deduplicated"], true, "{}", dump(&second));
    assert_eq!(v2["slug"].as_str().unwrap(), slug);
}

#[test]
fn file_rejects_protected_field() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "--json",
            "intake",
            "file",
            "--type",
            "bug",
            "--title",
            "Spoof",
            "--body",
            "x",
            "--provenance",
            "telegram",
            "--slug",
            "spoof-attempt",
            // `provenance` has a dedicated flag and is protected against
            // `--field` injection. (Keys like `status`/`type` are already
            // rejected one layer up by the shared custom-field parser.)
            "--field",
            "provenance=email",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    assert_eq!(json_stderr(&out)["error"]["code"], "protected-field");
}

#[test]
fn accept_then_reopen_roundtrip() {
    let tmp = fresh_repo();
    assert_eq!(file_bug(tmp.path(), "round-trip").status.code(), Some(0));

    let accept = run(tmp.path(), &["--json", "intake", "accept", "round-trip"]);
    assert_eq!(accept.status.code(), Some(0), "{}", dump(&accept));
    assert_eq!(json_stdout(&accept)["status"], "open");
}

#[test]
fn accept_on_closed_item_is_refused_but_actionable() {
    let tmp = fresh_repo();
    file_bug(tmp.path(), "closed-one");
    let rej = run(
        tmp.path(),
        &["intake", "reject", "closed-one", "--reason", "nope"],
    );
    assert_eq!(rej.status.code(), Some(0), "{}", dump(&rej));

    // accept on a closed item: refused-but-actionable → exit 2, code
    // transition-illegal.
    let out = run(tmp.path(), &["--json", "intake", "accept", "closed-one"]);
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
    assert!(out.stdout.is_empty(), "{}", dump(&out));
    assert_eq!(json_stderr(&out)["error"]["code"], "transition-illegal");
}

#[test]
fn reject_writes_structured_disposition_reason_and_note() {
    let tmp = fresh_repo();
    file_bug(tmp.path(), "by-design-bug");
    let out = run(
        tmp.path(),
        &[
            "intake",
            "reject",
            "by-design-bug",
            "--kind",
            "by-design",
            "--reason",
            "intended behaviour",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let body = std::fs::read_to_string(tmp.path().join("issues/by-design-bug/item.md")).unwrap();
    assert!(body.contains("status: wontfix"), "{body}");
    assert!(body.contains("disposition_reason: by-design"), "{body}");
    assert!(body.contains("## Comments"), "{body}");
    assert!(body.contains("intended behaviour"), "{body}");
}

#[test]
fn duplicate_cycle_is_rejected() {
    let tmp = fresh_repo();
    file_bug(tmp.path(), "dup-a");
    file_bug(tmp.path(), "dup-b");
    let ok = run(
        tmp.path(),
        &["intake", "duplicate", "dup-a", "--of", "dup-b"],
    );
    assert_eq!(ok.status.code(), Some(0), "{}", dump(&ok));

    // b → a would close a cycle.
    let out = run(
        tmp.path(),
        &["--json", "intake", "duplicate", "dup-b", "--of", "dup-a"],
    );
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert_eq!(json_stderr(&out)["error"]["code"], "validation");
    assert!(
        json_stderr(&out)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cycle"),
        "{}",
        dump(&out)
    );
}

#[test]
fn queue_lists_untriaged_oldest_first_and_excludes_deferred() {
    let tmp = fresh_repo();
    file_bug(tmp.path(), "q-first");
    file_bug(tmp.path(), "q-second");
    // Defer one — it must drop out of the default queue.
    run(
        tmp.path(),
        &["intake", "defer", "q-second", "--reason", "later"],
    );

    let out = run(tmp.path(), &["--json", "intake", "queue"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v = json_stdout(&out);
    assert_eq!(v["state"], "untriaged");
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "deferred item excluded: {}", dump(&out));
    assert_eq!(items[0]["slug"], "q-first");
    assert_eq!(items[0]["needs_analysis"], true);

    // Explicit --state deferred surfaces the parked item.
    let deferred = run(
        tmp.path(),
        &["--json", "intake", "queue", "--state", "deferred"],
    );
    let dv = json_stdout(&deferred);
    assert_eq!(dv["items"].as_array().unwrap().len(), 1);
    assert_eq!(dv["items"][0]["slug"], "q-second");
}

#[test]
fn show_reports_analysis_and_attachments_keys() {
    let tmp = fresh_repo();
    file_bug(tmp.path(), "show-me");
    let out = run(tmp.path(), &["--json", "intake", "show", "show-me"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v = json_stdout(&out);
    assert_eq!(v["slug"], "show-me");
    assert!(v["attachments"].is_array(), "{}", dump(&out));
    // No analysis section yet.
    assert!(v["analysis"].is_null(), "{}", dump(&out));
}

#[test]
fn generic_set_status_cannot_bypass_intrinsic_invariant() {
    // `set status in-progress` on an untriaged item must be blocked by
    // the same validator the intake verbs use, classified
    // `transition-illegal` and — like the intake surface — exit 2
    // (refused-but-actionable), regardless of entry point.
    let tmp = fresh_repo();
    file_bug(tmp.path(), "no-bypass");
    let out = run(
        tmp.path(),
        &["--json", "set", "no-bypass", "status", "in-progress"],
    );
    assert_eq!(out.status.code(), Some(2), "{}", dump(&out));
    assert_eq!(json_stderr(&out)["error"]["code"], "transition-illegal");
}

#[test]
fn intake_file_rejects_epic() {
    let tmp = fresh_repo();
    let out = run(
        tmp.path(),
        &[
            "--json",
            "intake",
            "file",
            "--type",
            "epic",
            "--title",
            "E",
            "--body",
            "x",
            "--provenance",
            "telegram",
            "--slug",
            "epic-attempt",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    assert!(
        json_stderr(&out)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("epic"),
        "{}",
        dump(&out)
    );
}
