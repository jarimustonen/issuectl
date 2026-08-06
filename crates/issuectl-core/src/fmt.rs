//! `issuectl fmt` — normalize `item.md` files. Idempotent.
//!
//! Reduces YAML churn so reviews focus on real changes (especially
//! agent edits and web mutations from `web-edit-sync`). The companion
//! merge driver in `merge_driver.rs` runs the same normalisation on
//! merged output so both formatted and merged files share one canonical
//! form.
//!
//! Normalisation rules (see [`CANONICAL_FRONTMATTER_KEYS`] and the body
//! pipeline in [`format_text`]):
//! - frontmatter key order: fixed canonical order; unknown keys
//!   alphabetically appended;
//! - array fields `labels`, `related`, `blocked_by` sorted; `commits`
//!   keeps order (chronological);
//! - blank-line policy: exactly one blank line between `---` close and
//!   the first body line; trailing whitespace stripped *outside code
//!   blocks*; markdown hard-breaks (`  \n`) preserved; final newline;
//! - markdown setext (`===`/`---`) headings rewritten to ATX (`#`/`##`)
//!   only when the underline is a real heading (preceded by a paragraph
//!   line, not inside a code fence, and the H2 case is disambiguated
//!   from a thematic break by requiring a blank/start line above);
//! - YAML quoting: whatever `serde_yaml` emits (minimum needed) plus
//!   `flowify_string_arrays` from [`crate::write`] for readability.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_yaml::{Mapping, Value};

use crate::item_text;
use crate::mutate::{write_item_atomic, WriteLock};
use crate::write::{self, ItemFile};

/// Canonical key order. Unknown keys are appended alphabetically after
/// these so user-added frontmatter is preserved without losing
/// determinism. Mirrors the spirit of `canonical::canonical_frontmatter_value`
/// — that function sorts alphabetically because it only feeds the hash;
/// `fmt` writes for humans, so the order is curated.
pub const CANONICAL_FRONTMATTER_KEYS: &[&str] = &[
    "created",
    "updated",
    "closed",
    "closed_by",
    "type",
    "status",
    "priority",
    "reporter",
    "assignee",
    "owner",
    "epic",
    "blocked_by",
    "related",
    "labels",
    "commits",
];

