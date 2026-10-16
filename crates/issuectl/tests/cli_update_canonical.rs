//! Black-box parity coverage for ADR 0004's canonical `update` forms.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use tempfile::TempDir;

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .current_dir(root)
        .arg("--root")
        .arg(root)
        .arg("--json")
        .args(args)
        .output()
        .expect("spawn issuectl")
}

fn run_stdin(root: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env_remove("RUST_BACKTRACE")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .current_dir(root)
        .arg("--root")
        .arg(root)
        .arg("--json")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn issuectl");
    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait for issuectl")
}

fn pair() -> (TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    std::fs::create_dir_all(a.join("issues")).unwrap();
    std::fs::create_dir_all(b.join("issues")).unwrap();
    (tmp, a, b)
}

fn create(root: &Path, slug: &str) {
    let out = run(
        root,
        &["create", "--type", "task", "--title", slug, "--slug", slug],
    );
    assert!(out.status.success(), "{out:?}");
}

fn normalized_json(out: &Output, root: &Path) -> Value {
    assert!(out.status.success(), "{out:?}");
    let mut value: Value = serde_json::from_slice(&out.stdout).expect("JSON stdout");
    normalize_strings(&mut value, &root.to_string_lossy());
    value
}

fn normalize_strings(value: &mut Value, root: &str) {
    match value {
        Value::String(s) => *s = s.replace(root, "<ROOT>"),
        Value::Array(values) => {
            for value in values {
                normalize_strings(value, root);
            }
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                normalize_strings(value, root);
            }
        }
        _ => {}
    }
}

fn assert_same_success(a_root: &Path, a_args: &[&str], b_root: &Path, b_args: &[&str]) {
    let a = run(a_root, a_args);
    let b = run(b_root, b_args);
    assert_eq!(
        normalized_json(&a, a_root),
        normalized_json(&b, b_root),
        "a={a:?}\nb={b:?}"
    );
}

fn assert_close_success(
    close_root: &Path,
    close_args: &[&str],
    update_root: &Path,
    update_args: &[&str],
) {
    let close = run(close_root, close_args);
    let update = run(update_root, update_args);
    let close_json = normalized_json(&close, close_root);
    let mut update_json = normalized_json(&update, update_root);
    // `close --json` historically omits this update-only reopen marker.
    // The 0.15 preparation slice preserves both existing envelopes; the
    // 0.16 alias adapter owns their eventual convergence.
    update_json["data"]
        .as_object_mut()
        .unwrap()
        .remove("moved_to_open");
    assert_eq!(
        close_json, update_json,
        "close={close:?}\nupdate={update:?}"
    );
}

fn assert_same_failure(a_root: &Path, a_args: &[&str], b_root: &Path, b_args: &[&str]) {
    let a = run(a_root, a_args);
    let b = run(b_root, b_args);
    assert_eq!(a.status.code(), Some(1), "{a:?}");
    assert_eq!(b.status.code(), Some(1), "{b:?}");
    assert!(a.stdout.is_empty(), "{a:?}");
    assert!(b.stdout.is_empty(), "{b:?}");
    let mut a_json: Value = serde_json::from_slice(&a.stderr).expect("a JSON stderr");
    let mut b_json: Value = serde_json::from_slice(&b.stderr).expect("b JSON stderr");
    normalize_strings(&mut a_json, &a_root.to_string_lossy());
    normalize_strings(&mut b_json, &b_root.to_string_lossy());
    assert_eq!(a_json, b_json, "a={a:?}\nb={b:?}");
}

fn item(root: &Path, slug: &str) -> String {
    std::fs::read_to_string(root.join("issues").join(slug).join("item.md")).unwrap()
}

