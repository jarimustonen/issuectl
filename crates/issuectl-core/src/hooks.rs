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

const PRE_COMMIT_BODY: &str = r#"set -eu
# Run `issuectl doctor` against the staged snapshot of issues/. We
# validate the index, not the working tree — partial `git add` would
# otherwise smuggle broken content past the hook. Skip with
# `--no-verify` or `ISSUECTL_SKIP_DOCTOR=1`.

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

/// `commit-msg` hook body: the non-blocking `Refs-Issue` reminder.
///
/// This lives in `commit-msg` — not `pre-commit` — deliberately. Its
/// `$1` is the path to the file holding the *final* commit message git
/// will use, so it sees the trailer regardless of how the message was
/// supplied (`-m`, `-F <file>`, `-F -`/stdin, or the editor). A
/// `pre-commit` hook runs before that message exists and cannot, which
/// is why the reminder used to false-fire on a message piped via
/// `git commit -F -` that already carried a matching trailer.
const COMMIT_MSG_BODY: &str = r#"set -eu
# Non-blocking reminder when the current branch name resolves to a
# known issue slug — direct or `<prefix>-<slug>` / `<prefix>/<slug>`
# shape. Encourages adding `Refs-Issue: @<slug>` trailers so
# `issuectl sync-commits` can attribute the commit. Always advisory;
# never fails the commit.
#
# $1 is the file holding the final commit message git will use, so the
# check sees a trailer supplied by any path (`-m`, `-F <file>`,
# `-F -`/stdin, editor) — not just the editor buffer.
#
# Skipped during in-progress rebase / cherry-pick / merge so the
# reminder doesn't spam the user once per replayed commit.
#
# Always advisory — every path exits 0 so a hiccup here never blocks a
# commit.
msg_file=${1:-}
[ -n "$msg_file" ] || exit 0
git_dir=$(git rev-parse --git-dir 2>/dev/null || echo .git)
if [ -d "$git_dir/rebase-merge" ] || [ -d "$git_dir/rebase-apply" ] \
   || [ -f "$git_dir/CHERRY_PICK_HEAD" ] || [ -f "$git_dir/MERGE_HEAD" ]; then
    exit 0
fi
branch=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)
[ -n "$branch" ] || exit 0
candidate=""
if [ -f "issues/$branch/item.md" ]; then
    candidate="$branch"
else
    # Always reduce to the last path segment first so a branch like
    # `dir-name/feat-foo-bar-baz` peels off the `dir-name/feat-`
    # prefix to reach `foo-bar-baz`.
    base="${branch##*/}"
    if [ "$base" != "$branch" ] && [ -f "issues/$base/item.md" ]; then
        candidate="$base"
    else
        rest="$base"
        while case "$rest" in *-*) true;; *) false;; esac; do
            rest="${rest#*-}"
            # `${rest#*-}` on `-foo` yields `foo`, but on a branch
            # named `-` yields the empty string — without this guard,
            # `[ -f "issues//item.md" ]` would match `issues/item.md`
            # if it existed.
            [ -n "$rest" ] || break
            if [ -f "issues/$rest/item.md" ]; then
                candidate="$rest"
                break
            fi
        done
    fi
fi
[ -n "$candidate" ] || exit 0
# The candidate is spliced into the ERE below. It is only ever a known
# issue slug (`[a-z0-9-]`), but it is derived from on-disk directory
# names, so guard against a hand-created `issues/<weird>/item.md`
# smuggling ERE metacharacters into the pattern — anything non-slug,
# stay silent rather than build a malformed regex.
case "$candidate" in
    *[!a-z0-9-]*) exit 0 ;;
esac
# Suppress the reminder when the message already carries a matching
# `Refs-Issue:`/`Fixes-Issue:` trailer for this slug. Parse the message
# with git's own trailer parser (`interpret-trailers --parse`) instead
# of grepping the raw file, so the check honours git's last-paragraph
# trailer-block semantics and ignores comment lines and anything below
# a `# ---- >8 ----` scissors line — the same trailers `issuectl
# sync-commits` will (and won't) attribute. Token is case-insensitive;
# the value may be bare or prefixed with `@`/`#`, and `^…$` anchoring
# requires the value to be exactly this slug.
if git interpret-trailers --parse <"$msg_file" 2>/dev/null \
    | grep -iEq "^(Refs-Issue|Fixes-Issue):[[:space:]]*[@#]?${candidate}[[:space:]]*$"; then
    exit 0
