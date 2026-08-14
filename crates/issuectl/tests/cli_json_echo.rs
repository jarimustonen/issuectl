//! Black-box coverage for the action-verb `--json` field echo
//! (issue action-verb-json-echo-mutation). The mutate verbs `update`,
//! `label`, and `close` must echo the RESULTING (post-mutation) value of
//! the field they changed in their `--json` result, so a caller can
//! confirm the write from that one call without a second `show`
//! round-trip. Before the fix, `.priority` / `.labels` / `.status` came
//! back `null`/absent even though the write had landed.

use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

fn fresh_repo() -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("issues")).unwrap();
    tmp
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .current_dir(root)
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .expect("spawn issuectl")
}

fn json(out: &Output) -> serde_json::Value {
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    serde_json::from_slice(&out.stdout).expect("json stdout")
}

fn new_issue(root: &Path, slug: &str) {
    let out = run(
        root,
        &[
            "--json", "new", "--slug", slug, "--title", slug, "--type", "task",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "new {slug}: {out:?}");
}

#[test]
fn update_priority_json_echoes_new_priority() {
    let tmp = fresh_repo();
    let r = tmp.path();
    new_issue(r, "prio-echo");

    let v = json(&run(
        r,
        &["--json", "update", "prio-echo", "--priority", "high"],
    ));
    assert_eq!(
        v["priority"],
        serde_json::json!("high"),
        "update --json must echo the resulting priority"
    );
    // The existing back-compat keys are still present.
    assert_eq!(v["slug"], serde_json::json!("prio-echo"));
    assert!(v.get("version").is_some());
}

#[test]
fn update_status_json_echoes_new_status() {
    let tmp = fresh_repo();
    let r = tmp.path();
    new_issue(r, "status-echo");

    let v = json(&run(
        r,
        &["--json", "update", "status-echo", "--status", "in-progress"],
    ));
    assert_eq!(
        v["status"],
        serde_json::json!("in-progress"),
        "update --json must echo the resulting status"
    );
}

#[test]
fn label_add_and_remove_json_echo_resulting_labels() {
    let tmp = fresh_repo();
    let r = tmp.path();
    new_issue(r, "label-echo");

    let added = json(&run(r, &["--json", "label", "label-echo", "add", "infra"]));
    assert_eq!(
        added["labels"],
        serde_json::json!(["infra"]),
        "label add --json must echo the resulting labels array"
    );

    let added2 = json(&run(
        r,
        &["--json", "label", "label-echo", "add", "backend"],
    ));
    assert_eq!(
        added2["labels"],
        serde_json::json!(["infra", "backend"]),
        "labels array reflects the full post-mutation set"
    );

    let removed = json(&run(
        r,
        &["--json", "label", "label-echo", "remove", "infra"],
    ));
    assert_eq!(
        removed["labels"],
        serde_json::json!(["backend"]),
        "label remove --json must echo the labels that remain"
    );

    // Removing the last label echoes `null` — the same shape `show` emits
    // for an issue that carries no labels, so both parse identically.
    let emptied = json(&run(
        r,
        &["--json", "label", "label-echo", "remove", "backend"],
    ));
    assert_eq!(
        emptied["labels"],
        serde_json::Value::Null,
        "removing the final label echoes null, mirroring `show`"
    );
}

#[test]
fn set_priority_json_echoes_new_priority() {
    // `set` shares the `finish_mutation` echo path with `label`; assert the
    // contract holds for it too, since a refactor could split it off.
    let tmp = fresh_repo();
    let r = tmp.path();
    new_issue(r, "set-echo");

    let v = json(&run(r, &["--json", "set", "set-echo", "priority", "high"]));
    assert_eq!(
        v["priority"],
        serde_json::json!("high"),
        "set --json must echo the resulting field value"
    );
    // The full core-field set is present on every mutating verb.
    assert!(v.get("status").is_some(), "status key present");
    assert!(v.get("labels").is_some(), "labels key present");
}

#[test]
fn close_json_echoes_resulting_status() {
    let tmp = fresh_repo();
    let r = tmp.path();
    new_issue(r, "close-echo");

    // Give it a non-default priority so we can prove `close` echoes the
    // full core-field set, not just `status`.
    run(r, &["--json", "update", "close-echo", "--priority", "high"]);

    // A `task` completes as `done` (per the default transition rules);
    // `fixed` is bug-only. Let `close` pick the default closing status and
    // assert the result echoes whichever status actually landed.
    let v = json(&run(r, &["--json", "close", "close-echo"]));
    assert_eq!(
        v["moved_to_closed"],
        serde_json::json!(true),
        "close moved the issue to closed"
    );
    assert_eq!(
        v["status"],
        serde_json::json!("done"),
        "close --json must echo the resulting closing status"
    );
    // `close` echoes the same core-field set as the other mutating verbs,
    // so a generic result parser needs no per-verb schema.
    assert_eq!(
        v["priority"],
        serde_json::json!("high"),
        "close --json echoes the (unchanged) priority too"
    );
    assert!(
        v.get("labels").is_some(),
        "close --json echoes the labels key too"
    );
}
