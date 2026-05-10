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
        let stripped = raw.strip_prefix('@').or_else(|| raw.strip_prefix('#')).unwrap_or(raw);
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
    if !token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
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
