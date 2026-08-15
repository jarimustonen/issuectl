//! Parse `Refs-Issue:` / `Fixes-Issue:` trailers from git commit
//! messages. The two recognized tokens are case-insensitive, and the
//! value is a single slug optionally prefixed with `@` or `#` (the
//! `@`/`#` is stripped before validation).
//!
//! Trailer block detection is a slimmed-down version of git's own
//! rules: only the *last* paragraph of the message body is considered,
//! and only when every non-blank line in that paragraph matches the
//! `Token: value` shape (token = `[A-Za-z0-9-]+`). This avoids
//! spawning `git interpret-trailers` once per commit and matches the
//! conventional case where trailers are the bottom block.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::slug;

/// One commit's data after trailer extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    /// Full 40-char SHA-1 from `git log --format=%H`. Stored in full
    /// because idempotent dedup needs an unambiguous identity:
    /// abbreviated hashes can collide on prefix at long-tail repo
    /// sizes, and the resulting "looks duplicate" silently drops a
    /// legitimate commit. The CLI presenter can still abbreviate
    /// for display — the storage layer doesn't.
    pub hash: String,
    /// Subject line (`%s`).
    pub summary: String,
    /// Slugs from `Refs-Issue:` trailers (de-duped, in order seen).
    pub refs_issue: Vec<String>,
    /// Slugs from `Fixes-Issue:` trailers (de-duped, in order seen).
    pub fixes_issue: Vec<String>,
}

impl CommitInfo {
    pub fn has_any_trailer(&self) -> bool {
        !self.refs_issue.is_empty() || !self.fixes_issue.is_empty()
    }
}