fi
printf 'issuectl: branch matches @%s; consider adding `Refs-Issue: @%s` to your commit message (or run `issuectl sync-commits`).\n' \
    "$candidate" "$candidate" >&2 || :
exit 0
"#;

/// Stash key in git config where we record the previous
/// `core.hooksPath` value so uninstall can restore it.
const PRIOR_HOOKS_PATH_KEY: &str = "issuectl.priorHooksPath";

pub fn run(repo_root: &Path, uninstall: bool, force: bool) -> Result<()> {
    if uninstall {
        uninstall_hook(repo_root)
    } else {
        let outcome = install_hook(repo_root, force)?;
        let hooks_dir = repo_root.join(".githooks");
        match outcome {
            InstallOutcome::Installed => {
                println!(
                    "Installed pre-commit and commit-msg hooks in {} and set core.hooksPath = .githooks.",
                    hooks_dir.display()
                );
                println!("Bypass with `git commit --no-verify`. Uninstall with `issuectl hooks install --uninstall`.");
            }
            InstallOutcome::AlreadyInstalled => {
                println!(
                    "Pre-commit and commit-msg hooks already installed in {} (core.hooksPath = .githooks).",
                    hooks_dir.display()
                );
            }
        }
        Ok(())
    }
}

/// Outcome reporter for `install_hook` so orchestrators (e.g. `init`)
/// can distinguish "newly installed" from "already installed" without
/// re-checking filesystem state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed,
    AlreadyInstalled,
}

/// Install the pre-commit hook and configure `core.hooksPath`. Returns
/// whether the hook block was newly added (or rewritten) or was already
/// present and current. Idempotent — re-running with the hook already
/// in place is a no-op aside from the `core.hooksPath` reassertion.
/// Used by `issuectl init` directly; `hooks::run` wraps this and
/// adds human-readable output.
pub fn install_hook(repo_root: &Path, force: bool) -> Result<InstallOutcome> {
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
    // Two managed hooks: `pre-commit` runs `issuectl doctor` on the
    // staged snapshot; `commit-msg` prints the advisory `Refs-Issue`
    // reminder (needs the final message file, unavailable at
    // pre-commit time). Neither is written when already current.
    let pre_changed = sync_hook_file(&hooks_dir, "pre-commit", PRE_COMMIT_BODY)?;
    let msg_changed = sync_hook_file(&hooks_dir, "commit-msg", COMMIT_MSG_BODY)?;
    set_git_config_key(repo_root, "core.hooksPath", ".githooks")?;
    let already_current = !pre_changed && !msg_changed && current.as_deref() == Some(".githooks");
    Ok(if already_current {
        InstallOutcome::AlreadyInstalled
    } else {
        InstallOutcome::Installed
    })
}

/// Compose and write one managed hook file, splicing our block into any
/// pre-existing user script. Returns whether the file's contents were
/// changed (a no-op when already current, so idempotent re-runs don't
/// rewrite).
fn sync_hook_file(hooks_dir: &Path, name: &str, body: &str) -> Result<bool> {
    let path = hooks_dir.join(name);
    let existing = read_existing(&path);
    let new_contents = compose_hook(&existing, body);
    if !existing.is_empty() && existing == new_contents {
        return Ok(false);
    }
    write_hook(&path, &new_contents)?;
    Ok(true)
}

