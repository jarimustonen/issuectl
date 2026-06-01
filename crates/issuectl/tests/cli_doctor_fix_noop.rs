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
}
