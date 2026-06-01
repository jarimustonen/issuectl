//! Issue @doctor-fix-noop regression: `doctor --fix` must not silently
//! no-op alias coercion or `.issuectl/AGENTS.md` regen just because some
//! other issue has an unmergeable `## Notes` / `## Comments` body.
//!
//! Fixture mirrors `/tmp/doctor-repro3/` from the bug report:
//!   - `.schema.yaml` with the built-in alias table
//!   - one legacy `status: closed` issue (alias coerces to `done`)
//!   - one issue with both `## Notes` and `## Comments` (manual merge)
//!   - one drifted `.issuectl/AGENTS.md` managed block

use std::process::{Command, Output};

use tempfile::TempDir;

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

fn build_repro_fixture() -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("issues")).unwrap();
    std::fs::create_dir_all(root.join(".issuectl")).unwrap();

    std::fs::write(
        root.join("issues/.schema.yaml"),
        "version: 1\nfields:\n  status:\n    enum: [open, in-progress, testing, done, fixed, wontfix, duplicate, cannot-reproduce, obsolete]\n",
    )
    .unwrap();

    let legacy = root.join("issues/legacy-closed-bug");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(
        legacy.join("item.md"),
        "---\ncreated: 2026-01-01\nupdated: 2026-01-01\ntype: bug\nstatus: closed\npriority: normal\nclosed: 2026-01-02\n---\n\n# legacy closed bug\n",
    )
    .unwrap();

    let conflict = root.join("issues/notes-conflict-bug");
    std::fs::create_dir_all(&conflict).unwrap();
    std::fs::write(
        conflict.join("item.md"),
        "---\ncreated: 2026-01-01\nupdated: 2026-01-01\ntype: bug\nstatus: open\npriority: normal\n---\n\n# notes conflict bug\n\n## Notes\nold\n\n## Comments\nnew\n",
    )
    .unwrap();

    std::fs::write(
        root.join(".issuectl/AGENTS.md"),
        "<!-- issuectl-managed:start -->\n# stale\n<!-- issuectl-managed:end -->\n",
    )
    .unwrap();

    tmp
}

/// Architecture fix (success criterion A): the `notes-conflict-bug`
/// finding no longer aborts the apply pass; alias coercion lands on
/// `legacy-closed-bug` and the `.issuectl/AGENTS.md` schema-derived
/// block is regenerated in a single invocation.
#[test]
fn doctor_fix_applies_alias_and_agents_md_despite_notes_conflict() {
    let tmp = build_repro_fixture();
    let root = tmp.path();
    let legacy_item = root.join("issues/legacy-closed-bug/item.md");
    let agents_md = root.join(".issuectl/AGENTS.md");
    let original_agents = std::fs::read_to_string(&agents_md).unwrap();

    let out = run(root, &["doctor", "--fix"]);

    // Alias coercion (status: closed → done) MUST have landed.
    let legacy_after = std::fs::read_to_string(&legacy_item).unwrap();
    assert!(
        legacy_after.contains("status: done"),
        "alias coercion must land. legacy item:\n{legacy_after}\n\n{}",
        dump(&out)
    );

    // AGENTS.md MUST have been regenerated (file changed).
    let agents_after = std::fs::read_to_string(&agents_md).unwrap();
    assert_ne!(
        agents_after,
        original_agents,
        "AGENTS.md schema-derived block must be regenerated. {}",
        dump(&out)
    );

    // notes-conflict-bug is reported, but only as a manual-attention
    // finding — not a preflight bail. Stdout must NOT carry the
    // "cannot safely apply --fix" banner.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("cannot safely apply --fix"),
        "no preflight refusal banner expected. {}",
        dump(&out)
    );
    // The summary line should NOT claim a clean "Applied." anymore —
    // it must be the partial form reflecting the manual-attention
    // notes-conflict.
    assert!(
        stdout.contains("Partial — auto-fixes ran") || stdout.contains("need manual attention"),
        "expected partial-summary line, got: {stdout}"
    );

    // Exit code is non-zero (manual leftovers remain) per the new
    // contract, but the human output is coherent and the writes
    // landed. The non-zero exit drives the agent loop forward
    // without re-applying the same fixes.
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
}

