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

const HOOK_BODY: &str = r#"set -eu
# Run `issuectl doctor` against the staged snapshot of issues/. We
# validate the index, not the working tree — partial `git add` would
# otherwise smuggle broken content past the hook. Skip with
# `--no-verify` or `ISSUECTL_SKIP_DOCTOR=1`.

# Non-blocking reminder when the current branch name resolves to a
# known issue slug — direct or `<prefix>-<slug>` / `<prefix>/<slug>`
# shape. Encourages adding `Refs-Issue: @<slug>` trailers so
# `issuectl sync-commits` can attribute the commit. Always advisory;
# never fails the commit.
branch=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)
if [ -n "$branch" ]; then
    candidate=""
    if [ -f "issues/$branch/item.md" ]; then
        candidate="$branch"
    else
        # Try `<prefix>/<tail>` and `<prefix>-<tail>` shapes.
        tail_slash="${branch##*/}"
        if [ "$tail_slash" != "$branch" ] && [ -f "issues/$tail_slash/item.md" ]; then
            candidate="$tail_slash"
        else
            rest="$branch"
            while case "$rest" in *-*) true;; *) false;; esac; do
                rest="${rest#*-}"
                if [ -f "issues/$rest/item.md" ]; then
                    candidate="$rest"
                    break
                fi
            done
        fi
    fi
    if [ -n "$candidate" ]; then
        printf 'issuectl: branch matches @%s; consider adding `Refs-Issue: @%s` to your commit message (or run `issuectl sync-commits`).\n' \
            "$candidate" "$candidate" >&2
    fi
fi

if [ "${ISSUECTL_SKIP_DOCTOR:-}" = "1" ]; then
    exit 0
fi

# Fail closed if `git diff` cannot inspect the index.
if ! changed=$(git diff --cached --name-only --diff-filter=ACMRD -- issues/ 2>/dev/null); then
    echo "issuectl: failed to inspect staged changes" >&2
    exit 1
fi
if [ -z "$changed" ]; then
    exit 0
fi

if ! command -v issuectl >/dev/null 2>&1; then
    echo "issuectl pre-commit hook installed but \`issuectl\` is not on PATH." >&2
    echo "Add it to PATH or remove the hook with \`issuectl hooks install --uninstall\`." >&2
    exit 1
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/issuectl-hook.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

# Anchor `issuectl doctor`'s repo-root walk inside the temp dir so a
# commit that deletes the last issues/ entry doesn't make doctor walk
# up out of /tmp searching for `issues/` or `.git`.
mkdir -p "$tmp/issues"

# Materialise only the staged `issues/` tree — not the entire index —
# so a monorepo with GBs of unrelated tracked files doesn't pay
# checkout cost on every commit.
if ! git ls-files -z --cached -- issues/ \
    | git checkout-index -z --stdin --prefix="$tmp/" >/dev/null 2>&1; then
    echo "issuectl: failed to materialize staged issues/ snapshot" >&2
    exit 1
fi

if ! out=$(cd "$tmp" && issuectl doctor 2>&1); then
    printf '%s\n' "$out" >&2
    echo "" >&2
    echo "issuectl doctor blocked the commit. Run \`issuectl doctor --fix\` and re-stage," >&2
    echo "bypass with \`ISSUECTL_SKIP_DOCTOR=1 git commit\` or \`git commit --no-verify\`." >&2
    exit 1
fi
"#;

/// Stash key in git config where we record the previous
/// `core.hooksPath` value so uninstall can restore it.
const PRIOR_HOOKS_PATH_KEY: &str = "issuectl.priorHooksPath";

pub fn run(repo_root: &Path, uninstall: bool, force: bool) -> Result<()> {
    if uninstall {
        uninstall_hook(repo_root)
    } else {
        install_hook(repo_root, force)
    }
}