/// Array fields that are sorted deterministically. `commits` is NOT in
/// this list — its order is chronological (caller-meaningful).
const SORTED_ARRAY_FIELDS: &[&str] = &["labels", "related", "blocked_by"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatMode {
    /// Rewrite the file in place if changed.
    Write,
    /// Don't write — only report whether the file would change.
    Check,
    /// Don't write — emit a unified diff to stdout for changed files.
    Diff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatStatus {
    /// File was already formatted; no change needed.
    Unchanged,
    /// File was reformatted (mode=Write) or would be (mode=Check/Diff).
    Changed,
}

#[derive(Debug)]
pub struct FormatResult {
    pub path: PathBuf,
    pub status: FormatStatus,
    pub diff: Option<String>,
}

/// Normalise raw item.md text. Pure function — no I/O. The merge driver
/// calls this on its merged output so a single round-trip suffices.
pub fn format_text(text: &str) -> Result<String> {
    let (frontmatter, body) = parse_into_item(text)?;
    let item = ItemFile {
        frontmatter,
        body: normalise_body(body),
    };
    write::serialize_item(&item)
}

/// Parse text into (frontmatter Mapping, body str). Shared with
/// `format_file` so the write path doesn't double-parse.
fn parse_into_item(text: &str) -> Result<(Mapping, &str)> {
    let split = item_text::split(text);
    let frontmatter: Mapping = match split.frontmatter {
        Some(yaml) if !yaml.trim().is_empty() => {
            serde_yaml::from_str(yaml).context("cannot parse frontmatter")?
        }
        _ => Mapping::new(),
    };
    Ok((reorder_and_sort(frontmatter), split.body))
}

fn reorder_and_sort(mut map: Mapping) -> Mapping {
    reorder_frontmatter(&mut map);
    sort_array_fields(&mut map);
    map
}

fn reorder_frontmatter(map: &mut Mapping) {
    let mut taken: Vec<(Value, Value)> = Vec::with_capacity(map.len());
    for key in CANONICAL_FRONTMATTER_KEYS {
        let k = Value::String((*key).to_string());
        if let Some(v) = map.remove(&k) {
            taken.push((k, v));
        }
    }
    // Whatever remains is unknown. Sort string-keyed remainders
    // alphabetically; non-string keys (rare but valid YAML) keep their
    // insertion order at the very tail so we don't silently drop them.
    let original = std::mem::take(map);
    let mut string_remaining: Vec<(Value, Value)> = Vec::new();
    let mut nonstring_remaining: Vec<(Value, Value)> = Vec::new();
    for (k, v) in original {
        if k.as_str().is_some() {
            string_remaining.push((k, v));
        } else {
            nonstring_remaining.push((k, v));
        }
    }
    string_remaining.sort_by(|a, b| a.0.as_str().unwrap_or("").cmp(b.0.as_str().unwrap_or("")));

    for (k, v) in taken {
        map.insert(k, v);
    }
    for (k, v) in string_remaining {
        map.insert(k, v);
    }
    for (k, v) in nonstring_remaining {
        map.insert(k, v);
    }
}

fn sort_array_fields(map: &mut Mapping) {
    for key in SORTED_ARRAY_FIELDS {
        let k = Value::String((*key).to_string());
        if let Some(Value::Sequence(seq)) = map.get_mut(&k) {
            // Sort by string form; entries that aren't strings are
            // partitioned to the tail so they don't get reshuffled
            // across string entries (a stable comparator that maps
            // non-strings to "" would reorder them past strings).
            let (mut strings, others): (Vec<Value>, Vec<Value>) = std::mem::take(seq)
                .into_iter()
                .partition(|v| v.as_str().is_some());
            strings.sort_by(|a, b| a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or("")));
            strings.dedup_by(|a, b| match (a.as_str(), b.as_str()) {
                (Some(x), Some(y)) => x == y,
                _ => false,
            });
            strings.extend(others);
            *seq = strings;
        }
    }
}

/// Body normalisation: setext→ATX (only outside code fences, only for
/// real headings), strip trailing whitespace outside code blocks while
/// preserving markdown hard-breaks (`  \n`), exactly one blank line
/// between `---` and first non-blank line, single trailing newline.
///
/// The output starts with `\n` so `serialize_item`'s "---\n" + body
/// produces "---\n\n<line>" — the blank-line policy.
fn normalise_body(body: &str) -> String {
    let lf = body.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = lf.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());

    let mut i = 0;
    let mut fence: Option<String> = None; // current code-fence marker
    let mut prev_blank_outside_fence = true; // start-of-doc counts as blank

    while i < lines.len() {
        let line = lines[i];

        // Code-fence detection (only outside an existing fence, or a
        // matching close inside one).
        if let Some(marker) = detect_fence_marker(line) {
            if let Some(open) = &fence {
                if marker == *open {
                    fence = None;
                }
            } else {
                fence = Some(marker);
            }
            // Inside fences, preserve content verbatim — including
            // trailing whitespace which can be semantic.
            out.push(line.to_string());
            prev_blank_outside_fence = false;
            i += 1;
            continue;
        }

        if fence.is_some() {
            // Verbatim line inside a fence.
            out.push(line.to_string());
            i += 1;
            continue;
        }

        let stripped = strip_trailing_preserving_hard_break(line);
        let is_blank = stripped.trim().is_empty();

        // Setext detection: a non-blank line followed by a line of pure
        // `=` (h1) or `-` (h2). For h2, also require the previous line
        // to be blank/BOF — otherwise a `---` is a thematic break, not
        // a setext underline.
        if !is_blank && i + 1 < lines.len() {
            let next = lines[i + 1].trim_end();
            if is_setext_underline(next, '=') {
                out.push(format!("# {}", stripped.trim()));
                prev_blank_outside_fence = false;
                i += 2;
                continue;
            }
            if is_setext_underline(next, '-') && prev_blank_outside_fence {
                out.push(format!("## {}", stripped.trim()));
                prev_blank_outside_fence = false;
                i += 2;
                continue;
            }
        }

        out.push(stripped);
        prev_blank_outside_fence = is_blank;
        i += 1;
    }

    // Collapse leading blank lines; we re-prepend exactly one below.
    let first_nonblank = out
        .iter()
        .position(|s| !s.trim().is_empty())
        .unwrap_or(out.len());
    // Strip trailing blank lines so the final-newline pass is canonical.
    let last_nonblank = out.iter().rposition(|s| !s.trim().is_empty());
    let trimmed: &[String] = match last_nonblank {
        Some(last) => &out[first_nonblank..=last],
        None => &[],
    };

    if trimmed.is_empty() {
        return String::new();
    }

    let mut result = String::with_capacity(lf.len() + 2);
    // Leading newline → blank line between `---` and first content line.
    result.push('\n');
    for (idx, line) in trimmed.iter().enumerate() {
        result.push_str(line);
        if idx + 1 < trimmed.len() {
            result.push('\n');
        }
    }
    // serialize_item appends a final '\n' if missing — leave that to it.
    result
}

