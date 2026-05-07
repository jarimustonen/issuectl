//! Append-only markdown body section conventions.
//!
//! See `docs/design/body-sections.md` for the spec. Sections are H2
//! headings with reserved names; blocks within a section are H3
//! headings carrying a UTC ISO-8601 timestamp and an `@author`.
//!
//! All operations are append-only and idempotent w.r.t. existing
//! content: appending to a section never reorders earlier blocks, and
//! `issuectl fmt`'s body normaliser leaves the resulting shape alone.

use chrono::{SecondsFormat, Utc};

/// Canonical section name for free-form human/agent comments.
pub const COMMENTS: &str = "Comments";
/// Alias the parser recognises so a body that already has `## Notes`
/// keeps its heading instead of growing a parallel `## Comments`.
pub const NOTES_ALIAS: &str = "Notes";
#[allow(dead_code)]
pub const DECISIONS: &str = "Decisions";
#[allow(dead_code)]
pub const AGENT_RUNS: &str = "Agent Runs";

/// Format the timestamp half of a block heading. UTC, second-precision.
pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Render a block heading + body for a `Comments`-style section.
pub fn render_note_block(ts: &str, author: &str, message: &str) -> String {
    format!(
        "### {ts} · @{author}\n\n{}\n",
        message.trim_end_matches('\n')
    )
}

/// Append a block to a named H2 section, creating the section if it
/// doesn't exist. Returns the new body.
///
/// The `section` is the heading text without the `## ` prefix. If
/// `section == "Comments"` and the body already has a `## Notes`
/// section (the documented alias), the block is appended there
/// instead of growing a parallel `## Comments`.
pub fn append_block(body: &str, section: &str, block: &str) -> String {
    let lines: Vec<&str> = body.split('\n').collect();
    let aliases = section_aliases(section);
    let section_idx = lines
        .iter()
        .position(|l| heading_matches_any(l, &aliases));

    match section_idx {
        Some(start) => insert_block_in_section(&lines, start, block),
        None => append_new_section(body, section, block),
    }
}

fn section_aliases(section: &str) -> Vec<&str> {
    if section == COMMENTS {
        vec![COMMENTS, NOTES_ALIAS]
    } else {
        vec![section]
    }
}

fn heading_matches_any(line: &str, names: &[&str]) -> bool {
    let Some(rest) = line.strip_prefix("## ") else {
        return false;
    };
    let trimmed = rest.trim_end();
    names.iter().any(|n| trimmed == *n)
}

fn is_h2(line: &str) -> bool {
    line.starts_with("## ") && !line.starts_with("### ")
}

/// Insert `block` at the end of the section starting at `lines[start]`.
/// Section ends at the next H2 line or EOF. Trailing blank lines inside
/// the section are preserved; we add exactly one blank line of
/// separation before the new block.
fn insert_block_in_section(lines: &[&str], start: usize, block: &str) -> String {
    let mut end = lines.len();
    for (i, l) in lines.iter().enumerate().skip(start + 1) {
        if is_h2(l) {
            end = i;
            break;
        }
    }
    // Find the last non-blank line within [start, end). We splice in
    // immediately after it so the inserted block sits flush against
    // existing content rather than after a tail of blank lines.
    let mut splice = start + 1;
    for i in (start + 1..end).rev() {
        if !lines[i].trim().is_empty() {
            splice = i + 1;
            break;
        }
    }

    let head = lines[..splice].join("\n");
    let tail_lines = &lines[splice..];

    let mut out = String::with_capacity(head.len() + block.len() + 16);
    out.push_str(&head);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(block.trim_end_matches('\n'));
    out.push('\n');
    if !tail_lines.is_empty() {
        out.push('\n');
        out.push_str(&tail_lines.join("\n"));
    }
    out
}

fn append_new_section(body: &str, section: &str, block: &str) -> String {
    let trimmed = body.trim_end_matches('\n');
    let mut out = String::with_capacity(body.len() + section.len() + block.len() + 16);
    out.push_str(trimmed);
    if !trimmed.is_empty() {
        out.push_str("\n\n");
    } else {
        // Match the read_item leading-newline convention.
        out.push('\n');
    }
    out.push_str("## ");
    out.push_str(section);
    out.push_str("\n\n");
    out.push_str(block.trim_end_matches('\n'));
    out.push('\n');
    out
}