/// Walk `git log <range>` and parse each commit's trailers. `range`
/// may be empty (defaults to git's "everything reachable from HEAD").
pub fn parse_log(repo_root: &Path, range: &str) -> Result<Vec<CommitInfo>> {
    // NUL bytes between fields and between records: git rejects NUL
    // in commit messages, so the framing is unambiguous regardless
    // of body content. (The earlier RS/FS bytes 0x1e/0x1f were not
    // — git permits them in messages, so a malicious or
    // accidentally-binary body could desync the parser.)
    let format = "--format=%H%x00%s%x00%B%x00";
    // Reject ranges that look like flags (e.g. `--output=...`) before
    // they reach git's argv. We do not run a shell, so this is purely
    // about preventing git itself from interpreting the value as an
    // option. `clap` already strips the leading `--range`, but the
    // value it carries is user-supplied.
    if range.starts_with('-') {
        bail!(
            "range expression must not start with '-' (got {range:?}); pass `<rev>..<rev>` or a SHA"
        );
    }
    let mut cmd = Command::new("git");
    cmd.arg("log").arg(format);
    if !range.is_empty() {
        cmd.arg(range);
    }
    let out = cmd
        .current_dir(repo_root)
        .output()
        .with_context(|| "running `git log`")?;
    if !out.status.success() {
        bail!(
            "`git log` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut commits = Vec::new();
    // Split on NUL: %H%x00%s%x00%B%x00 means each commit yields
    // exactly three NUL-separated fields, then a trailing NUL ending
    // the record. So a chunked iter of three fields per commit is
    // the natural shape.
    let mut parts = text.split('\0');
    loop {
        let Some(hash) = parts.next() else { break };
        // `tformat:`-style output (the default) appends a newline
        // after each record's terminator, which `split('\0')` then
        // surfaces as a leading `\n` on the next record's hash
        // field. Trim it; an empty hash after trimming means we hit
        // the trailing NUL past the last record — stop.
        let hash = hash.trim_start_matches('\n');
        if hash.is_empty() {
            break;
        }
        let summary = parts.next().unwrap_or("");
        let body = parts.next().unwrap_or("");
        let (refs_issue, fixes_issue) = parse_trailers(body);
        commits.push(CommitInfo {
            hash: hash.to_string(),
            summary: summary.to_string(),
            refs_issue,
            fixes_issue,
        });
    }
    Ok(commits)
}

/// Find `Refs-Issue:` / `Fixes-Issue:` trailers in a commit body.
/// Returns (refs_issue_slugs, fixes_issue_slugs), each de-duped in
/// order of first appearance.
pub fn parse_trailers(body: &str) -> (Vec<String>, Vec<String>) {
    let trimmed = body.trim_end_matches(['\n', '\r']);
    // The trailer block is the last paragraph (separated from the
    // rest of the message by a blank line). Walk lines from the end,
    // collect until we hit a blank line, then verify the collected
    // block is a real trailer block.
    let mut block: Vec<&str> = Vec::new();
    for line in trimmed.lines().rev() {
        if line.trim().is_empty() {
            break;
        }
        block.push(line);
    }
    block.reverse();
    if block.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if !block.iter().all(|l| is_trailer_line(l)) {
        return (Vec::new(), Vec::new());
    }
    let mut refs_issue: Vec<String> = Vec::new();
    let mut fixes_issue: Vec<String> = Vec::new();
    for line in &block {
        let Some((token, value)) = split_trailer(line) else {
            continue;
        };
        let bucket = if token.eq_ignore_ascii_case("Refs-Issue") {
            &mut refs_issue
        } else if token.eq_ignore_ascii_case("Fixes-Issue") {
            &mut fixes_issue
        } else {
            continue;
        };
        let raw = value.trim();
        let stripped = raw
            .strip_prefix('@')
            .or_else(|| raw.strip_prefix('#'))
            .unwrap_or(raw);
        // Reject anything that isn't a valid slug — protects against
        // accidental garbage like `Refs-Issue: TBD` or `Refs-Issue:
        // <fill in>`.
        if !slug::is_valid(stripped) {
            continue;
        }
        let s = stripped.to_string();
        if !bucket.iter().any(|x| x == &s) {
            bucket.push(s);
        }
    }
    (refs_issue, fixes_issue)
}

fn is_trailer_line(line: &str) -> bool {
    // Permit `# ...` git-template comment lines so a trailer block
    // immediately preceded by `# Conflicts:` doesn't disqualify the
    // whole block. Other shapes must match `Token: value`.
    if line.trim_start().starts_with('#') {
        return true;
    }
    split_trailer(line).is_some()
}

fn split_trailer(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':')?;
    let token = &line[..colon];
    if token.is_empty() {
        return None;
    }
    if !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    Some((token, &line[colon + 1..]))
}

/// Determine the default range for `sync-commits`: `<base>..HEAD`
/// where `base` is the merge-base of HEAD with `main` (falls back to
/// `master`). Returns `None` if no merge-base can be found (e.g.,
/// fresh repo with only HEAD's history) — caller should then walk all
/// of HEAD.
pub fn default_range(repo_root: &Path) -> Result<Option<String>> {
    for candidate in ["main", "master"] {
        let out = Command::new("git")
            .args(["merge-base", "HEAD", candidate])
            .current_dir(repo_root)
            .output()
            .with_context(|| "running `git merge-base`")?;
        if out.status.success() {
            let base = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !base.is_empty() {
                return Ok(Some(format!("{base}..HEAD")));
            }
        }
    }
    Ok(None)
}

/// Current branch name (`git symbolic-ref --short HEAD`). Returns
/// `None` for a detached HEAD.
pub fn current_branch(repo_root: &Path) -> Result<Option<String>> {
    let out = Command::new("git")
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .current_dir(repo_root)
        .output()
        .with_context(|| "running `git symbolic-ref`")?;
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

/// Outcome of [`stamp_fixes_trailer`]. Stamping is a best-effort,
/// fail-safe side effect: it never aborts the close it rides on, so
/// every "couldn't / didn't stamp" case is modelled as data, never an
/// error. The caller (`cmd_close`) maps any residual `Err` from an
/// unexpected git fault to [`StampOutcome::Skipped`] too, so a stamp
/// failure never turns a successful close into a command failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StampOutcome {
    /// A `Fixes-Issue: @<slug>` trailer was written and HEAD moved to
    /// the rewritten commit. Carries the *new* sha (`old` is the sha
    /// before the rewrite, so a caller can fix up any reference it had
    /// already recorded to the pre-stamp commit).
    Stamped { old: String, sha: String },
    /// HEAD already carried a matching `Fixes-Issue:` trailer for this
    /// slug — nothing rewritten, sha unchanged. Idempotent re-run.
    AlreadyPresent { sha: String },
    /// Stamping was declined for a safety reason. Non-fatal.
    Skipped { reason: String },
}

/// Rewrite the current HEAD commit so its message carries
/// `Fixes-Issue: @<slug>` — the exact trailer shape [`parse_trailers`]
/// accepts and `report::changelog` compiles — so an issue closed with
/// `issuectl close --stamp` seeds the trailer-driven changelog with
/// zero human discipline.
///
/// **Mechanism.** Plumbing, not `git commit --amend`: the trailer is
/// appended to the exact original message bytes (as its own paragraph,
/// so [`parse_trailers`]' last-paragraph rule always sees it), a new
/// commit object is built with [`git commit-tree`] over HEAD's *own*
/// tree and parents while preserving the original author and committer
/// identity/dates, and HEAD is moved with a compare-and-swap
/// `git update-ref HEAD <new> <old>`. This deliberately bypasses the
/// index, hooks, `commit.cleanup`, and GPG re-signing — the tree is
/// provably unchanged and the ref update is atomic, so no staged change
/// can be folded in and no concurrent HEAD move can be clobbered.
///
/// **Fail-safe.** Every unsafe or ambiguous situation returns
/// `Ok(StampOutcome::Skipped {..})` rather than erroring:
/// - not inside a git repo, or HEAD has no commit yet;
/// - a detached HEAD (would rewrite whatever commit happens to be
///   checked out — e.g. during `git bisect` or a manual sha checkout);
/// - an in-progress rebase / cherry-pick / merge / revert;
/// - HEAD is a merge commit (2+ parents) — the landing fix is a normal
///   commit, and we won't rewrite a merge;
/// - HEAD is signed (rewriting would silently drop the signature);
/// - the compare-and-swap ref update loses a race (HEAD moved).
///
/// **Ordering.** Run this *after* the landing fix is committed — it
/// stamps whatever HEAD currently is. And run it before the commit is
/// pushed/merged: rewriting changes HEAD's sha.
pub fn stamp_fixes_trailer(repo_root: &Path, slug: &str) -> Result<StampOutcome> {
    if !slug::is_valid(slug) {
        bail!("invalid slug shape: {slug:?}");
    }

    // Resolve HEAD. A failure here means "no commits yet" or "not a git
    // repo" — nothing to stamp. This is the one probe where a non-zero
    // exit is an expected signal rather than a fault.
    let Some(head) = git_probe(repo_root, &["rev-parse", "--verify", "--quiet", "HEAD"])? else {
        return Ok(StampOutcome::Skipped {
            reason: "no commit at HEAD to stamp (empty repo or not a git repo)".into(),
        });
    };

    // Refuse a detached HEAD: `symbolic-ref -q HEAD` fails when HEAD is
    // not a branch. Rewriting a detached HEAD stamps whatever commit is
    // checked out for inspection (bisect, `git checkout <sha>`), which
    // is never the intent.
    if git_probe(repo_root, &["symbolic-ref", "--quiet", "HEAD"])?.is_none() {
        return Ok(StampOutcome::Skipped {
            reason: "HEAD is detached; not rewriting".into(),
        });
    }

    // Bail out of any in-progress sequencer operation: rewriting HEAD
    // mid replay fights the operation and can orphan work. Resolve each
    // marker with `rev-parse --git-path` so worktrees / `.git`-file /
    // external git-dirs resolve correctly instead of a guessed `.git`.
    for marker in [
        "rebase-merge",
        "rebase-apply",
        "CHERRY_PICK_HEAD",
        "MERGE_HEAD",
        "REVERT_HEAD",
    ] {
        let path = git_capture(repo_root, &["rev-parse", "--git-path", marker])?;
        // `--git-path` prints a path relative to git's cwd (repo_root);
        // join so existence is checked there, not the process cwd.
        if repo_root.join(path.trim_end()).exists() {
            return Ok(StampOutcome::Skipped {
                reason: format!("in-progress git operation ({marker}); not rewriting HEAD"),
            });
        }
    }

    // Refuse to rewrite a merge commit: `%P` lists only the parents, so
    // >1 token means 2+ parents. A read failure here is fatal (a safety
    // check must fail closed, not silently treat HEAD as non-merge).
    let parents = git_capture(repo_root, &["show", "-s", "--format=%P", "HEAD"])?;
    let parents: Vec<&str> = parents.split_whitespace().collect();
    if parents.len() > 1 {
        return Ok(StampOutcome::Skipped {
            reason: "HEAD is a merge commit; not rewriting".into(),
        });
    }

    // Refuse to rewrite a signed commit: a rewrite would silently drop
    // the signature (or, worse, block on a pinentry prompt). `%G?` is
    // `N` only when there is no signature.
    let sig = git_capture(repo_root, &["show", "-s", "--format=%G?", "HEAD"])?;
    if sig.trim() != "N" {
        return Ok(StampOutcome::Skipped {
            reason: "HEAD is signed; not rewriting (would drop the signature)".into(),
        });
    }

    // Read the *raw message bytes* (not lossily decoded) so a non-UTF-8
    // commit message round-trips byte-for-byte through the rewrite.
    let msg = git_capture_bytes(repo_root, &["show", "-s", "--format=%B", "HEAD"])?;

    // Already stamped? Parse the message with the same rules the
    // changelog uses (a lossy view is fine — slugs are ASCII) and
    // short-circuit so a re-run is a clean no-op.
    let (_, fixes) = parse_trailers(&String::from_utf8_lossy(&msg));
    if fixes.iter().any(|s| s == slug) {
        return Ok(StampOutcome::AlreadyPresent { sha: head });
    }

    // Build the new message by appending the trailer to the *exact*
    // original bytes. We construct the block ourselves rather than
    // shelling to `interpret-trailers` so the result is guaranteed to
    // satisfy `parse_trailers`' stricter last-paragraph grammar: the
    // trailer goes in its own paragraph unless the last paragraph is
    // already trailer-shaped, in which case it joins that block.
    let mut new_msg = msg.clone();
    while matches!(new_msg.last(), Some(b'\n' | b'\r')) {
        new_msg.pop();
    }
    let last_para_is_trailers = {
        let s = String::from_utf8_lossy(&new_msg);
        last_paragraph_is_trailer_block(&s)
    };
    new_msg.extend_from_slice(if last_para_is_trailers {
        b"\n"
    } else {
        b"\n\n"
    });
    new_msg.extend_from_slice(format!("Fixes-Issue: @{slug}\n").as_bytes());

    // Postcondition: the message we are about to commit must parse back
    // to this slug. If it somehow does not, skip WITHOUT rewriting —
    // never report a `Stamped` the changelog can't see.
    let (_, new_fixes) = parse_trailers(&String::from_utf8_lossy(&new_msg));
    if !new_fixes.iter().any(|s| s == slug) {
        return Ok(StampOutcome::Skipped {
            reason: "constructed message does not parse back to the trailer; not rewriting".into(),
        });
    }

    // Build the replacement commit over HEAD's own tree + parents,
    // preserving author and committer identity/dates. commit-tree
    // otherwise stamps the current user/now, which would rewrite
    // authorship and reorder the commit in date-sorted changelog output.
    let tree = git_capture(repo_root, &["show", "-s", "--format=%T", "HEAD"])?;
    let tree = tree.trim();
    let ident = git_capture(
        repo_root,
        &[
            "show",
            "-s",
            "--format=%an%x00%ae%x00%aI%x00%cn%x00%ce%x00%cI",
            "HEAD",
        ],
    )?;
    let ident: Vec<&str> = ident.trim_end_matches('\n').split('\0').collect();
    if ident.len() != 6 {
        return Ok(StampOutcome::Skipped {
            reason: "could not read HEAD author/committer identity; not rewriting".into(),
        });
    }

    let mut args: Vec<String> = vec!["commit-tree".into(), tree.into()];
    for p in &parents {
        args.push("-p".into());
        args.push((*p).into());
    }
    let new_sha = git_commit_tree(repo_root, &args, &new_msg, &ident)?;

    // Compare-and-swap: move HEAD only if it is still `head`. If a
    // concurrent writer moved HEAD since we read it, this fails and we
    // skip — never clobbering the other write, and never leaving a
    // dangling rewrite (the new object is simply unreferenced).
    let updated = Command::new("git")
        .args([
            "update-ref",
            "-m",
            "issuectl close --stamp",
            "HEAD",
            &new_sha,
            &head,
        ])
        .current_dir(repo_root)
        .output()
        .with_context(|| "running `git update-ref`")?;
    if !updated.status.success() {
        return Ok(StampOutcome::Skipped {
            reason: format!(
                "HEAD moved during stamping; not rewriting ({})",
                String::from_utf8_lossy(&updated.stderr).trim()
            ),
        });
    }

    Ok(StampOutcome::Stamped {
        old: head,
        sha: new_sha,
    })
}

/// True when `msg`'s last paragraph is a valid trailer block by
/// [`parse_trailers`]' rules (every non-blank line is `Token: value` or
/// a `#` comment). Used to decide whether an appended trailer joins the
/// last paragraph or starts a new one.
fn last_paragraph_is_trailer_block(msg: &str) -> bool {
    let trimmed = msg.trim_end_matches(['\n', '\r']);
    let mut block: Vec<&str> = Vec::new();
    for line in trimmed.lines().rev() {
        if line.trim().is_empty() {
            break;
        }
        block.push(line);
    }
    !block.is_empty() && block.iter().all(|l| is_trailer_line(l))
}

/// Run `git <args>`, returning `Some(trimmed stdout)` on success or
/// `None` on a non-zero exit. For probes where a non-zero exit is an
/// *expected* signal (`rev-parse --verify`, `symbolic-ref`), never a
/// fault to surface.
fn git_probe(repo_root: &Path, args: &[&str]) -> Result<Option<String>> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("running `git {}`", args.join(" ")))?;
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