/// Recognise a markdown fenced-code-block delimiter. Returns the marker
/// string if `line` opens or closes a fence, else `None`. Markers must
/// be ≥3 backticks or tildes; a closing fence must repeat the same char
/// at least the same length — we approximate by storing the char + len.
fn detect_fence_marker(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let leading = line.len() - trimmed.len();
    if leading >= 4 {
        // 4+ leading spaces = indented code block, not a fenced one.
        return None;
    }
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let run: usize = trimmed.chars().take_while(|c| *c == first).count();
    if run < 3 {
        return None;
    }
    Some(format!("{}", first.to_string().repeat(run)))
}

fn is_setext_underline(s: &str, c: char) -> bool {
    !s.is_empty() && s.chars().count() >= 2 && s.chars().all(|ch| ch == c)
}

/// Strip trailing whitespace from a line, but preserve a trailing pair
/// of spaces (markdown's "two-space hard-break" rule). Also preserves
/// the line if its only content is whitespace (don't promote a
/// "whitespace-only" line — caller handles blank-line policy).
fn strip_trailing_preserving_hard_break(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut end = bytes.len();
    while end > 0 {
        let b = bytes[end - 1];
        if b == b' ' || b == b'\t' {
            end -= 1;
        } else {
            break;
        }
    }
    let trailing = bytes.len() - end;
    if end == 0 {
        // Line is whitespace-only; return empty so blank-line policy
        // collapses it. Hard-break preservation only matters for
        // continuation lines.
        return String::new();
    }
    if trailing >= 2 && bytes.get(end).copied() == Some(b' ') {
        // Preserve exactly two spaces (markdown hard break).
        let mut s = line[..end].to_string();
        s.push_str("  ");
        s
    } else {
        line[..end].to_string()
    }
}

// ── File-level operations ───────────────────────────────────────────────

/// Format a single file. Writing is atomic via `write_item_atomic` so
/// `fmt` shares the partial-write protection with the rest of the
/// mutation surface.
pub fn format_file(path: &Path, mode: FormatMode) -> Result<FormatResult> {
    let original =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    // Refuse to format files with unresolved git conflict markers —
    // the line-by-line normaliser would happily eat the `=======`
    // separator (treating it as a setext underline) and leave the file
    // unrecoverable.
    if has_conflict_markers(&original) {
        anyhow::bail!(
            "{}: file has unresolved git conflict markers — resolve before formatting",
            path.display()
        );
    }
    let formatted = format_text(&original)?;
    if formatted == original {
        return Ok(FormatResult {
            path: path.to_path_buf(),
            status: FormatStatus::Unchanged,
            diff: None,
        });
    }
    let diff = if mode == FormatMode::Diff {
        Some(unified_diff(&original, &formatted, path))
    } else {
        None
    };
    if mode == FormatMode::Write {
        // Use the parsed Mapping + body directly to avoid re-parsing
        // our own output. `write_item_atomic` reuses the same
        // serialisation path so on-disk bytes match `formatted`.
        let (frontmatter, body) = parse_into_item(&original)?;
        let item = ItemFile {
            frontmatter,
            body: normalise_body(body),
        };
        write_item_atomic(path, &item)?;
    }
    Ok(FormatResult {
        path: path.to_path_buf(),
        status: FormatStatus::Changed,
        diff,
    })
}

