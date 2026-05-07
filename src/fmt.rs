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
//!   the first body line; trailing whitespace stripped; final newline;
//! - markdown setext (`===`/`---`) headings rewritten to ATX (`#`/`##`);
//! - YAML quoting: whatever `serde_yaml` emits (minimum needed) plus
//!   `flowify_string_arrays` from [`crate::write`] for readability.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_yaml::{Mapping, Value};

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
    // Reuse `read_item`'s split semantics so blank-line conventions are
    // shared with the rest of the writer.
    let (fm_text, body) = split_for_fmt(text);
    let mut frontmatter: Mapping = match fm_text {
        Some(yaml) if !yaml.trim().is_empty() => serde_yaml::from_str(yaml)
            .context("cannot parse frontmatter")?,
        _ => Mapping::new(),
    };
    reorder_frontmatter(&mut frontmatter);
    sort_array_fields(&mut frontmatter);

    let normalised_body = normalise_body(body);
    let item = ItemFile {
        frontmatter,
        body: normalised_body,
    };
    write::serialize_item(&item)
}

/// Split text into (frontmatter_yaml, body_after_close). Body retains
/// any leading newline that followed the closing `---`; `normalise_body`
/// owns the blank-line policy.
fn split_for_fmt(text: &str) -> (Option<&str>, &str) {
    let trimmed = text.trim_start();
    if !trimmed.starts_with("---") {
        return (None, text);
    }
    let rest = &trimmed[3..];
    if let Some(end) = rest.find("\n---") {
        let yaml = &rest[..end];
        let mut after = end + 4;
        if rest.as_bytes().get(after) == Some(&b'\n') {
            after += 1;
        } else if rest.as_bytes().get(after) == Some(&b'\r')
            && rest.as_bytes().get(after + 1) == Some(&b'\n')
        {
            after += 2;
        }
        (Some(yaml), &rest[after..])
    } else {
        (None, text)
    }
}

fn reorder_frontmatter(map: &mut Mapping) {
    let mut taken: Vec<(Value, Value)> = Vec::with_capacity(map.len());
    for key in CANONICAL_FRONTMATTER_KEYS {
        let k = Value::String((*key).to_string());
        if let Some(v) = map.remove(&k) {
            taken.push((k, v));
        }
    }
    // Whatever remains is unknown; sort alphabetically by string key
    // (non-string keys retain insertion order at the very end).
    let mut remaining: Vec<(Value, Value)> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    remaining.sort_by(|a, b| match (a.0.as_str(), b.0.as_str()) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    map.clear();
    for (k, v) in taken {
        map.insert(k, v);
    }
    for (k, v) in remaining {
        map.insert(k, v);
    }
}

fn sort_array_fields(map: &mut Mapping) {
    for key in SORTED_ARRAY_FIELDS {
        let k = Value::String((*key).to_string());
        if let Some(Value::Sequence(seq)) = map.get_mut(&k) {
            // Stable sort by string form. Non-string entries (shouldn't
            // happen for these fields) keep their relative position via
            // a stable comparator.
            seq.sort_by(|a, b| {
                let sa = a.as_str().unwrap_or("");
                let sb = b.as_str().unwrap_or("");
                sa.cmp(sb)
            });
            seq.dedup_by(|a, b| match (a.as_str(), b.as_str()) {
                (Some(x), Some(y)) => x == y,
                _ => false,
            });
        }
    }
}

/// Body normalisation: setext→ATX, strip trailing whitespace, exactly
/// one blank line between `---` and first non-blank line, single
/// trailing newline. The output starts with `\n` so `serialize_item`'s
/// "---\n" + body produces "---\n\n<line>" — the blank-line policy.
fn normalise_body(body: &str) -> String {
    // Convert CRLF → LF so line iteration is uniform.
    let lf = body.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = lf.split('\n').collect();
    let mut atx: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let stripped = line.trim_end();
        // Setext heading detection: a non-blank line followed by a line
        // of `=` (h1) or `-` (h2). Only trigger when the underline is
        // pure repetition of a single char so we don't misclassify
        // `---` separators or `--foo` content.
        if !stripped.trim().is_empty() && i + 1 < lines.len() {
            let underline = lines[i + 1].trim_end();
            if is_setext_underline(underline, '=') {
                atx.push(format!("# {}", stripped.trim()));
                i += 2;
                continue;
            }
            if is_setext_underline(underline, '-') {
                atx.push(format!("## {}", stripped.trim()));
                i += 2;
                continue;
            }
        }
        atx.push(stripped.to_string());
        i += 1;
    }

    // Collapse leading blank lines; we re-prepend exactly one below.
    while atx.first().map(|s| s.is_empty()).unwrap_or(false) {
        atx.remove(0);
    }
    // Strip trailing blank lines so the final-newline pass is canonical.
    while atx.last().map(|s| s.is_empty()).unwrap_or(false) {
        atx.pop();
    }

    if atx.is_empty() {
        // Empty body: no leading blank line, no content, just a single
        // trailing newline (provided by serialize_item).
        return String::new();
    }

    let mut out = String::with_capacity(lf.len() + 2);
    // Leading newline → blank line between `---` and first content line.
    out.push('\n');
    for (idx, line) in atx.iter().enumerate() {
        out.push_str(line);
        if idx + 1 < atx.len() {
            out.push('\n');
        }
    }
    // serialize_item appends a final '\n' if missing — leave that to it.
    out
}