fn install_hook(repo_root: &Path, force: bool) -> Result<()> {
    // Refuse to be in a non-git directory before touching the
    // filesystem — `git config --local` would fail later anyway, but
    // not before we'd already written `.githooks/pre-commit`.
    let st = std::process::Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(repo_root)
        .output()
        .with_context(|| "running `git rev-parse`")?;
    if !st.status.success() {
        bail!("{} is not inside a git repository", repo_root.display());
    }

    // Refuse to clobber a non-default `core.hooksPath` (Husky,
    // lefthook, pre-commit.com all set this). Stash the prior value
    // so uninstall can restore it.
    let current = current_hooks_path(repo_root)?;
    if let Some(ref existing) = current {
        if existing != ".githooks" && !force {
            bail!(
                "core.hooksPath is already set to {existing:?}; refusing to overwrite. \
                 Re-run with --force, or splice the issuectl block manually into {existing}/pre-commit."
            );
        }
        if existing != ".githooks" && force {
            // Stash for restore-on-uninstall.
            set_git_config_key(repo_root, PRIOR_HOOKS_PATH_KEY, existing)?;
        }
    }

    let hooks_dir = repo_root.join(".githooks");
    fs::create_dir_all(&hooks_dir)
        .with_context(|| format!("cannot create {}", hooks_dir.display()))?;
    let hook_path = hooks_dir.join("pre-commit");
    let new_contents = compose_hook(&read_existing(&hook_path));
    write_hook(&hook_path, &new_contents)?;
    set_git_config_key(repo_root, "core.hooksPath", ".githooks")?;
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
    // Restore the prior `core.hooksPath` if we stashed one on install;
    // otherwise unset (only when current value is still `.githooks` —
    // a manual edit since install is left intact).
    let current = current_hooks_path(repo_root)?;
    if current.as_deref() == Some(".githooks") {
        let prior = get_git_config_key(repo_root, PRIOR_HOOKS_PATH_KEY)?;
        match prior {
            Some(p) => {
                set_git_config_key(repo_root, "core.hooksPath", &p)?;
                unset_git_config_key(repo_root, PRIOR_HOOKS_PATH_KEY)?;
            }
            None => unset_git_config_key(repo_root, "core.hooksPath")?,
        }
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
    get_git_config_key(repo_root, "core.hooksPath")
}

fn get_git_config_key(repo_root: &Path, key: &str) -> Result<Option<String>> {
    let out = std::process::Command::new("git")
        .args(["config", "--local", "--get", key])
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

fn set_git_config_key(repo_root: &Path, key: &str, value: &str) -> Result<()> {
    let status = std::process::Command::new("git")
        .args(["config", "--local", key, value])
        .current_dir(repo_root)
        .status()
        .with_context(|| "running `git config`")?;
    if !status.success() {
        bail!("`git config {key}` failed");
    }
    Ok(())
}

fn unset_git_config_key(repo_root: &Path, key: &str) -> Result<()> {
    // `--unset` exits 5 when the key is already absent; treat that as
    // success (idempotent) but bail on other failures.
    let out = std::process::Command::new("git")
        .args(["config", "--local", "--unset", key])
        .current_dir(repo_root)
        .output()
        .with_context(|| "running `git config --unset`")?;
    let code = out.status.code().unwrap_or(-1);
    if !out.status.success() && code != 5 {
        bail!(
            "`git config --unset {key}` failed (exit {code}): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
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
        run(tmp.path(), false, false).unwrap();
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
        run(tmp.path(), false, false).unwrap();
        run(tmp.path(), false, false).unwrap();
        let body = fs::read_to_string(&hook).unwrap();
        assert!(body.contains("echo user-thing"));
        // Exactly one managed block.
        assert_eq!(body.matches(BEGIN_MARKER).count(), 1);
    }

    #[test]
    fn uninstall_removes_block_and_config() {
        let tmp = fresh_git_repo();
        run(tmp.path(), false, false).unwrap();
        run(tmp.path(), true, false).unwrap();
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
        run(tmp.path(), false, false).unwrap();
        run(tmp.path(), true, false).unwrap();
        let body = fs::read_to_string(&hook).unwrap();
        assert!(body.contains("echo user-thing"));
        assert!(!body.contains(BEGIN_MARKER));
    }

    #[test]
    fn install_refuses_when_existing_non_default_hooks_path() {
        let tmp = fresh_git_repo();
        set_git_config_key(tmp.path(), "core.hooksPath", ".husky").unwrap();
        let err = run(tmp.path(), false, false).unwrap_err();
        assert!(err.to_string().contains(".husky"), "got: {err}");
        // Hook file should not have been written.
        assert!(!tmp.path().join(".githooks/pre-commit").exists());
        // Existing config preserved.
        let cfg = current_hooks_path(tmp.path()).unwrap();
        assert_eq!(cfg.as_deref(), Some(".husky"));
    }

    #[test]
    fn install_with_force_stashes_prior_path_and_uninstall_restores() {
        let tmp = fresh_git_repo();
        set_git_config_key(tmp.path(), "core.hooksPath", ".husky").unwrap();
        run(tmp.path(), false, true).unwrap();
        let cfg = current_hooks_path(tmp.path()).unwrap();
        assert_eq!(cfg.as_deref(), Some(".githooks"));
        let stashed = get_git_config_key(tmp.path(), PRIOR_HOOKS_PATH_KEY).unwrap();
        assert_eq!(stashed.as_deref(), Some(".husky"));
        run(tmp.path(), true, false).unwrap();
        let cfg = current_hooks_path(tmp.path()).unwrap();
        assert_eq!(cfg.as_deref(), Some(".husky"), "prior path restored");
        let stashed_after = get_git_config_key(tmp.path(), PRIOR_HOOKS_PATH_KEY).unwrap();
        assert_eq!(stashed_after, None, "stash key cleaned up");
    }

    #[test]
    fn install_refuses_in_non_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let err = run(tmp.path(), false, false).unwrap_err();
        assert!(err.to_string().contains("not inside a git repository"));
        assert!(!tmp.path().join(".githooks/pre-commit").exists());
    }

    #[cfg(unix)]
    #[test]
    fn hook_body_validates_staged_snapshot_not_working_tree() {
        // Drives the actual shell hook against a stub `issuectl` that
        // exits non-zero iff the staged snapshot it sees has the
        // sentinel content. Confirms the hook reads the index, not
        // the working tree.
        use std::os::unix::fs::PermissionsExt;
        let tmp = fresh_git_repo();
        // Configure git identity so commits work.
        for (k, v) in [("user.email", "t@example.com"), ("user.name", "t")] {
            let st = std::process::Command::new("git")
                .args(["config", "--local", k, v])
                .current_dir(tmp.path())
                .status()
                .unwrap();
            assert!(st.success());
        }
        // Stub `issuectl` that fails when the staged snapshot under
        // its cwd contains the bad sentinel marker.
        let stub_dir = tmp.path().join("stub");
        fs::create_dir_all(&stub_dir).unwrap();
        let stub = stub_dir.join("issuectl");
        fs::write(
            &stub,
            "#!/bin/sh\nif grep -rq STAGED_BAD issues/ 2>/dev/null; then\n  echo BAD >&2; exit 1\nfi\nexit 0\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&stub, perms).unwrap();

        run(tmp.path(), false, false).unwrap();

        // Stage CLEAN content, then dirty the working tree without
        // staging. Hook must pass: index is clean.
        let item = tmp.path().join("issues/foo/item.md");
        fs::create_dir_all(item.parent().unwrap()).unwrap();
        fs::write(&item, "STAGED_OK\n").unwrap();
        let st = std::process::Command::new("git")
            .args(["add", "issues/foo/item.md"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(st.success());
        // Now corrupt the working tree only.
        fs::write(&item, "STAGED_BAD\n").unwrap();
        let mut new_path = std::env::var("PATH").unwrap_or_default();
        new_path = format!("{}:{new_path}", stub_dir.display());
        let st = std::process::Command::new("git")
            .args(["commit", "-m", "clean staged"])
            .env("PATH", &new_path)
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(st.success(), "hook must pass when index is clean");

        // Now stage the bad content; hook must fail.
        let st = std::process::Command::new("git")
            .args(["add", "issues/foo/item.md"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(st.success());
        let st = std::process::Command::new("git")
            .args(["commit", "-m", "bad staged"])
            .env("PATH", &new_path)
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(!st.success(), "hook must fail when staged snapshot is bad");
    }
}