/// JSON envelope contract (success criterion C): a `--fix --json`
/// run that exits non-zero must emit
/// `{"error":{"code","message","details"}}` on stderr (stdout empty),
/// with `details.apply_outcome` carrying the structured outcome.
#[test]
fn doctor_fix_json_emits_error_envelope_on_partial_exit() {
    let tmp = build_repro_fixture();
    let root = tmp.path();

    let out = run(root, &["--json", "doctor", "--fix"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.trim().is_empty(),
        "stdout must be empty on --json error path, got: {stdout}"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let v: serde_json::Value = serde_json::from_str(&stderr)
        .unwrap_or_else(|e| panic!("stderr must be JSON envelope, got: {stderr} ({e})"));
    let err = &v["error"];
    assert_eq!(
        err["code"].as_str(),
        Some("doctor-partial"),
        "expected doctor-partial code, got: {v}"
    );
    assert!(err["message"].is_string());
    // Nested `details` carries the full result so consumers can read
    // what actually got applied.
    let details = &err["details"];
    let outcome = &details["apply_outcome"];
    assert_eq!(
        outcome["stop_phase"].as_str(),
        Some("ok"),
        "stop_phase must be ok (the apply ran to completion)"
    );
    // notes-conflict-bug surfaces in notes_conflicts_at_apply.
    let nca = outcome["notes_conflicts_at_apply"].as_array().unwrap();
    assert!(
        nca.iter().any(|v| v.as_str() == Some("notes-conflict-bug")),
        "notes_conflicts_at_apply must include the conflicted slug, got: {nca:?}"
    );
    // Alias coercion is recorded on the outcome.
    let aliases = outcome["alias_coercions_applied"].as_array().unwrap();
    assert!(
        aliases
            .iter()
            .any(|v| v["slug"].as_str() == Some("legacy-closed-bug")
                && v["field"].as_str() == Some("status")),
        "alias coercion must be recorded, got: {aliases:?}"
    );
    assert_eq!(
        outcome["agents_md_regenerated"],
        serde_json::Value::Bool(true)
    );
    // Tighten the message assertion: it must call out the specific
    // manual-merge action (not the generic "unfixable" message).
    assert!(
        err["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("manual"),
        "message must mention the manual merge, got: {}",
        err["message"]
    );
}

/// Issue @doctor-fix-noop: the conflict can also live under
/// `issues/{open,closed}/<slug>/` (pre-flat-layout). The post-flat-layout
/// rescan must feed it into `notes_conflicts_at_apply`, AND NN-rename
/// for an unrelated numbered-legacy dir MUST still run.
#[test]
fn doctor_fix_handles_legacy_folder_notes_conflict() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("issues/open/foo-bar")).unwrap();
    std::fs::write(
        root.join("issues/open/foo-bar/item.md"),
        "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-01-01\n---\n# T\n\n## Notes\nold\n\n## Comments\nnew\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("issues/closed/3-old")).unwrap();
    std::fs::write(
        root.join("issues/closed/3-old/item.md"),
        "---\nnumber: 3\ntype: bug\nstatus: open\npriority: normal\n---\n# Old\n",
    )
    .unwrap();

    let out = run(root, &["--json", "doctor", "--fix"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let v: serde_json::Value = serde_json::from_str(&stderr)
        .unwrap_or_else(|e| panic!("expected JSON envelope, got: {stderr} ({e})"));
    let outcome = &v["error"]["details"]["apply_outcome"];

    // foo-bar must surface in notes_conflicts_at_apply (via the
    // post-flat-layout rescan).
    let nca = outcome["notes_conflicts_at_apply"].as_array().unwrap();
    assert!(
        nca.iter().any(|s| s.as_str() == Some("foo-bar")),
        "post-flat-layout notes conflict must surface, got: {nca:?}"
    );
    // The unrelated numbered-legacy dir must have been renamed.
    let migrated = outcome["legacy_dirs_migrated"].as_array().unwrap();
    assert!(
        !migrated.is_empty(),
        "NN-rename must run despite an unrelated notes conflict, got: {migrated:?}"
    );
    // foo-bar still landed at flat layout (the flat-layout migration
    // ran independently of the notes conflict).
    assert!(root.join("issues/foo-bar/item.md").is_file());
}

/// Issue @doctor-fix-noop: read-only `--json doctor` on an unhealthy
/// repo MUST keep the historical contract — full result on stdout,
/// exit 1. The envelope-on-stderr contract is scoped to `--fix --json`
/// only; widening it would silently break `issuectl --json doctor | jq …`.
#[test]
fn doctor_readonly_json_still_emits_result_on_stdout() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("issues")).unwrap();
    // Duplicate-slug fixture → critical finding → exit 1.
    std::fs::create_dir_all(root.join("issues/open/quiet-brave-otter")).unwrap();
    std::fs::write(
        root.join("issues/open/quiet-brave-otter/item.md"),
        "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("issues/closed/quiet-brave-otter")).unwrap();
    std::fs::write(
        root.join("issues/closed/quiet-brave-otter/item.md"),
        "---\ntype: bug\nstatus: closed\npriority: normal\nclosed: 2026-01-01\n---\n# T\n",
    )
    .unwrap();

    let out = run(root, &["--json", "doctor"]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "read-only --json must keep emitting on stdout. {}",
        dump(&out)
    );
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout must be JSON, got: {stdout} ({e})"));
    // Sanity: the duplicate finding is in the payload, in the
    // historical (non-envelope) shape.
    assert!(
        !v["both_open_and_closed"].as_array().unwrap().is_empty(),
        "expected duplicate-slug finding in result, got: {v}"
    );
}

/// Issue @doctor-fix-noop: clean `--fix --json` run still emits a
/// result object on stdout (not an envelope) with exit 0. Pin the
/// success-path contract so a future widening of the envelope to
/// success cases is caught.
#[test]
fn doctor_fix_json_clean_run_stays_on_stdout() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("issues")).unwrap();

    let out = run(root, &["--json", "doctor", "--fix"]);
    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout must be JSON, got: {stdout} ({e})"));
    assert!(v.get("apply_outcome").is_some(), "expected apply_outcome");
    assert!(
        v.get("error").is_none(),
        "clean run must NOT include error envelope, got: {v}"
    );
}