#[test]
fn patch_file_matches_apply_for_mutation_output_and_conflict() {
    let (tmp, apply_root, update_root) = pair();
    create(&apply_root, "patch-target");
    create(&update_root, "patch-target");
    let shown = run(&apply_root, &["show", "patch-target"]);
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    let version = shown["data"]["version"].as_str().unwrap();
    let patch = tmp.path().join("patch.yaml");
    std::fs::write(
        &patch,
        format!("slug: patch-target\nexpected_version: {version}\npriority: high\n"),
    )
    .unwrap();
    let patch_s = patch.to_string_lossy();

    let before = item(&apply_root, "patch-target");
    assert_same_success(
        &apply_root,
        &["apply", &patch_s, "--dry-run"],
        &update_root,
        &["update", "--patch-file", &patch_s, "--dry-run"],
    );
    assert_eq!(item(&apply_root, "patch-target"), before);
    assert_eq!(item(&update_root, "patch-target"), before);

    assert_same_success(
        &apply_root,
        &["apply", &patch_s],
        &update_root,
        &["update", "--patch-file", &patch_s],
    );
    assert_eq!(
        item(&apply_root, "patch-target"),
        item(&update_root, "patch-target")
    );

    // Reusing the now-stale patch must produce the same compare-and-swap error.
    assert_same_failure(
        &apply_root,
        &["apply", &patch_s],
        &update_root,
        &["update", "--patch-file", &patch_s],
    );
}