fn has_conflict_markers(text: &str) -> bool {
    // `<<<<<<<` at line start is unambiguous — `=======` and `>>>>>>>`
    // would false-positive on setext underlines and bash heredoc
    // separators inside code fences. The leading marker only appears
    // in genuine git conflicts.
    text.lines().any(|l| l.starts_with("<<<<<<<"))
}

/// Format every `issues/<slug>/item.md`, or only the supplied slugs.
/// Holds the repo `flock` for the entire run when writing so concurrent
/// mutate writes do not race fmt's read-then-write cycle. Read-only
/// modes (`Check`/`Diff`) do NOT acquire the lock — they're advisory
/// snapshots and the lock acquisition would write `.issuectl/write.lock`
/// to the working tree, which breaks read-only checkouts and CI
/// volumes.
pub fn format_repo(root: &Path, slugs: &[String], mode: FormatMode) -> Result<Vec<FormatResult>> {
    let _lock = if mode == FormatMode::Write {
        Some(WriteLock::acquire(root)?)
    } else {
        None
    };
    let issues = root.join("issues");
    let mut results = Vec::new();
    let mut targets: Vec<PathBuf> = Vec::new();
    if slugs.is_empty() {
        // Walk every direct child of `issues/` that contains `item.md`.
        // Skip legacy `open/` and `closed/` parents — those are
        // pre-flat-layout and should be migrated before fmt'ing.
        if let Ok(rd) = std::fs::read_dir(&issues) {
            for entry in rd.flatten() {
                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "open" || name == "closed" {
                    continue;
                }
                let item = entry.path().join("item.md");
                if item.is_file() {
                    targets.push(item);
                }
            }
        }
        targets.sort();
    } else {
        for slug in slugs {
            if !crate::slug::is_valid(slug) {
                anyhow::bail!("invalid slug shape: {slug:?}");
            }
            targets.push(issues.join(slug).join("item.md"));
        }
    }
    for path in targets {
        results.push(format_file(&path, mode)?);
    }
    Ok(results)
}

