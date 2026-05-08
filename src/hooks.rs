//! `issuectl hooks install` — opt-in pre-commit hook that runs
//! `issuectl doctor` against the staged `issues/**` files. Mechanism is
//! `core.hooksPath = .githooks` (idiomatic, single source of truth, and
//! reverses cleanly): the hook script lives in-repo under `.githooks/`,
//! and the install command points git at it.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

/// Block of script we own. Wrapped in BEGIN/END markers so an existing
/// pre-commit can keep its own logic and we can splice ours in
/// idempotently.
const BEGIN_MARKER: &str = "# >>> issuectl hooks (managed) >>>";
const END_MARKER: &str = "# <<< issuectl hooks (managed) <<<";

const HOOK_BODY: &str = r#"# Run `issuectl doctor` against staged issues/** files. Aborts the
# commit if doctor reports critical findings. Skip with `--no-verify`.
changed=$(git diff --cached --name-only --diff-filter=ACMR -- 'issues/**' || true)
if [ -n "$changed" ]; then
    if ! issuectl doctor >/dev/null 2>&1; then
        echo "issuectl doctor found problems in staged issues/ files." >&2
        echo "Re-run \`issuectl doctor\` (or \`issuectl doctor --fix\`) for details." >&2
        exit 1
    fi
fi
"#;

pub fn run(repo_root: &Path, uninstall: bool) -> Result<()> {
    if uninstall {
        uninstall_hook(repo_root)
    } else {
        install_hook(repo_root)
    }
}

fn install_hook(repo_root: &Path) -> Result<()> {
    let hooks_dir = repo_root.join(".githooks");
    fs::create_dir_all(&hooks_dir)
        .with_context(|| format!("cannot create {}", hooks_dir.display()))?;
    let hook_path = hooks_dir.join("pre-commit");
    let new_contents = compose_hook(&read_existing(&hook_path));
    write_hook(&hook_path, &new_contents)?;
    set_git_config(repo_root, ".githooks")?;
    println!(
        "Installed pre-commit hook at {} and set core.hooksPath = .githooks.",
        hook_path.display()
    );
    println!("Bypass with `git commit --no-verify`. Uninstall with `issuectl hooks install --uninstall`.");
    Ok(())
}

fn uninstall_hook(repo_root: &Path) -> Result<()> {
    let hook_path = repo_root.join(".githooks/pre-commit");
    if hook_path.exists() {
        let existing = read_existing(&hook_path);
        let stripped = strip_managed_block(&existing);
        if stripped.trim().is_empty() || stripped.trim() == "#!/bin/sh" {
            fs::remove_file(&hook_path)
                .with_context(|| format!("cannot remove {}", hook_path.display()))?;
        } else {
            write_hook(&hook_path, &stripped)?;
        }
    }
    // Remove our git config pointer if it points at .githooks. Leave any
    // unrelated value intact.
    let current = current_hooks_path(repo_root)?;
    if current.as_deref() == Some(".githooks") {
        unset_git_config(repo_root)?;
    }
    println!("Uninstalled issuectl pre-commit hook.");
    Ok(())
}

fn read_existing(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn compose_hook(existing: &str) -> String {
    let stripped = strip_managed_block(existing);
    let trimmed = stripped.trim_end();
    let mut out = String::new();
    if trimmed.is_empty() {
        out.push_str("#!/bin/sh\n");
    } else if !trimmed.starts_with("#!") {
        out.push_str("#!/bin/sh\n");
        out.push_str(trimmed);
        out.push('\n');
    } else {
        out.push_str(trimmed);
        out.push('\n');
    }
    out.push('\n');
    out.push_str(BEGIN_MARKER);
    out.push('\n');
    out.push_str(HOOK_BODY);
    out.push_str(END_MARKER);
    out.push('\n');
    out
}

fn strip_managed_block(text: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in text.lines() {
        if line.trim() == BEGIN_MARKER {
            skipping = true;
            continue;
        }
        if line.trim() == END_MARKER {
            skipping = false;
            continue;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn write_hook(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents).with_context(|| format!("cannot write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

fn current_hooks_path(repo_root: &Path) -> Result<Option<String>> {
    let out = std::process::Command::new("git")
        .args(["config", "--local", "--get", "core.hooksPath"])
        .current_dir(repo_root)
        .output()
        .with_context(|| "running `git config`")?;
    if !out.status.success() {
        return Ok(None);
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        Ok(None)
    } else {
        Ok(Some(s))
    }
}

fn set_git_config(repo_root: &Path, value: &str) -> Result<()> {
    let status = std::process::Command::new("git")
        .args(["config", "--local", "core.hooksPath", value])
        .current_dir(repo_root)
        .status()
        .with_context(|| "running `git config`")?;
    if !status.success() {
        bail!("`git config core.hooksPath` failed");
    }
    Ok(())
}

fn unset_git_config(repo_root: &Path) -> Result<()> {
    let _ = std::process::Command::new("git")
        .args(["config", "--local", "--unset", "core.hooksPath"])
        .current_dir(repo_root)
        .status();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_git_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let st = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(st.success());
        tmp
    }

    #[test]
    fn install_writes_hook_and_sets_config() {
        let tmp = fresh_git_repo();
        run(tmp.path(), false).unwrap();
        let hook = tmp.path().join(".githooks/pre-commit");
        assert!(hook.is_file());
        let body = fs::read_to_string(&hook).unwrap();
        assert!(body.contains("issuectl doctor"));
        assert!(body.contains(BEGIN_MARKER));
        let cfg = current_hooks_path(tmp.path()).unwrap();
        assert_eq!(cfg.as_deref(), Some(".githooks"));
    }

    #[test]
    fn install_is_idempotent_and_preserves_user_block() {
        let tmp = fresh_git_repo();
        let dir = tmp.path().join(".githooks");
        fs::create_dir_all(&dir).unwrap();
        let hook = dir.join("pre-commit");
        fs::write(&hook, "#!/bin/sh\necho user-thing\n").unwrap();
        run(tmp.path(), false).unwrap();
        run(tmp.path(), false).unwrap();
        let body = fs::read_to_string(&hook).unwrap();
        assert!(body.contains("echo user-thing"));
        // Exactly one managed block.
        assert_eq!(body.matches(BEGIN_MARKER).count(), 1);
    }

    #[test]
    fn uninstall_removes_block_and_config() {
        let tmp = fresh_git_repo();
        run(tmp.path(), false).unwrap();
        run(tmp.path(), true).unwrap();
        let hook = tmp.path().join(".githooks/pre-commit");
        assert!(!hook.exists(), "hook should be gone");
        let cfg = current_hooks_path(tmp.path()).unwrap();
        assert_eq!(cfg, None);
    }

    #[test]
    fn uninstall_keeps_user_block_when_present() {
        let tmp = fresh_git_repo();
        let dir = tmp.path().join(".githooks");
        fs::create_dir_all(&dir).unwrap();
        let hook = dir.join("pre-commit");
        fs::write(&hook, "#!/bin/sh\necho user-thing\n").unwrap();
        run(tmp.path(), false).unwrap();
        run(tmp.path(), true).unwrap();
        let body = fs::read_to_string(&hook).unwrap();
        assert!(body.contains("echo user-thing"));
        assert!(!body.contains(BEGIN_MARKER));
    }
}