#[test]
fn stdin_patch_matches_apply_and_update_for_dry_run_and_write() {
    let (_tmp, apply_root, update_root) = pair();
    create(&apply_root, "stdin-target");
    create(&update_root, "stdin-target");
    let shown = run(&apply_root, &["show", "stdin-target"]);
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    let version = shown["data"]["version"].as_str().unwrap();
    let patch =
        format!(r#"{{"slug":"stdin-target","expected_version":"{version}","priority":"high"}}"#);

    let before = item(&apply_root, "stdin-target");
    let apply_dry = run_stdin(&apply_root, &["apply", "-", "--dry-run"], &patch);
    let update_dry = run_stdin(
        &update_root,
        &["update", "--patch-file", "-", "--dry-run"],
        &patch,
    );
    assert_eq!(
        normalized_json(&apply_dry, &apply_root),
        normalized_json(&update_dry, &update_root)
    );
    assert_eq!(item(&apply_root, "stdin-target"), before);
    assert_eq!(item(&update_root, "stdin-target"), before);

    let apply = run_stdin(&apply_root, &["apply", "-"], &patch);
    let update = run_stdin(&update_root, &["update", "--patch-file", "-"], &patch);
    assert_eq!(
        normalized_json(&apply, &apply_root),
        normalized_json(&update, &update_root)
    );
    assert_eq!(
        item(&apply_root, "stdin-target"),
        item(&update_root, "stdin-target")
    );
}

#[test]
fn malformed_and_unsupported_patch_inputs_have_parity_and_clear_errors() {
    let (_tmp, apply_root, update_root) = pair();
    let apply = run_stdin(&apply_root, &["apply", "-"], "{not valid");
    let update = run_stdin(&update_root, &["update", "--patch-file", "-"], "{not valid");
    assert_same_error_bytes(&apply, &update);
    let malformed: Value = serde_json::from_slice(&apply.stderr).unwrap();
    let malformed_message = malformed["error"]["message"].as_str().unwrap();
    assert!(malformed_message.contains("YAML or JSON"));
    assert!(malformed_message.contains("from stdin"));

    let empty = run_stdin(&apply_root, &["apply", "-"], "");
    assert_eq!(empty.status.code(), Some(1), "{empty:?}");
    let empty_message = String::from_utf8(empty.stderr).unwrap();
    assert!(
        empty_message.contains("from stdin is empty"),
        "{empty_message}"
    );

    let inline = r#"{"slug":"some-issue","url":"https://example.test/a.b"}"#;
    let apply = run(&apply_root, &["apply", inline]);
    let update = run(&update_root, &["update", "--patch-file", inline]);
    assert_same_error_bytes(&apply, &update);
    let unsupported = String::from_utf8(apply.stderr).unwrap();
    for accepted in [
        "patch file path",
        "`-`",
        "`./-`",
        "inline patch input is not accepted",
    ] {
        assert!(
            unsupported.contains(accepted),
            "missing {accepted:?}: {unsupported}"
        );
    }

    let apply = run(&apply_root, &["apply", "missing-patch"]);
    let update = run(&update_root, &["update", "--patch-file", "missing-patch"]);
    assert_same_error_bytes(&apply, &update);
    let message = String::from_utf8(apply.stderr).unwrap();
    assert!(
        message.contains("cannot read patch file missing-patch"),
        "{message}"
    );
}

fn assert_same_error_bytes(a: &Output, b: &Output) {
    assert_eq!(a.status.code(), Some(1), "{a:?}");
    assert_eq!(b.status.code(), Some(1), "{b:?}");
    assert!(a.stdout.is_empty(), "{a:?}");
    assert!(b.stdout.is_empty(), "{b:?}");
    assert_eq!(a.stderr, b.stderr, "a={a:?}\nb={b:?}");
}

#[test]
fn literal_dash_file_is_addressable_as_dot_slash_dash() {
    let (_tmp, apply_root, update_root) = pair();
    create(&apply_root, "dash-file-target");
    create(&update_root, "dash-file-target");
    let shown = run(&apply_root, &["show", "dash-file-target"]);
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    let version = shown["data"]["version"].as_str().unwrap();
    let patch = format!(
        r#"{{"slug":"dash-file-target","expected_version":"{version}","priority":"high"}}"#
    );
    std::fs::write(apply_root.join("-"), &patch).unwrap();
    std::fs::write(update_root.join("-"), &patch).unwrap();
    assert_same_success(
        &apply_root,
        &["apply", "./-"],
        &update_root,
        &["update", "--patch-file", "./-"],
    );
    assert!(item(&apply_root, "dash-file-target").contains("priority: high"));
    assert!(item(&update_root, "dash-file-target").contains("priority: high"));

    // Even while the literal file exists, bare `-` remains stdin rather than
    // falling back to that file. Give the two sources different priorities so
    // the persisted result proves which one won.
    let shown = run(&apply_root, &["show", "dash-file-target"]);
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    let version = shown["data"]["version"].as_str().unwrap();
    let file_patch =
        format!(r#"{{"slug":"dash-file-target","expected_version":"{version}","priority":"low"}}"#);
    std::fs::write(apply_root.join("-"), &file_patch).unwrap();
    std::fs::write(update_root.join("-"), &file_patch).unwrap();
    let stdin_patch = format!(
        r#"{{"slug":"dash-file-target","expected_version":"{version}","priority":"normal"}}"#
    );
    let apply = run_stdin(&apply_root, &["apply", "-"], &stdin_patch);
    let update = run_stdin(&update_root, &["update", "--patch-file", "-"], &stdin_patch);
    assert_eq!(
        normalized_json(&apply, &apply_root),
        normalized_json(&update, &update_root)
    );
    assert!(item(&apply_root, "dash-file-target").contains("priority: normal"));
    assert!(item(&update_root, "dash-file-target").contains("priority: normal"));
}

#[test]
fn query_matches_bulk_for_apply_and_dry_run() {
    let (_tmp, bulk_root, update_root) = pair();
    for slug in ["first-task", "second-task"] {
        create(&bulk_root, slug);
        create(&update_root, slug);
    }
    assert_same_success(
        &bulk_root,
        &[
            "bulk",
            "status:open",
            "--set",
            "priority=high",
            "--add-label",
            "triaged",
        ],
        &update_root,
        &[
            "update",
            "--query",
            "status:open",
            "--priority",
            "high",
            "--add-label",
            "triaged",
        ],
    );
    for slug in ["first-task", "second-task"] {
        assert_eq!(item(&bulk_root, slug), item(&update_root, slug));
    }

    let (_tmp, bulk_root, update_root) = pair();
    create(&bulk_root, "dry-target");
    create(&update_root, "dry-target");
    let before = item(&bulk_root, "dry-target");
    assert_same_success(
        &bulk_root,
        &["bulk", "status:open", "--add-label", "planned", "--dry-run"],
        &update_root,
        &[
            "update",
            "--query",
            "status:open",
            "--add-label",
            "planned",
            "--dry-run",
        ],
    );
    assert_eq!(item(&bulk_root, "dry-target"), before);
    assert_eq!(item(&update_root, "dry-target"), before);
}

#[test]
fn existing_update_flags_match_the_folded_commands() {
    let cases: &[(&[&str], &[&str])] = &[
        (
            &["label", "subject-task", "add", "triaged"],
            &["update", "subject-task", "--add-label", "triaged"],
        ),
        (
            &["assign", "subject-task", "alice"],
            &["update", "subject-task", "--assignee", "alice"],
        ),
        (
            &[
                "depend",
                "add",
                "subject-task",
                "--blocked-by",
                "blocker-task",
            ],
            &["update", "subject-task", "--add-blocked-by", "blocker-task"],
        ),
        (
            &["set", "subject-task", "priority", "high"],
            &["update", "subject-task", "--priority", "high"],
        ),
        (
            &["close", "subject-task", "--status", "done"],
            &["update", "subject-task", "--status", "done"],
        ),
    ];

    for (folded, canonical) in cases {
        let (_tmp, folded_root, update_root) = pair();
        for slug in ["subject-task", "blocker-task"] {
            create(&folded_root, slug);
            create(&update_root, slug);
        }
        if folded[0] == "close" {
            assert_close_success(&folded_root, folded, &update_root, canonical);
            let closed = item(&folded_root, "subject-task");
            assert!(closed.contains("closed:"), "{closed}");
        } else {
            assert_same_success(&folded_root, folded, &update_root, canonical);
        }
        assert_eq!(
            item(&folded_root, "subject-task"),
            item(&update_root, "subject-task"),
            "folded={folded:?} canonical={canonical:?}"
        );
    }

    let removal_cases: &[(&[&str], &[&str], &[&str])] = &[
        (
            &["update", "subject-task", "--add-label", "triaged"],
            &["label", "subject-task", "remove", "triaged"],
            &["update", "subject-task", "--remove-label", "triaged"],
        ),
        (
            &["assign", "subject-task", "alice"],
            &["assign", "subject-task", "--clear"],
            &["update", "subject-task", "--no-assignee"],
        ),
        (
            &[
                "depend",
                "add",
                "subject-task",
                "--blocked-by",
                "blocker-task",
            ],
            &[
                "depend",
                "remove",
                "subject-task",
                "--blocked-by",
                "blocker-task",
            ],
            &[
                "update",
                "subject-task",
                "--remove-blocked-by",
                "blocker-task",
            ],
        ),
        (
            &["set", "subject-task", "team", "payments"],
            &["set", "subject-task", "team", "--clear"],
            &["update", "subject-task", "--clear-field", "team"],
        ),
    ];
    for (prepare, folded, canonical) in removal_cases {
        let (_tmp, folded_root, update_root) = pair();
        for slug in ["subject-task", "blocker-task"] {
            create(&folded_root, slug);
            create(&update_root, slug);
        }
        assert!(run(&folded_root, prepare).status.success());
        assert!(run(&update_root, prepare).status.success());
        assert_same_success(&folded_root, folded, &update_root, canonical);
        assert_eq!(
            item(&folded_root, "subject-task"),
            item(&update_root, "subject-task")
        );
    }

    let (_tmp, set_root, update_root) = pair();
    create(&set_root, "subject-task");
    create(&update_root, "subject-task");
    assert_same_success(
        &set_root,
        &["set", "subject-task", "team", "payments"],
        &update_root,
        &["update", "subject-task", "--field", "team=payments"],
    );
    assert_eq!(
        item(&set_root, "subject-task"),
        item(&update_root, "subject-task")
    );
}

#[test]
fn folded_forms_and_query_surface_the_same_invalid_input_errors() {
    let stale = "sha256:v1:0000000000000000000000000000000000000000000000000000000000000000";
    let cases: &[(Vec<&str>, Vec<&str>)] = &[
        (
            vec![
                "label",
                "subject-task",
                "add",
                "triaged",
                "--expected-version",
                stale,
            ],
            vec![
                "update",
                "subject-task",
                "--add-label",
                "triaged",
                "--expected-version",
                stale,
            ],
        ),
        (
            vec![
                "assign",
                "subject-task",
                "alice",
                "--expected-version",
                stale,
            ],
            vec![
                "update",
                "subject-task",
                "--assignee",
                "alice",
                "--expected-version",
                stale,
            ],
        ),
        (
            vec![
                "depend",
                "add",
                "subject-task",
                "--blocked-by",
                "blocker-task",
                "--expected-version",
                stale,
            ],
            vec![
                "update",
                "subject-task",
                "--add-blocked-by",
                "blocker-task",
                "--expected-version",
                stale,
            ],
        ),
        (
            vec![
                "set",
                "subject-task",
                "priority",
                "high",
                "--expected-version",
                stale,
            ],
            vec![
                "update",
                "subject-task",
                "--priority",
                "high",
                "--expected-version",
                stale,
            ],
        ),
        (
            vec![
                "close",
                "subject-task",
                "--status",
                "done",
                "--expected-version",
                stale,
            ],
            vec![
                "update",
                "subject-task",
                "--status",
                "done",
                "--expected-version",
                stale,
            ],
        ),
    ];
    for (folded, canonical) in cases {
        let (_tmp, folded_root, update_root) = pair();
        for slug in ["subject-task", "blocker-task"] {
            create(&folded_root, slug);
            create(&update_root, slug);
        }
        assert_same_failure(&folded_root, folded, &update_root, canonical);
    }

    let (_tmp, bulk_root, update_root) = pair();
    create(&bulk_root, "subject-task");
    create(&update_root, "subject-task");
    assert_same_failure(
        &bulk_root,
        &["bulk", "status:", "--add-label", "triaged"],
        &update_root,
        &["update", "--query", "status:", "--add-label", "triaged"],
    );
}

#[test]
fn update_status_reopens_and_unarchives_an_archived_issue() {
    let (_tmp, root, _) = pair();
    create(&root, "archived-task");
    assert!(run(&root, &["close", "archived-task", "--status", "done"])
        .status
        .success());
    assert!(run(&root, &["archive", "--older-than", "0d"])
        .status
        .success());
    assert!(!root.join("issues/archived-task").exists());

    let reopened = run(&root, &["update", "archived-task", "--status", "open"]);
    let data = normalized_json(&reopened, &root);
    assert_eq!(data["data"]["moved_to_open"], true);
    assert!(root.join("issues/archived-task/item.md").is_file());
    assert!(item(&root, "archived-task").contains("status: open"));
}

#[test]
fn canonical_targets_and_patch_fields_are_mutually_exclusive() {
    let (_tmp, root, _) = pair();
    create(&root, "subject-task");
    let both_targets = run(
        &root,
        &[
            "update",
            "subject-task",
            "--query",
            "status:open",
            "--priority",
            "high",
        ],
    );
    assert_eq!(both_targets.status.code(), Some(1), "{both_targets:?}");
    assert!(both_targets.stdout.is_empty(), "{both_targets:?}");
    let error: Value = serde_json::from_slice(&both_targets.stderr).unwrap();
    assert_eq!(error["error"]["code"], "usage-error");
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("cannot be used with '--query"));

    let patch = root.join("patch.yaml");
    std::fs::write(&patch, "slug: subject-task\npriority: high\n").unwrap();
    let patch_s = patch.to_string_lossy();
    let mixed = run(
        &root,
        &["update", "--patch-file", &patch_s, "--priority", "high"],
    );
    assert_eq!(mixed.status.code(), Some(1), "{mixed:?}");
    let stderr = String::from_utf8_lossy(&mixed.stderr);
    assert!(
        stderr.contains("--patch-file") && stderr.contains("--priority"),
        "{stderr}"
    );

    let slug_dry_run = run(
        &root,
        &["update", "subject-task", "--priority", "high", "--dry-run"],
    );
    assert_eq!(slug_dry_run.status.code(), Some(1), "{slug_dry_run:?}");
    assert!(slug_dry_run.stdout.is_empty(), "{slug_dry_run:?}");
    let error: Value = serde_json::from_slice(&slug_dry_run.stderr).unwrap();
    assert_eq!(error["error"]["code"], "usage-error");
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--dry-run"));
}

fn seed_old_summary(root: &Path, slug: &str) {
    let dir = root.join("issues").join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("item.md"), "---\ntype: task\nstatus: open\npriority: normal\ncreated: 2020-01-01\nupdated: 2020-01-02\n---\n\n# Summary fixture\n").unwrap();
}

