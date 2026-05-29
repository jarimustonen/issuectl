//! End-to-end tests for `issuectl export` / `issuectl import`. These
//! drive the real binary so they exercise the CLI wiring (arg parsing,
//! folder defaults, `--json` reporting) on top of the pure-function unit
//! tests in `issuectl-core::transfer`. GitHub import is not covered here
//! because it shells out to `gh`; its parsing half is unit-tested in core.

use std::process::{Command, Output};

use tempfile::TempDir;

fn fresh_repo() -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("issues")).expect("mkdir issues");
    tmp
}

fn run(root: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_issuectl"))
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

fn stdout(out: &Output) -> String {
    assert!(out.status.success(), "{}", dump(out));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn export_csv_lists_open_issues_with_header() {
    let repo = fresh_repo();
    run(repo.path(), &["new", "--type", "bug", "--title", "First bug"]);
    let out = stdout(&run(repo.path(), &["export", "csv"]));
    let mut lines = out.lines();
    assert_eq!(
        lines.next().unwrap(),
        "slug,type,status,priority,assignee,owner,reporter,epic,labels,title,created,updated,closed"
    );
    let row = lines.next().expect("one data row");
    assert!(row.contains(",bug,open,normal,"), "row was {row}");
    assert!(row.ends_with(",First bug,") || row.contains(",First bug,"));
}

#[test]
fn export_json_round_trips_through_import_into_fresh_repo() {
    let src = fresh_repo();
    run(src.path(), &["new", "--type", "bug", "--title", "Login loops", "--assignee", "bob"]);
    run(src.path(), &["new", "--type", "feature", "--title", "Dark mode"]);
    let json = stdout(&run(src.path(), &["export", "json"]));

    let dst = fresh_repo();
    let import_file = dst.path().join("import.json");
    std::fs::write(&import_file, &json).unwrap();
    let out = run(
        dst.path(),
        &["--json", "import", "json", import_file.to_str().unwrap()],
    );
    let report = stdout(&out);
    assert!(report.contains("\"created_count\": 2"), "{report}");
    assert!(report.contains("\"failed_count\": 0"), "{report}");

    // The imported issues exist with their titles and types preserved.
    let listing = stdout(&run(dst.path(), &["export", "csv"]));
    assert!(listing.contains(",bug,open,normal,bob,"), "{listing}");
    assert!(listing.contains("Login loops"), "{listing}");
    assert!(listing.contains("Dark mode"), "{listing}");
}

#[test]
fn import_json_default_type_applies_when_omitted() {
    let repo = fresh_repo();
    let file = repo.path().join("in.json");
    std::fs::write(&file, r#"[{"title":"Typeless"}]"#).unwrap();
    run(
        repo.path(),
        &["import", "json", file.to_str().unwrap(), "--default-type", "chore"],
    );
    let csv = stdout(&run(repo.path(), &["export", "csv"]));
    assert!(csv.contains(",chore,open,"), "{csv}");
}

#[test]
fn import_json_missing_title_fails_with_exit_1() {
    let repo = fresh_repo();
    let file = repo.path().join("bad.json");
    std::fs::write(&file, r#"[{"type":"bug"}]"#).unwrap();
    let out = run(repo.path(), &["import", "json", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{}", dump(&out));
}

#[test]
fn export_markdown_includes_heading_and_metadata() {
    let repo = fresh_repo();
    run(repo.path(), &["new", "--type", "bug", "--title", "Crash on boot"]);
    let md = stdout(&run(repo.path(), &["export", "markdown"]));
    assert!(md.starts_with("# Issues"), "{md}");
    assert!(md.contains("## Crash on boot ("), "{md}");
    assert!(md.contains("- **type**: bug"), "{md}");
}
