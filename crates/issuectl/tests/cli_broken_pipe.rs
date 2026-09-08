//! Black-box coverage for stdout failures. Process exit behavior and the
//! absence of panic diagnostics cannot be observed from an inline test.

use std::process::{Command, Stdio};

fn fresh_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(repo.path().join("issues")).expect("create issues directory");
    repo
}

#[cfg(unix)]
fn run_with_closed_stdout(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;

    let (reader, writer) = UnixStream::pair().expect("create stdout socket pair");
    drop(reader);
    let writer: OwnedFd = writer.into();

    Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("RUST_BACKTRACE", "1")
        .current_dir(root)
        .arg("--root")
        .arg(root)
        .args(args)
        .stdout(Stdio::from(writer))
        .stderr(Stdio::piped())
        .output()
        .expect("run issuectl")
}

#[cfg(unix)]
fn assert_silent_success(output: &std::process::Output) {
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

#[cfg(unix)]
#[test]
fn json_command_exits_successfully_when_stdout_consumer_closes_early() {
    let repo = fresh_repo();

    // The peer is closed before spawn, so the command's first stdout write
    // deterministically encounters BrokenPipe instead of racing the parent.
    let output = run_with_closed_stdout(repo.path(), &["--json", "config", "show"]);

    assert_silent_success(&output);
}

#[cfg(unix)]
#[test]
fn text_and_direct_writer_commands_also_tolerate_closed_stdout() {
    let repo = fresh_repo();

    assert_silent_success(&run_with_closed_stdout(repo.path(), &["config", "show"]));
    assert_silent_success(&run_with_closed_stdout(
        repo.path(),
        &["completions", "bash"],
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn non_broken_pipe_stdout_failure_remains_fatal() {
    let repo = fresh_repo();
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");

    let output = Command::new(env!("CARGO_BIN_EXE_issuectl"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("RUST_BACKTRACE", "0")
        .current_dir(repo.path())
        .arg("--root")
        .arg(repo.path())
        .args(["--json", "config", "show"])
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .output()
        .expect("run issuectl");

    assert!(!output.status.success(), "stdout ENOSPC must fail");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("stdout must be writable"),
        "stdout failure must remain surfaced, stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