fn success_json(out: Output) -> Value {
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert!(value["warnings"].is_array());
    value
}

fn error_json(out: Output) -> Value {
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    let value: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert!(value["error"]["code"].is_string());
    value
}

#[test]
fn preserve_updated_cli_single_patch_query_and_rejections() {
    let (_tmp, root, _) = pair();
    seed_old_summary(&root, "summary-one");
    seed_old_summary(&root, "summary-two");
    let shown = success_json(run(&root, &["show", "summary-one"]));
    let v0 = shown["data"]["version"].as_str().unwrap();
    let out = success_json(run(
        &root,
        &[
            "update",
            "summary-one",
            "--field",
            "status_summary=Waiting",
            "--preserve-updated",
            "--expected-version",
            v0,
        ],
    ));
    assert_ne!(out["data"]["version"], v0);
    assert!(item(&root, "summary-one").contains("updated: 2020-01-02"));
    let before = item(&root, "summary-one");
    error_json(run(
        &root,
        &[
            "update",
            "summary-one",
            "--field",
            "status_summary=Later",
            "--preserve-updated",
            "--expected-version",
            v0,
        ],
    ));
    for extra in [
        vec!["--status", "open"],
        vec!["--title", "Retitle"],
        vec!["--body", "# Body"],
        vec!["--priority", "high"],
        vec!["--add-label", "triaged"],
        vec!["--lane", "maintenance"],
        vec!["--field", "preparation=ready"],
    ] {
        let mut args = vec![
            "update",
            "summary-one",
            "--preserve-updated",
            "--field",
            "status_summary=Later",
        ];
        args.extend(extra);
        error_json(run(&root, &args));
        assert_eq!(item(&root, "summary-one"), before);
    }
    error_json(run(&root, &["update", "summary-one", "--preserve-updated"]));
    error_json(run(
        &root,
        &["update", "--patch-file", "-", "--preserve-updated"],
    ));

    let shown = success_json(run(&root, &["show", "summary-one"]));
    let version = shown["data"]["version"].as_str().unwrap();
    let patch = format!("slug: summary-one\nexpected_version: {version}\npreserve_updated: true\ncustom_fields:\n  status_summary: Refreshed\n");
    let dry = success_json(run_stdin(
        &root,
        &["update", "--patch-file", "-", "--dry-run"],
        &patch,
    ));
    assert_eq!(dry["data"]["dry_run"], true);
    assert!(!dry["data"]["diff"].as_str().unwrap().contains("-updated:"));
    assert_eq!(item(&root, "summary-one"), before);
    error_json(run_stdin(
        &root,
        &["update", "--patch-file", "-"],
        &format!("{patch}status: in-progress\n"),
    ));
    assert_eq!(item(&root, "summary-one"), before);
    success_json(run_stdin(&root, &["update", "--patch-file", "-"], &patch));
    assert!(item(&root, "summary-one").contains("updated: 2020-01-02"));
    error_json(run_stdin(&root, &["update", "--patch-file", "-"], &patch));

    let before_one = item(&root, "summary-one");
    let before_two = item(&root, "summary-two");
    let args = [
        "update",
        "--query",
        "status:open",
        "--preserve-updated",
        "--field",
        "status_summary=Batch",
    ];
    let mut dry_args = args.to_vec();
    dry_args.push("--dry-run");
    assert_eq!(success_json(run(&root, &dry_args))["data"]["count"], 2);
    assert_eq!(item(&root, "summary-one"), before_one);
    assert_eq!(item(&root, "summary-two"), before_two);
    let mut mixed = args.to_vec();
    mixed.extend(["--add-blocked-by", "other-issue"]);
    error_json(run(&root, &mixed));
    assert_eq!(item(&root, "summary-one"), before_one);
    assert_eq!(item(&root, "summary-two"), before_two);
    assert_eq!(success_json(run(&root, &args))["data"]["count"], 2);
    for slug in ["summary-one", "summary-two"] {
        assert!(item(&root, slug).contains("updated: 2020-01-02"));
    }
    success_json(run(
        &root,
        &[
            "update",
            "summary-one",
            "--clear-field",
            "status_summary",
            "--preserve-updated",
        ],
    ));
    assert!(!item(&root, "summary-one").contains("status_summary:"));
    assert!(item(&root, "summary-one").contains("updated: 2020-01-02"));
    success_json(run(
        &root,
        &["update", "summary-one", "--status", "in-progress"],
    ));
    assert!(!item(&root, "summary-one").contains("updated: 2020-01-02"));
}