/// Append a `## Reopen Notes — <date>` section to the body. Each
/// reopen event creates its own section so multiple reopens stack
/// chronologically rather than merging into one.
pub fn append_reopen_notes(body: &str, date: &str) -> String {
    let heading = format!("Reopen Notes — {date}");
    let stub = "_Add rationale for reopening here._";
    // Always create a NEW section (do not merge into a same-day prior
    // reopen): each transition is a discrete event.
    append_new_section(body, &heading, stub)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_creates_section_when_missing() {
        let body = "\n# Title\n\n## Description\n\nHello.\n";
        let out = append_block(body, COMMENTS, "### 2026-05-07T12:00:00Z · @alice\n\nhi\n");
        assert!(out.contains("## Comments"));
        assert!(out.contains("@alice"));
        assert!(out.contains("hi"));
        // Existing section preserved
        assert!(out.contains("## Description"));
        assert!(out.contains("Hello."));
    }

    #[test]
    fn append_into_existing_section_keeps_prior_blocks() {
        let body = "\n# T\n\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @bob\n\nfirst\n";
        let out = append_block(body, COMMENTS, "### 2026-05-02T00:00:00Z · @alice\n\nsecond\n");
        let i_first = out.find("first").unwrap();
        let i_second = out.find("second").unwrap();
        assert!(i_first < i_second, "newest must be appended after older");
        assert!(out.matches("## Comments").count() == 1, "no duplicate section");
    }

    #[test]
    fn notes_alias_is_used_when_present() {
        let body = "\n# T\n\n## Notes\n\nlegacy block\n";
        let out = append_block(body, COMMENTS, "### t · @x\n\nnew\n");
        assert!(out.contains("## Notes"));
        assert!(!out.contains("## Comments"));
        assert!(out.contains("legacy block"));
        assert!(out.contains("new"));
    }

    #[test]
    fn unrelated_edits_to_other_sections_are_preserved() {
        let body = "\n# T\n\n## Description\n\nbody text\n\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @bob\n\nfirst\n\n## Decisions\n\n\
            ### 2026-05-02T00:00:00Z · @cara\n\npicked X\n";
        let out = append_block(body, COMMENTS, "### 2026-05-03T00:00:00Z · @alice\n\nsecond\n");
        assert!(out.contains("body text"));
        assert!(out.contains("first"));
        assert!(out.contains("second"));
        assert!(out.contains("picked X"));
        // Section ordering preserved.
        let i_desc = out.find("## Description").unwrap();
        let i_com = out.find("## Comments").unwrap();
        let i_dec = out.find("## Decisions").unwrap();
        assert!(i_desc < i_com && i_com < i_dec);
    }

    #[test]
    fn reopen_notes_stack_into_separate_sections() {
        let body = "\n# T\n\n## Description\n\nx\n";
        let out1 = append_reopen_notes(body, "2026-05-07");
        let out2 = append_reopen_notes(&out1, "2026-05-09");
        assert_eq!(out2.matches("## Reopen Notes — 2026-05-07").count(), 1);
        assert_eq!(out2.matches("## Reopen Notes — 2026-05-09").count(), 1);
        let i1 = out2.find("Reopen Notes — 2026-05-07").unwrap();
        let i2 = out2.find("Reopen Notes — 2026-05-09").unwrap();
        assert!(i1 < i2);
    }

    #[test]
    fn append_idempotent_under_fmt() {
        // Round-trip the result through fmt's body normaliser; the
        // bytes must stabilise after one pass.
        let body = "\n# T\n\n## Description\n\nbody.\n";
        let appended = append_block(body, COMMENTS, "### 2026-05-07T12:00:00Z · @x\n\nhi\n");
        let formatted = crate::fmt::format_text(&format!(
            "---\nstatus: open\n---\n{appended}"
        ))
        .unwrap();
        let formatted_again = crate::fmt::format_text(&formatted).unwrap();
        assert_eq!(formatted, formatted_again, "fmt must be idempotent");
    }

    #[test]
    fn render_note_block_strips_trailing_newlines() {
        let b = render_note_block("2026-05-07T12:00:00Z", "alice", "hello\n\n");
        assert!(b.starts_with("### 2026-05-07T12:00:00Z · @alice\n\n"));
        assert!(b.ends_with("hello\n"));
    }
}