fn uninstall_hook(repo_root: &Path) -> Result<()> {
    let hooks_dir = repo_root.join(".githooks");
    for name in ["pre-commit", "commit-msg"] {
        strip_managed_hook(&hooks_dir.join(name))?;
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
    println!("Uninstalled issuectl pre-commit and commit-msg hooks.");
    Ok(())
}

fn read_existing(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// Strip our managed block from a hook file, removing the file when
/// nothing but the shebang remains and rewriting it otherwise. A no-op
/// when the file does not exist.
fn strip_managed_hook(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let existing = read_existing(path);
    let stripped = strip_managed_block(&existing);
    if stripped.trim().is_empty() || stripped.trim() == "#!/bin/sh" {
        fs::remove_file(path).with_context(|| format!("cannot remove {}", path.display()))?;
    } else {
        write_hook(path, &stripped)?;
    }
    Ok(())
}

fn compose_hook(existing: &str, body: &str) -> String {
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
    out.push_str(body);
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
        let pre = tmp.path().join(".githooks/pre-commit");
        assert!(pre.is_file());
        let pre_body = fs::read_to_string(&pre).unwrap();
        assert!(pre_body.contains("issuectl doctor"));
        assert!(pre_body.contains(BEGIN_MARKER));
        // The advisory reminder now lives in `commit-msg`, not
        // `pre-commit` (a pre-commit hook can't see the final message).
        assert!(!pre_body.contains("branch matches"));
        let msg = tmp.path().join(".githooks/commit-msg");
        assert!(msg.is_file());
        let msg_body = fs::read_to_string(&msg).unwrap();
        assert!(msg_body.contains("branch matches"));
        assert!(msg_body.contains(BEGIN_MARKER));
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
        assert!(
            !tmp.path().join(".githooks/pre-commit").exists(),
            "pre-commit hook should be gone"
        );
        assert!(
            !tmp.path().join(".githooks/commit-msg").exists(),
            "commit-msg hook should be gone"
        );
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
        // Neither hook file should have been written.
        assert!(!tmp.path().join(".githooks/pre-commit").exists());
        assert!(!tmp.path().join(".githooks/commit-msg").exists());
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

    #[cfg(unix)]
    #[test]
    fn commit_msg_hint_respects_trailer_supplied_via_stdin() {
        // Regression for the `-F -`/stdin false-fire: the `Refs-Issue`
        // reminder must be suppressed when the *final* commit message
        // already carries a matching trailer, even when that message is
        // piped in on stdin (which a `pre-commit` hook never sees). It
        // must still fire for a message with no trailer.
        use std::io::Write;
        use std::process::{Command, Stdio};

        let tmp = fresh_git_repo();
        for (k, v) in [("user.email", "t@example.com"), ("user.name", "t")] {
            let st = Command::new("git")
                .args(["config", "--local", k, v])
                .current_dir(tmp.path())
                .status()
                .unwrap();
            assert!(st.success());
        }
        run(tmp.path(), false, false).unwrap();

        // Branch named exactly as the issue slug so the reminder
        // resolves a candidate. `checkout -b` on the unborn branch
        // just renames it.
        let slug = "foo-bar-baz";
        let st = Command::new("git")
            .args(["checkout", "-q", "-b", slug])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(st.success());

        let item = tmp.path().join(format!("issues/{slug}/item.md"));
        fs::create_dir_all(item.parent().unwrap()).unwrap();

        // Commit via `git commit -F -`, feeding `msg` on stdin.
        // `ISSUECTL_SKIP_DOCTOR=1` short-circuits the pre-commit hook so
        // the test needs no `issuectl` on PATH — we exercise commit-msg.
        let commit_via_stdin = |msg: &str| -> (bool, String) {
            let mut child = Command::new("git")
                .args(["commit", "-F", "-"])
                .env("ISSUECTL_SKIP_DOCTOR", "1")
                .current_dir(tmp.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(msg.as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            (
                out.status.success(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            )
        };

        let stage = || {
            let st = Command::new("git")
                .args(["add", "issues/"])
                .current_dir(tmp.path())
                .status()
                .unwrap();
            assert!(st.success());
        };

        // 1. Message with a matching trailer (via stdin) → suppressed.
        fs::write(&item, "issue\n").unwrap();
        stage();
        let (ok, err) = commit_via_stdin("do a thing\n\nRefs-Issue: @foo-bar-baz\n");
        assert!(ok, "commit should succeed; stderr: {err}");
        assert!(
            !err.contains("consider adding"),
            "hint must be suppressed when trailer present via -F -; got: {err}"
        );

        // 2. Next commit, message with no trailer → hint fires.
        fs::write(&item, "issue v2\n").unwrap();
        stage();
        let (ok, err) = commit_via_stdin("another thing, no trailer\n");
        assert!(ok, "commit should succeed; stderr: {err}");
        assert!(
            err.contains("consider adding") && err.contains(slug),
            "hint must fire when no matching trailer; got: {err}"
        );

        // 3. Trailer-shaped line only in the BODY (not the last
        // paragraph) → not a real trailer, so `sync-commits` won't
        // attribute it and the hint must still fire. This is what
        // parsing via `git interpret-trailers` (last-paragraph
        // semantics) buys us over a raw whole-file grep.
        fs::write(&item, "issue v3\n").unwrap();
        stage();
        let (ok, err) =
            commit_via_stdin("Refs-Issue: @foo-bar-baz was mentioned here\n\nreal body last\n");
        assert!(ok, "commit should succeed; stderr: {err}");
        assert!(
            err.contains("consider adding"),
            "hint must fire when the trailer is only in the body, not the trailer block; got: {err}"
        );
    }

    #[test]
    fn install_upgrades_old_combined_pre_commit_and_moves_reminder() {
        // Primary deployment path for the fix: a repo that installed the
        // previous single-hook version has the advisory reminder baked
        // into `pre-commit`'s managed block. Re-running install must
        // strip the reminder from `pre-commit`, leave the doctor gate,
        // and relocate the reminder into a new `commit-msg` hook — while
        // preserving any user script outside the managed markers.
        let tmp = fresh_git_repo();
        let dir = tmp.path().join(".githooks");
        fs::create_dir_all(&dir).unwrap();
        let pre = dir.join("pre-commit");
        // Simulate the old combined managed block: a user prologue, then
        // a managed block carrying BOTH the reminder and the doctor gate.
        let old = format!(
            "#!/bin/sh\necho user-thing\n\n{BEGIN_MARKER}\nset -eu\n\
             printf 'issuectl: branch matches @%s; ...' \"$c\" >&2\n\
             issuectl doctor\n{END_MARKER}\n"
        );
        fs::write(&pre, &old).unwrap();
        set_git_config_key(tmp.path(), "core.hooksPath", ".githooks").unwrap();

        run(tmp.path(), false, false).unwrap();

        let pre_body = fs::read_to_string(&pre).unwrap();
        assert!(pre_body.contains("echo user-thing"), "user block preserved");
        assert!(pre_body.contains("issuectl doctor"), "doctor gate kept");
        assert!(
            !pre_body.contains("branch matches"),
            "reminder removed from pre-commit; got:\n{pre_body}"
        );
        assert_eq!(
            pre_body.matches(BEGIN_MARKER).count(),
            1,
            "exactly one managed block in pre-commit"
        );

        let msg = dir.join("commit-msg");
        let msg_body = fs::read_to_string(&msg).unwrap();
        assert!(
            msg_body.contains("branch matches"),
            "reminder relocated to commit-msg"
        );
        assert_eq!(msg_body.matches(BEGIN_MARKER).count(), 1);

        // Re-running is a no-op (idempotent upgrade).
        let before = (pre_body, msg_body);
        run(tmp.path(), false, false).unwrap();
        let after = (
            fs::read_to_string(&pre).unwrap(),
            fs::read_to_string(&msg).unwrap(),
        );
        assert_eq!(before, after, "second install must not rewrite anything");

        // Uninstall removes the new commit-msg hook and keeps the user
        // prologue in pre-commit.
        run(tmp.path(), true, false).unwrap();
        assert!(!msg.exists(), "commit-msg removed on uninstall");
        let pre_after = fs::read_to_string(&pre).unwrap();
        assert!(pre_after.contains("echo user-thing"));
        assert!(!pre_after.contains(BEGIN_MARKER));
    }
}
