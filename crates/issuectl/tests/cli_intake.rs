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

/// Hand-write a legacy label-encoded issue (the ad-hoc Telegram path's
/// on-disk shape) that the intake filer would refuse to produce. Injects
/// the schema-required `priority`/`created` fields.
fn write_legacy(root: &std::path::Path, slug: &str, frontmatter: &str) {
    let dir = root.join("issues").join(slug);
    std::fs::create_dir_all(&dir).expect("mkdir slug");
    let body = format!(
        "---\npriority: normal\ncreated: 2026-01-01\ntype: bug\nstatus: open\n{frontmatter}---\n\n# {slug}\n\nlegacy body\n"
    );
    std::fs::write(dir.join("item.md"), body).expect("write item.md");
}

#[test]
fn migrate_dry_run_then_apply_is_idempotent() {
    let tmp = fresh_repo();
    write_legacy(
        tmp.path(),
        "legacy-bug-one",
        "labels: [needs-triage, via:telegram]\n",
    );

    // Dry-run: reports the plan, writes nothing.
    let dry = run(tmp.path(), &["--json", "intake", "migrate"]);
    assert_eq!(dry.status.code(), Some(0), "{}", dump(&dry));
    let dv = json_stdout(&dry);
    assert_eq!(dv["applied"], false);
    assert_eq!(dv["summary"]["migrated"], 1);
    assert_eq!(dv["actions"][0]["action"], "migrate");
    assert_eq!(dv["actions"][0]["status_change"]["to"], "untriaged");
    assert_eq!(dv["actions"][0]["applied"], false);
    // The queue still shows it as a legacy form (nothing migrated yet).
    let q = json_stdout(&run(tmp.path(), &["--json", "intake", "queue"]));
    assert_eq!(q["legacy_pending"], 1, "still legacy pre-apply");

    // Apply: writes the change.
    let apply = run(tmp.path(), &["--json", "intake", "migrate", "--apply"]);
    assert_eq!(apply.status.code(), Some(0), "{}", dump(&apply));
    let av = json_stdout(&apply);
    assert_eq!(av["applied"], true);
    assert_eq!(av["actions"][0]["applied"], true);

    // Now a first-class untriaged item with provenance set, no legacy flag.
    let q2 = json_stdout(&run(tmp.path(), &["--json", "intake", "queue"]));
    assert!(q2.get("legacy_pending").is_none(), "no legacy left: {q2}");
    let items = q2["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["slug"], "legacy-bug-one");
    assert_eq!(items[0]["status"], "untriaged");
    assert_eq!(items[0]["provenance"], "telegram");
    assert_eq!(items[0]["legacy"], false);

    // Second apply is a no-op.
    let again = json_stdout(&run(
        tmp.path(),
        &["--json", "intake", "migrate", "--apply"],
    ));
    assert_eq!(again["summary"]["total"], 0, "idempotent: {again}");
}

#[test]
fn migrate_reports_conflict_as_skip_without_writing() {
    let tmp = fresh_repo();
    write_legacy(
        tmp.path(),
        "ambiguous-legacy",
        "labels: [needs-triage, deferred]\n",
    );
    let out = run(tmp.path(), &["--json", "intake", "migrate", "--apply"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v = json_stdout(&out);
    assert_eq!(v["summary"]["skipped"], 1);
    assert_eq!(v["actions"][0]["action"], "skip");
    assert!(v["actions"][0]["conflict"].is_string());
    assert_eq!(v["actions"][0]["applied"], false);
    // Unchanged on disk: still open with BOTH labels intact (a conflict
    // touches nothing).
    let body = std::fs::read_to_string(tmp.path().join("issues/ambiguous-legacy/item.md")).unwrap();
    assert!(body.contains("status: open"), "{body}");
    assert!(body.contains("needs-triage"), "{body}");
    assert!(body.contains("deferred"), "{body}");
}

#[test]
fn migrate_reports_write_failure_and_exits_nonzero_but_migrates_the_rest() {
    // A repo whose schema constrains `provenance` to an enum excluding
    // `telegram`: the via:telegram write fails, but a sibling needs-triage
    // item still migrates. The command exits 1 (failed write ≠ conflict).
    let tmp = fresh_repo();
    std::fs::write(
        tmp.path().join("issues/.schema.yaml"),
        "version: 1\nfields:\n  provenance:\n    enum: [email]\n",
    )
    .unwrap();
    write_legacy(tmp.path(), "good-legacy", "labels: [needs-triage]\n");
    write_legacy(tmp.path(), "bad-legacy", "labels: [via:telegram]\n");

    let out = run(tmp.path(), &["--json", "intake", "migrate", "--apply"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "failed write → exit 1: {}",
        dump(&out)
    );
    let v = json_stdout(&out);
    assert_eq!(v["summary"]["failed"], 1);
    assert_eq!(v["summary"]["migrated"], 1);
    // The good one committed; the bad one untouched (retryable).
    let good = json_stdout(&run(tmp.path(), &["--json", "show", "good-legacy"]));
    assert_eq!(good["status"], "untriaged");
    let bad = json_stdout(&run(tmp.path(), &["--json", "show", "bad-legacy"]));
    assert_eq!(bad["status"], "open");
}

#[test]
fn queue_surfaces_legacy_form_with_flag_and_nudge() {
    let tmp = fresh_repo();
    // A first-class untriaged item and a legacy open+needs-triage item.
    file_bug(tmp.path(), "modern-one");
    write_legacy(tmp.path(), "legacy-one", "labels: [needs-triage]\n");

    let out = run(tmp.path(), &["--json", "intake", "queue"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let v = json_stdout(&out);
    assert_eq!(v["legacy_pending"], 1);
    assert!(v["migration_hint"].as_str().unwrap().contains("migrate"));
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "both surfaced: {}", dump(&out));
    let legacy: Vec<_> = items.iter().filter(|i| i["legacy"] == true).collect();
    assert_eq!(legacy.len(), 1);
    assert_eq!(legacy[0]["slug"], "legacy-one");
    assert_eq!(legacy[0]["status"], "open");

    // The --state deferred view folds only legacy *deferred* forms; there
    // are none here (this legacy item is a needs-triage form), so it stays
    // empty.
    let deferred = run(
        tmp.path(),
        &["--json", "intake", "queue", "--state", "deferred"],
    );
    let dv = json_stdout(&deferred);
    assert!(
        dv.get("legacy_pending").is_none(),
        "no legacy deferred here"
    );
    assert!(dv["items"].as_array().unwrap().is_empty());
}

#[test]
fn queue_human_mode_flags_legacy_row_and_prints_nudge() {
    // §6 requires the legacy surfacing in the HUMAN output path too, not
    // just `--json`: the legacy row carries a `[legacy]` flag and a trailing
    // `Note:` line nudges the reader to run the migration. The JSON tests
    // above pin the machine contract; this pins the text a developer reads.
    let tmp = fresh_repo();
    file_bug(tmp.path(), "modern-one");
    write_legacy(tmp.path(), "legacy-one", "labels: [needs-triage]\n");

    let out = run(tmp.path(), &["intake", "queue"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The legacy row is flagged inline...
    let legacy_row = stdout
        .lines()
        .find(|l| l.contains("legacy-one"))
        .unwrap_or_else(|| panic!("legacy row missing: {}", dump(&out)));
    assert!(
        legacy_row.contains("[legacy]"),
        "legacy row not flagged: {}",
        dump(&out)
    );
    // ...and the modern row is NOT.
    let modern_row = stdout
        .lines()
        .find(|l| l.contains("modern-one"))
        .unwrap_or_else(|| panic!("modern row missing: {}", dump(&out)));
    assert!(
        !modern_row.contains("[legacy]"),
        "modern row wrongly flagged: {}",
        dump(&out)
    );
    // The migration nudge names the count, the legacy label, and the command.
    assert!(
        stdout.contains("Note: 1 legacy item(s)"),
        "no nudge line: {}",
        dump(&out)
    );
    assert!(stdout.contains("needs-triage"), "{}", dump(&out));
    assert!(stdout.contains("issuectl intake migrate"), "{}", dump(&out));
}

#[test]
fn queue_state_deferred_surfaces_legacy_deferred_form() {
    // §6 also migrates `open + deferred` → `deferred`. Those items must be
    // visible in the deferred view before migration, else they are silently
    // abandoned (invisible in both the default and the deferred queue).
    let tmp = fresh_repo();
    write_legacy(tmp.path(), "parked-legacy", "labels: [deferred]\n");

    // The default (untriaged) queue does NOT surface a deferred-only form.
    let def_default = json_stdout(&run(tmp.path(), &["--json", "intake", "queue"]));
    assert!(def_default.get("legacy_pending").is_none());
    assert!(def_default["items"].as_array().unwrap().is_empty());

    // The deferred view does.
    let out = run(
        tmp.path(),
        &["--json", "intake", "queue", "--state", "deferred"],
    );
    let v = json_stdout(&out);
    assert_eq!(v["legacy_pending"], 1, "{}", dump(&out));
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["slug"], "parked-legacy");
    assert_eq!(items[0]["legacy"], true);
    assert_eq!(items[0]["status"], "open");
}

#[test]
fn queue_provenance_filter_surfaces_unmigrated_telegram_items() {
    // `--provenance telegram` must still find a legacy item that encodes
    // provenance via the `via:telegram` label (no `provenance` field yet),
    // otherwise the transition filter hides exactly the population being
    // migrated.
    let tmp = fresh_repo();
    write_legacy(
        tmp.path(),
        "tg-legacy",
        "labels: [needs-triage, via:telegram]\n",
    );
    // A non-telegram legacy item that must be filtered OUT.
    write_legacy(tmp.path(), "other-legacy", "labels: [needs-triage]\n");

    let out = run(
        tmp.path(),
        &["--json", "intake", "queue", "--provenance", "telegram"],
    );
    let v = json_stdout(&out);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "only the telegram item: {}", dump(&out));
    assert_eq!(items[0]["slug"], "tg-legacy");
    assert_eq!(items[0]["legacy"], true);
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
