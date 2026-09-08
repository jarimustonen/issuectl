//! Black-box coverage for stdout failures. Process exit behavior and the
//! absence of panic diagnostics cannot be observed from an inline test.

use std::process::{Command, Stdio};

#[test]
fn json_command_exits_successfully_when_stdout_consumer_closes_early() {
    let repo = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(repo.path().join("issues")).expect("create issues directory");

    let mut child = Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("RUST_BACKTRACE", "1")
        .current_dir(repo.path())
        .arg("--root")
        .arg(repo.path())
        .args(["--json", "config", "show"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn issuectl");

    // Taking and immediately dropping the read end models a downstream
    // consumer that exits before issuectl writes its JSON result. Unlike a
    // size-dependent `| head` test, this deterministically leaves no reader.
    drop(child.stdout.take().expect("piped stdout"));

    let output = child.wait_with_output().expect("wait for issuectl");
    assert!(
        output.status.success(),
        "status: {:?}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "broken pipe must be silent, stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