/// Run `git <args>` and return stdout as a `String`, erroring on a
/// non-zero exit. Unlike [`git_probe`], a failure here is a genuine
/// fault: a safety check that cannot read git state must fail closed.
fn git_capture(repo_root: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git_capture_bytes(repo_root, args)?).into_owned())
}

/// Like [`git_capture`] but returns the raw stdout bytes, for payloads
/// (commit messages) that are not guaranteed UTF-8 and must not be
/// lossily rewritten.
fn git_capture_bytes(repo_root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("running `git {}`", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

/// Run `git commit-tree <args>` feeding `message` bytes on stdin, with
/// the six author/committer identity fields (`an ae aI cn ce cI`) set as
/// environment so the new object preserves the original authorship and
/// dates. Returns the new commit sha. The stdin write runs on a helper
/// thread so a large message can't deadlock against git's stdout pipe.
fn git_commit_tree(
    repo_root: &Path,
    args: &[String],
    message: &[u8],
    ident: &[&str],
) -> Result<String> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .env("GIT_AUTHOR_NAME", ident[0])
        .env("GIT_AUTHOR_EMAIL", ident[1])
        .env("GIT_AUTHOR_DATE", ident[2])
        .env("GIT_COMMITTER_NAME", ident[3])
        .env("GIT_COMMITTER_EMAIL", ident[4])
        .env("GIT_COMMITTER_DATE", ident[5])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| "spawning `git commit-tree`")?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let message = message.to_vec();
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        let _ = stdin.write_all(&message);
        // Drop closes the pipe, signalling EOF to git.
    });
    let out = child
        .wait_with_output()
        .with_context(|| "running `git commit-tree`")?;
    let _ = writer.join();
    if !out.status.success() {
        bail!(
            "`git commit-tree` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Map a branch name to a slug if its name *is* a known slug or has
/// the shape `<prefix>/<slug>` or `<prefix>-<slug>` whose tail is a
/// known slug. `known_slugs` is the set to match against — caller
/// loads it from `repo::load_issue_summaries`.
pub fn branch_slug<'a, I>(branch: &str, known_slugs: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let known: std::collections::BTreeSet<&str> = known_slugs.into_iter().collect();
    if known.contains(branch) {
        return Some(branch.to_string());
    }
    // Always reduce to the last path segment first. For
    // `dir-name/feat-foo-bar-baz`, that's `feat-foo-bar-baz`; for
    // `wt-foo-bar-baz` (no slash), `base` is the whole branch.
    // Without this reduction, the `-` peel below would walk across
    // the slash and break out on `slug::is_valid` as soon as it
    // produced a `/`-containing string, never reaching the real
    // slug at the tail.
    let base = branch.rsplit_once('/').map(|(_, t)| t).unwrap_or(branch);
    if known.contains(base) {
        return Some(base.to_string());
    }
    // For `<prefix>-<slug>`, peel one dash-segment at a time off
    // `base`, looking for the longest tail that's a known slug.
    // Stops as soon as the tail is no longer a valid slug shape.
    let mut rest = base;
    while let Some((_, tail)) = rest.split_once('-') {
        if known.contains(tail) {
            return Some(tail.to_string());
        }
        rest = tail;
        if !slug::is_valid(rest) {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_refs_and_fixes_trailers() {
        let body = "subject\n\nbody text here\n\nRefs-Issue: @foo-bar-baz\nFixes-Issue: #qux-quux-corge\nSigned-off-by: a@b\n";
        let (refs, fixes) = parse_trailers(body);
        assert_eq!(refs, vec!["foo-bar-baz".to_string()]);
        assert_eq!(fixes, vec!["qux-quux-corge".to_string()]);
    }

    #[test]
    fn dedupes_within_trailer_block() {
        let body = "x\n\nRefs-Issue: @foo-bar-baz\nRefs-Issue: foo-bar-baz\n";
        let (refs, _) = parse_trailers(body);
        assert_eq!(refs, vec!["foo-bar-baz".to_string()]);
    }

    #[test]
    fn ignores_trailers_in_body_not_in_last_paragraph() {
        // `Refs-Issue:` here is inside the body paragraph, not the
        // trailer block — git's own parser would also ignore it.
        let body = "subject\n\nRefs-Issue: @foo-bar-baz is mentioned\n\nSigned-off-by: a@b\n";
        let (refs, _) = parse_trailers(body);
        assert!(refs.is_empty());
    }

    #[test]
    fn case_insensitive_token() {
        let body = "x\n\nrefs-issue: @foo-bar-baz\nFIXES-ISSUE: qux-quux-corge\n";
        let (refs, fixes) = parse_trailers(body);
        assert_eq!(refs, vec!["foo-bar-baz".to_string()]);
        assert_eq!(fixes, vec!["qux-quux-corge".to_string()]);
    }

    #[test]
    fn rejects_invalid_slug_shape() {
        let body = "x\n\nRefs-Issue: TBD\nRefs-Issue: @valid-slug-here\n";
        let (refs, _) = parse_trailers(body);
        // `TBD` doesn't pass slug validation; `valid-slug-here` does
        // (3-word slugs are typical, but slug::is_valid permits
        // shorter forms used elsewhere — we don't second-guess it).
        assert!(refs.iter().any(|s| s == "valid-slug-here"));
        assert!(!refs.iter().any(|s| s == "TBD"));
    }

    #[test]
    fn empty_body_returns_empty() {
        let (r, f) = parse_trailers("");
        assert!(r.is_empty());
        assert!(f.is_empty());
    }

    #[test]
    fn branch_slug_exact_match() {
        let known = ["foo-bar-baz", "qux-quux-corge"];
        assert_eq!(
            branch_slug("foo-bar-baz", known.iter().copied()),
            Some("foo-bar-baz".to_string())
        );
    }

    #[test]
    fn branch_slug_prefix_slash() {
        let known = ["foo-bar-baz"];
        assert_eq!(
            branch_slug("feat/foo-bar-baz", known.iter().copied()),
            Some("foo-bar-baz".to_string())
        );
    }

    #[test]
    fn branch_slug_prefix_dash() {
        let known = ["foo-bar-baz"];
        assert_eq!(
            branch_slug("wt-foo-bar-baz", known.iter().copied()),
            Some("foo-bar-baz".to_string())
        );
    }

    #[test]
    fn branch_slug_no_match_returns_none() {
        let known = ["foo-bar-baz"];
        assert_eq!(branch_slug("unrelated-branch", known.iter().copied()), None);
    }

    #[test]
    fn branch_slug_dash_in_prefix_segment_still_resolves() {
        // Regression: when the prefix segment before `/` contains
        // its own dash (e.g. `dir-name/feat-foo-bar-baz`), the old
        // dash-peel walked across the slash and broke out before
        // reaching the real slug. After reduction to the last path
        // segment first, the peel finds it.
        let known = ["foo-bar-baz"];
        assert_eq!(
            branch_slug("dir-name/feat-foo-bar-baz", known.iter().copied()),
            Some("foo-bar-baz".to_string())
        );
    }

    #[test]
    fn branch_slug_longest_tail_wins_when_multiple_known() {
        let known = ["bar-baz", "foo-bar-baz"];
        // First dash-peel from `wt-foo-bar-baz` gives `foo-bar-baz`,
        // which is in the set — return immediately (longest known
        // tail, not the shorter `bar-baz`).
        assert_eq!(
            branch_slug("wt-foo-bar-baz", known.iter().copied()),
            Some("foo-bar-baz".to_string())
        );
    }

    #[test]
    fn rejects_path_traversal_slug() {
        // `Refs-Issue: @../../etc/passwd` must be rejected at the
        // parser, not relied on downstream. `slug::is_valid` does
        // the heavy lifting (no `/`, no leading/trailing dashes,
        // no `.` segments).
        let body = "x\n\nRefs-Issue: @../../etc/passwd\n";
        let (refs, _) = parse_trailers(body);
        assert!(refs.is_empty(), "got: {refs:?}");
        let body = "x\n\nRefs-Issue: @foo/bar\n";
        let (refs, _) = parse_trailers(body);
        assert!(refs.is_empty(), "got: {refs:?}");
    }

    // --- stamp_fixes_trailer ---------------------------------------

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?} failed");
    }

    fn fresh_repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-q", "-b", "main"]);
        for (k, v) in [("user.email", "t@example.com"), ("user.name", "t")] {
            git(tmp.path(), &["config", "--local", k, v]);
        }
        tmp
    }

    fn commit(dir: &Path, msg: &str) {
        let data = dir.join("work.txt");
        let prev = std::fs::read_to_string(&data).unwrap_or_default();
        std::fs::write(&data, format!("{prev}{msg}\n")).unwrap();
        git(dir, &["add", "work.txt"]);
        let f = dir.join(".msg");
        std::fs::write(&f, msg).unwrap();
        git(dir, &["commit", "-q", "-F", ".msg"]);
        std::fs::remove_file(&f).ok();
    }

    fn head_body(dir: &Path) -> String {
        show(dir, "%B")
    }

    fn show(dir: &Path, format: &str) -> String {
        let out = Command::new("git")
            .args(["show", "-s", &format!("--format={format}")])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim_end().to_string()
    }

    #[test]
    fn stamp_appends_trailer_in_exact_parse_format() {
        let tmp = fresh_repo();
        let root = tmp.path();
        commit(root, "feat: do the thing\n\nlonger body\n");
        let tree_before = show(root, "%T");

        let outcome = stamp_fixes_trailer(root, "foo-bar-baz").unwrap();
        assert!(
            matches!(outcome, StampOutcome::Stamped { .. }),
            "{outcome:?}"
        );

        // (a) The stamped trailer parses back to exactly this slug via
        // the same path the changelog compiler uses.
        let commits = parse_log(root, "").unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].fixes_issue, vec!["foo-bar-baz".to_string()]);
        assert!(commits[0].refs_issue.is_empty());
        // Byte-level: the trailer line is present verbatim.
        assert!(
            head_body(root).contains("Fixes-Issue: @foo-bar-baz"),
            "body was: {:?}",
            head_body(root)
        );
        // The rewrite is message-only: the tree is untouched.
        assert_eq!(show(root, "%T"), tree_before, "tree must not change");
    }

    #[test]
    fn stamp_into_prose_last_paragraph_still_parses() {
        // Regression: a body whose last paragraph is prose with NO
        // trailing blank line (what `git commit -m` produces). The
        // trailer must land in its OWN paragraph so `parse_trailers`'
        // "last paragraph must be all-trailer" rule still sees it.
        let tmp = fresh_repo();
        let root = tmp.path();
        commit(root, "feat: do the thing\n\nprose tail with no newline");

        let outcome = stamp_fixes_trailer(root, "foo-bar-baz").unwrap();
        assert!(
            matches!(outcome, StampOutcome::Stamped { .. }),
            "{outcome:?}"
        );
        let commits = parse_log(root, "").unwrap();
        assert_eq!(
            commits[0].fixes_issue,
            vec!["foo-bar-baz".to_string()],
            "trailer must be parseable; body was {:?}",
            head_body(root)
        );
    }

    #[test]
    fn stamp_preserves_author_and_dates() {
        let tmp = fresh_repo();
        let root = tmp.path();
        git(root, &["config", "--local", "user.name", "Original Author"]);
        git(
            root,
            &["config", "--local", "user.email", "orig@example.com"],
        );
        // A fixed author date in the past; committer date preserved too.
        std::fs::write(root.join("work.txt"), "x").unwrap();
        git(root, &["add", "work.txt"]);
        git(
            root,
            &[
                "commit",
                "-q",
                "--date=2020-01-02T03:04:05",
                "-m",
                "feat: thing",
            ],
        );
        let (an, ae, ad, cd) = (
            show(root, "%an"),
            show(root, "%ae"),
            show(root, "%aI"),
            show(root, "%cI"),
        );

        stamp_fixes_trailer(root, "foo-bar-baz").unwrap();

        assert_eq!(show(root, "%an"), an, "author name preserved");
        assert_eq!(show(root, "%ae"), ae, "author email preserved");
        assert_eq!(show(root, "%aI"), ad, "author date preserved");
        assert_eq!(show(root, "%cI"), cd, "committer date preserved");
    }

    #[test]
    fn stamp_is_idempotent() {
        let tmp = fresh_repo();
        let root = tmp.path();
        commit(root, "feat: do the thing\n");

        let first = stamp_fixes_trailer(root, "foo-bar-baz").unwrap();
        let StampOutcome::Stamped { sha, .. } = first else {
            panic!("expected Stamped, got {first:?}");
        };
        // Second stamp: trailer already present → no rewrite, sha stable.
        let second = stamp_fixes_trailer(root, "foo-bar-baz").unwrap();
        assert_eq!(second, StampOutcome::AlreadyPresent { sha });
        // Exactly one trailer line, not two.
        let body = head_body(root);
        assert_eq!(body.matches("Fixes-Issue: @foo-bar-baz").count(), 1);
    }

    #[test]
    fn stamp_adds_second_distinct_fixes_trailer() {
        // A commit that fixes two issues: stamping a second, different
        // slug must add it, not silently no-op.
        let tmp = fresh_repo();
        let root = tmp.path();
        commit(root, "feat: do the thing\n");
        stamp_fixes_trailer(root, "foo-bar-baz").unwrap();
        stamp_fixes_trailer(root, "qux-quux-corge").unwrap();
        let commits = parse_log(root, "").unwrap();
        assert_eq!(
            commits[0].fixes_issue,
            vec!["foo-bar-baz".to_string(), "qux-quux-corge".to_string()]
        );
    }

    #[test]
    fn stamp_preserves_existing_trailer_block() {
        let tmp = fresh_repo();
        let root = tmp.path();
        commit(
            root,
            "feat: do the thing\n\nbody\n\nRefs-Issue: @other-slug-here\n",
        );

        stamp_fixes_trailer(root, "foo-bar-baz").unwrap();
        let commits = parse_log(root, "").unwrap();
        // The pre-existing Refs-Issue survives and the new Fixes-Issue
        // joins the same trailer block.
        assert_eq!(commits[0].refs_issue, vec!["other-slug-here".to_string()]);
        assert_eq!(commits[0].fixes_issue, vec!["foo-bar-baz".to_string()]);
    }

    #[test]
    fn stamp_ignores_staged_changes() {
        // Plumbing rewrites HEAD's own tree, so a dirty index is neither
        // folded in nor a reason to skip — it is left exactly as staged.
        let tmp = fresh_repo();
        let root = tmp.path();
        commit(root, "feat: do the thing\n");
        let tree_before = show(root, "%T");
        std::fs::write(root.join("staged.txt"), "content").unwrap();
        git(root, &["add", "staged.txt"]);

        let outcome = stamp_fixes_trailer(root, "foo-bar-baz").unwrap();
        assert!(
            matches!(outcome, StampOutcome::Stamped { .. }),
            "{outcome:?}"
        );
        // HEAD's tree unchanged (staged file not folded)…
        assert_eq!(show(root, "%T"), tree_before);
        // …and the staged file is still staged.
        let staged = Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&staged.stdout).contains("staged.txt"));
    }

    #[test]
    fn stamp_skips_detached_head() {
        let tmp = fresh_repo();
        let root = tmp.path();
        commit(root, "one\n");
        commit(root, "two\n");
        git(root, &["checkout", "-q", "HEAD~1"]); // detached
        let outcome = stamp_fixes_trailer(root, "foo-bar-baz").unwrap();
        assert!(
            matches!(&outcome, StampOutcome::Skipped { reason } if reason.contains("detached")),
            "expected detached skip, got {outcome:?}"
        );
    }

    #[test]
    fn stamp_skips_in_progress_revert() {
        let tmp = fresh_repo();
        let root = tmp.path();
        commit(root, "base\n");
        commit(root, "second\n");
        // Simulate a revert-in-progress by creating the REVERT_HEAD
        // marker directly — robust across git versions. The `--git-path`
        // output is relative to root, so join it.
        let marker = root.join(show_raw(root, &["rev-parse", "--git-path", "REVERT_HEAD"]).trim());
        std::fs::write(&marker, show(root, "%H")).unwrap();
        let outcome = stamp_fixes_trailer(root, "foo-bar-baz").unwrap();
        assert!(
            matches!(&outcome, StampOutcome::Skipped { reason } if reason.contains("REVERT_HEAD")),
            "expected revert skip, got {outcome:?}"
        );
    }

    fn show_raw(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn stamp_skips_merge_commit() {
        let tmp = fresh_repo();
        let root = tmp.path();
        commit(root, "base");
        git(root, &["checkout", "-q", "-b", "side"]);
        // Distinct file per branch so the merge is conflict-free and
        // produces a real two-parent merge commit.
        std::fs::write(root.join("side.txt"), "s").unwrap();
        git(root, &["add", "side.txt"]);
        git(root, &["commit", "-q", "-m", "side work"]);
        git(root, &["checkout", "-q", "main"]);
        std::fs::write(root.join("main.txt"), "m").unwrap();
        git(root, &["add", "main.txt"]);
        git(root, &["commit", "-q", "-m", "main work"]);
        // Force a real merge commit (two parents).
        git(root, &["merge", "-q", "--no-ff", "--no-edit", "side"]);

        let outcome = stamp_fixes_trailer(root, "foo-bar-baz").unwrap();
        assert!(
            matches!(outcome, StampOutcome::Skipped { .. }),
            "expected Skipped for merge commit, got {outcome:?}"
        );
    }

    #[test]
    fn stamp_skips_empty_repo() {
        let tmp = fresh_repo();
        let outcome = stamp_fixes_trailer(tmp.path(), "foo-bar-baz").unwrap();
        assert!(
            matches!(outcome, StampOutcome::Skipped { .. }),
            "expected Skipped for empty repo, got {outcome:?}"
        );
    }

    #[test]
    fn stamp_rejects_invalid_slug() {
        let tmp = fresh_repo();
        commit(tmp.path(), "feat: thing\n");
        assert!(stamp_fixes_trailer(tmp.path(), "../etc/passwd").is_err());
    }

    #[test]
    fn handles_crlf_in_commit_body() {
        // Editor-prepended CRLF line endings used to be a risk —
        // confirm `\r` is trimmed and the slug value isn't
        // suffixed with `\r`.
        let body = "subject\r\n\r\nbody\r\n\r\nRefs-Issue: @foo-bar-baz\r\n";
        let (refs, _) = parse_trailers(body);
        assert_eq!(refs, vec!["foo-bar-baz".to_string()]);
    }
}