fn is_setext_underline(s: &str, c: char) -> bool {
    !s.is_empty()
        && s.chars().count() >= 2
        && s.chars().all(|ch| ch == c)
}

// ── File-level operations ───────────────────────────────────────────────

/// Format a single file. Writing is atomic via `write_item_atomic` so
/// `fmt` shares the partial-write protection with the rest of the
/// mutation surface.
pub fn format_file(path: &Path, mode: FormatMode) -> Result<FormatResult> {
    let original = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let formatted = format_text(&original)?;
    if formatted == original {
        return Ok(FormatResult {
            path: path.to_path_buf(),
            status: FormatStatus::Unchanged,
            diff: None,
        });
    }
    let diff = match mode {
        FormatMode::Diff => Some(unified_diff(&original, &formatted, path)),
        _ => None,
    };
    if mode == FormatMode::Write {
        // Reuse the atomic writer so a partial fmt run cannot corrupt
        // item.md mid-rewrite. Re-parse formatted text into ItemFile
        // because `write_item_atomic` takes the structured form.
        let (fm_text, body) = split_for_fmt(&formatted);
        let frontmatter: Mapping = match fm_text {
            Some(yaml) if !yaml.trim().is_empty() => serde_yaml::from_str(yaml)
                .context("internal: re-parse of formatted YAML failed")?,
            _ => Mapping::new(),
        };
        let item = ItemFile {
            frontmatter,
            body: body.to_string(),
        };
        write_item_atomic(path, &item)?;
    }
    Ok(FormatResult {
        path: path.to_path_buf(),
        status: FormatStatus::Changed,
        diff,
    })
}

/// Format every `issues/<slug>/item.md`, or only the supplied slugs.
/// Holds the repo `flock` for the entire run so concurrent mutate
/// writes do not race fmt's read-then-write cycle.
pub fn format_repo(
    root: &Path,
    slugs: &[String],
    mode: FormatMode,
) -> Result<Vec<FormatResult>> {
    let _lock = WriteLock::acquire(root)?;
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

/// Minimal unified-diff. Avoids pulling in a diff crate just for `--diff`.
/// Format is human-oriented; not intended to be machine-parseable.
fn unified_diff(before: &str, after: &str, path: &Path) -> String {
    let a: Vec<&str> = before.split_inclusive('\n').collect();
    let b: Vec<&str> = after.split_inclusive('\n').collect();
    let mut out = String::new();
    out.push_str(&format!("--- {} (current)\n", path.display()));
    out.push_str(&format!("+++ {} (formatted)\n", path.display()));
    // Naive: emit all '-' then all '+'. Good enough for review of small
    // formatting churn; not a full Myers diff.
    for line in &a {
        out.push('-');
        out.push_str(line);
        if !line.ends_with('\n') {
            out.push('\n');
        }
    }
    for line in &b {
        out.push('+');
        out.push_str(line);
        if !line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
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
        let input = "---\nstatus: open\ncreated: 2026-01-01\ntype: bug\npriority: normal\n---\n\n# T\n";
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
        let input = "---\ntype: bug\nstatus: open\npriority: normal\nzeta: 1\nalpha: 2\n---\n\n# T\n";
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
        // An item with no body must round-trip identically. The exact
        // trailing-newline shape is whatever `serialize_item` emits;
        // we only require idempotence (no churn on re-fmt).
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
    fn setext_h2_to_atx() {
        let input = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T\n\nSection\n-------\n\nBody\n";
        let out = format_text(input).unwrap();
        assert!(out.contains("## Section"));
    }

    #[test]
    fn strips_trailing_whitespace() {
        let input = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T   \n\nBody\t\n";
        let out = format_text(input).unwrap();
        assert!(!out.contains("T   "));
        assert!(!out.contains("Body\t"));
    }

    #[test]
    fn ensures_final_newline() {
        let input = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T";
        let out = format_text(input).unwrap();
        assert!(out.ends_with('\n'));
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
        // First normalize it.
        std::fs::write(&path, "---\nstatus: open\ntype: bug\n---\n\n# T\n").unwrap();
        format_file(&path, FormatMode::Write).unwrap();
        // Second pass: must be a no-op.
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
        std::fs::create_dir_all(issues.join("flat-thing-here")).unwrap();
        std::fs::write(
            issues.join("flat-thing-here/item.md"),
            "---\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let results = format_repo(tmp.path(), &[], FormatMode::Check).unwrap();
        // Only the flat-layout issue should be visited.
        assert_eq!(results.len(), 1);
        assert!(results[0].path.to_string_lossy().contains("flat-thing-here"));
    }
}