/// Real unified diff via the `similar` crate. Used by `--diff` mode and
/// for the JSON output of changed files when `--json --diff` is set.
fn unified_diff(before: &str, after: &str, path: &Path) -> String {
    use similar::TextDiff;
    let diff = TextDiff::from_lines(before, after);
    diff.unified_diff()
        .header(
            &format!("{} (current)", path.display()),
            &format!("{} (formatted)", path.display()),
        )
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_is_idempotent() {
        let input = "---\nstatus: open\ntype: bug\ncreated: 2026-01-01\n---\n# Title\n\nBody\n";
        let once = format_text(input).unwrap();
        let twice = format_text(&once).unwrap();
        assert_eq!(once, twice, "fmt must be idempotent");
    }

    #[test]
    fn reorders_to_canonical_order() {
        let input =
            "---\nstatus: open\ncreated: 2026-01-01\ntype: bug\npriority: normal\n---\n\n# T\n";
        let out = format_text(input).unwrap();
        let pos_created = out.find("created:").unwrap();
        let pos_type = out.find("type:").unwrap();
        let pos_status = out.find("status:").unwrap();
        let pos_priority = out.find("priority:").unwrap();
        assert!(pos_created < pos_type);
        assert!(pos_type < pos_status);
        assert!(pos_status < pos_priority);
    }

    #[test]
    fn sorts_labels_alphabetically() {
        let input = "---\ntype: bug\nstatus: open\npriority: normal\nlabels: [zeta, alpha, mu]\n---\n\n# T\n";
        let out = format_text(input).unwrap();
        assert!(out.contains("labels: [alpha, mu, zeta]"));
    }

    #[test]
    fn dedupes_label_arrays() {
        let input = "---\nlabels:\n- foo\n- foo\n- bar\n---\n\n# T\n";
        let out = format_text(input).unwrap();
        assert!(out.contains("labels: [bar, foo]"));
    }

    #[test]
    fn commits_order_preserved() {
        let input = "---\ntype: bug\nstatus: open\npriority: normal\ncommits:\n- hash: zzz\n  summary: late\n- hash: aaa\n  summary: early\n---\n\n# T\n";
        let out = format_text(input).unwrap();
        let pos_zzz = out.find("zzz").unwrap();
        let pos_aaa = out.find("aaa").unwrap();
        assert!(pos_zzz < pos_aaa, "commits must keep chronological order");
    }

    #[test]
    fn unknown_keys_appended_alphabetically() {
        let input =
            "---\ntype: bug\nstatus: open\npriority: normal\nzeta: 1\nalpha: 2\n---\n\n# T\n";
        let out = format_text(input).unwrap();
        let pos_alpha = out.find("alpha:").unwrap();
        let pos_zeta = out.find("zeta:").unwrap();
        let pos_priority = out.find("priority:").unwrap();
        assert!(pos_priority < pos_alpha);
        assert!(pos_alpha < pos_zeta);
    }

    #[test]
    fn enforces_one_blank_line_before_body() {
        let input = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n\n\n# Title\n";
        let out = format_text(input).unwrap();
        assert!(out.contains("---\n\n# Title\n"));
        assert!(!out.contains("---\n\n\n#"));
    }

    #[test]
    fn empty_body_is_idempotent() {
        let input = "---\ntype: bug\nstatus: open\npriority: normal\n---\n";
        let once = format_text(input).unwrap();
        let twice = format_text(&once).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn setext_h1_to_atx() {
        let input = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\nTitle\n=====\n\nBody\n";
        let out = format_text(input).unwrap();
        assert!(out.contains("# Title"));
        assert!(!out.contains("====="));
    }

    #[test]
    fn setext_h2_to_atx_when_preceded_by_blank() {
        let input = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T\n\nSection\n-------\n\nBody\n";
        let out = format_text(input).unwrap();
        assert!(out.contains("## Section"));
    }

    #[test]
    fn setext_h2_underline_after_paragraph_is_thematic_break_not_heading() {
        // Paragraph immediately above `---` (no blank line between
        // paragraph block above the candidate heading line) means
        // the `---` is a thematic break, not a setext underline.
        let input = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\nFirst paragraph.\nSecond line.\n\n---\n\nNext section.\n";
        let out = format_text(input).unwrap();
        assert!(out.contains("---\n"), "thematic break must survive: {out}");
        assert!(!out.contains("## Second line"));
    }

    #[test]
    fn setext_inside_fence_is_preserved() {
        let input =
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n```text\nTitle\n=====\n```\n";
        let out = format_text(input).unwrap();
        assert!(out.contains("Title\n====="), "got: {out}");
        assert!(!out.contains("# Title\n```"));
    }

    #[test]
    fn fenced_code_preserves_trailing_whitespace_and_dashes() {
        let input = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n```text\nx  \n--- separator\n```\n";
        let out = format_text(input).unwrap();
        assert!(out.contains("x  \n"));
        assert!(out.contains("--- separator"));
    }

    #[test]
    fn tilde_fence_also_handled() {
        let input =
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n~~~yaml\nSetext\n=====\n~~~\n";
        let out = format_text(input).unwrap();
        assert!(out.contains("Setext\n====="), "got: {out}");
    }

    #[test]
    fn strips_trailing_whitespace_outside_code() {
        let input = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T   \n\nBody\t\n";
        let out = format_text(input).unwrap();
        assert!(!out.contains("T   "));
        assert!(!out.contains("Body\t"));
    }

    #[test]
    fn preserves_markdown_hard_break_two_spaces() {
        // Two trailing spaces are markdown's hard-break (<br>); fmt
        // must NOT strip them outside code.
        let input = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\nLine one  \nLine two\n";
        let out = format_text(input).unwrap();
        assert!(
            out.contains("Line one  \n"),
            "expected hard-break in: {out:?}"
        );
    }

    #[test]
    fn ensures_final_newline() {
        let input = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T";
        let out = format_text(input).unwrap();
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn refuses_to_format_files_with_conflict_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("item.md");
        std::fs::write(
            &path,
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> theirs\n",
        )
        .unwrap();
        let err = format_file(&path, FormatMode::Check).unwrap_err();
        assert!(err.to_string().contains("conflict markers"));
    }

    #[test]
    fn check_mode_does_not_write() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("item.md");
        let unformatted = "---\nstatus: open\ntype: bug\n---\n\n# T\n";
        std::fs::write(&path, unformatted).unwrap();
        let r = format_file(&path, FormatMode::Check).unwrap();
        assert_eq!(r.status, FormatStatus::Changed);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, unformatted, "check must not modify file");
    }

    #[test]
    fn check_mode_does_not_create_lock_file() {
        // Read-only modes must not write to the repo at all (CI on a
        // read-only checkout, sandboxed eval).
        let tmp = tempfile::tempdir().unwrap();
        let issues = tmp.path().join("issues/some-test-here");
        std::fs::create_dir_all(&issues).unwrap();
        std::fs::write(
            issues.join("item.md"),
            "---\nstatus: open\ntype: bug\n---\n\n# T\n",
        )
        .unwrap();
        let _ = format_repo(tmp.path(), &[], FormatMode::Check).unwrap();
        assert!(
            !tmp.path().join(".issuectl").exists(),
            "check mode must not create .issuectl/"
        );
    }

    #[test]
    fn write_mode_persists_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("item.md");
        let unformatted = "---\nstatus: open\ntype: bug\n---\n\n# T\n";
        std::fs::write(&path, unformatted).unwrap();
        let r = format_file(&path, FormatMode::Write).unwrap();
        assert_eq!(r.status, FormatStatus::Changed);
        let after = std::fs::read_to_string(&path).unwrap();
        let pos_type = after.find("type:").unwrap();
        let pos_status = after.find("status:").unwrap();
        assert!(pos_type < pos_status, "fmt should reorder");
    }

    #[test]
    fn already_formatted_is_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("item.md");
        std::fs::write(&path, "---\nstatus: open\ntype: bug\n---\n\n# T\n").unwrap();
        format_file(&path, FormatMode::Write).unwrap();
        let r2 = format_file(&path, FormatMode::Check).unwrap();
        assert_eq!(r2.status, FormatStatus::Unchanged);
    }

    #[test]
    fn legacy_dirs_are_skipped_in_repo_walk() {
        let tmp = tempfile::tempdir().unwrap();
        let issues = tmp.path().join("issues");
        std::fs::create_dir_all(issues.join("open/legacy-thing-here")).unwrap();
        std::fs::write(
            issues.join("open/legacy-thing-here/item.md"),
            "---\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        std::fs::create_dir_all(issues.join("closed/legacy-closed-here")).unwrap();
        std::fs::write(
            issues.join("closed/legacy-closed-here/item.md"),
            "---\nstatus: fixed\n---\n# T\n",
        )
        .unwrap();
        std::fs::create_dir_all(issues.join("flat-thing-here")).unwrap();
        std::fs::write(
            issues.join("flat-thing-here/item.md"),
            "---\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let results = format_repo(tmp.path(), &[], FormatMode::Check).unwrap();
        // Only the flat-layout issue should be visited.
        assert_eq!(results.len(), 1);
        assert!(results[0]
            .path
            .to_string_lossy()
            .contains("flat-thing-here"));
    }

    #[test]
    fn unified_diff_uses_real_diff_format() {
        let before = "---\nstatus: open\ntype: bug\n---\n\n# T\n";
        let after = format_text(before).unwrap();
        let d = unified_diff(before, &after, std::path::Path::new("item.md"));
        // Real unified diff has @@ hunk headers.
        assert!(d.contains("@@"), "expected unified diff hunks: {d}");
    }
}
