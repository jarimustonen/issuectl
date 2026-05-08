//! Append-only markdown body section conventions.
//!
//! See `docs/design/body-sections.md` for the spec. Sections are H2
//! headings with reserved names; blocks within a section are H3
//! headings carrying a UTC ISO-8601 timestamp and an `@author`.
//!
//! Heading detection is fence-aware: `## …` or `### …` lines inside
//! a fenced code block are treated as content, not as section /
//! block boundaries. Without this, a user pasting a shell or Python
//! snippet with a `## comment` line would have subsequent appends
//! spliced into their code block — silent on-disk corruption.

use anyhow::{bail, Result};
use chrono::{SecondsFormat, Utc};

/// Canonical section name for free-form human/agent comments.
pub const COMMENTS: &str = "Comments";
#[allow(dead_code)]
pub const DECISIONS: &str = "Decisions";
#[allow(dead_code)]
pub const AGENT_RUNS: &str = "Agent Runs";

/// Format the timestamp half of a block heading. UTC, second-precision.
pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Reject author strings that would let the heading shape be
/// fabricated. Headings are single lines; `\n`/`\r` inside an author
/// would mint additional lines. We also disallow `@` (we add the
/// sigil ourselves) and the middle-dot separator we use as the
/// heading delimiter.
pub fn validate_author(author: &str) -> Result<()> {
    let trimmed = author.trim();
    if trimmed.is_empty() {
        bail!("author cannot be empty");
    }
    for ch in trimmed.chars() {
        if ch.is_control() {
            bail!("author cannot contain control characters");
        }
        if ch == '@' || ch == '·' {
            bail!("author cannot contain {:?}", ch);
        }
        if ch.is_whitespace() {
            bail!("author cannot contain whitespace");
        }
    }
    Ok(())
}

/// Reject messages that would inject section / block headings. A
/// legitimate user can still discuss headings by quoting them inside
/// a fenced code block — the parser is fence-aware.
pub fn validate_message(message: &str) -> Result<()> {
    if message.trim().is_empty() {
        bail!("message cannot be empty");
    }
    let mut fence: Option<String> = None;
    for line in message.split('\n') {
        if let Some(marker) = detect_fence_marker(line) {
            match &fence {
                Some(open) if open == &marker => fence = None,
                None => fence = Some(marker),
                _ => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if line.starts_with("## ") || line.starts_with("### ") {
            bail!(
                "message line begins with `## ` or `### ` outside a code fence; \
                 this would break out of the comment block — wrap it in a code fence"
            );
        }
    }
    Ok(())
}

/// Render a block heading + body for a `Comments`-style section.
pub fn render_note_block(ts: &str, author: &str, message: &str) -> String {
    format!(
        "### {ts} · @{author}\n\n{}\n",
        message.trim_end_matches('\n')
    )
}

// ── Heading / fence detection ───────────────────────────────────────────

/// Recognise an opening or closing fenced-code-block delimiter. Same
/// rules `fmt::detect_fence_marker` uses: ≥3 backticks or tildes,
/// fewer than 4 leading spaces. Returns the marker so callers can
/// match opens against closes.
fn detect_fence_marker(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let leading = line.len() - trimmed.len();
    if leading >= 4 {
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
    Some(first.to_string().repeat(run))
}

fn is_h2_named(line: &str, name: &str) -> bool {
    let Some(rest) = line.strip_prefix("## ") else {
        return false;
    };
    rest.trim_end() == name
}

fn is_any_h2(line: &str) -> bool {
    line.starts_with("## ") && !line.starts_with("### ")
}

#[allow(dead_code)]
fn is_h3(line: &str) -> bool {
    line.starts_with("### ") && !line.starts_with("#### ")
}

/// Walk `lines` outside fenced code blocks, calling `f` with the
/// real-line index. Used to locate genuine H2/H3 boundaries while
/// respecting code fences. Returns the indices for which `f`
/// returned true.
fn scan_outside_fences<F: FnMut(usize, &str) -> bool>(
    lines: &[&str],
    mut f: F,
) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut fence: Option<String> = None;
    for (i, l) in lines.iter().enumerate() {
        if let Some(marker) = detect_fence_marker(l) {
            match &fence {
                Some(open) if open == &marker => fence = None,
                None => fence = Some(marker),
                _ => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if f(i, l) {
            hits.push(i);
        }
    }
    hits
}

// ── Public writer API ───────────────────────────────────────────────────

/// Append a block to a named H2 section, creating the section if it
/// doesn't exist. Returns the new body. Heading detection is
/// fence-aware: a `## …` line inside a fenced code block is treated
/// as content, not as a section boundary.
pub fn append_block(body: &str, section: &str, block: &str) -> String {
    let lines: Vec<&str> = body.split('\n').collect();
    let section_idx = scan_outside_fences(&lines, |_, l| is_h2_named(l, section))
        .into_iter()
        .next();

    match section_idx {
        Some(start) => insert_block_in_section(&lines, start, block),
        None => append_new_section(body, section, block),
    }
}

/// Insert `block` at the end of the section starting at `lines[start]`.
/// Section ends at the next H2 line *outside any code fence* or EOF.
/// Trailing blank lines inside the section are collapsed; we add
/// exactly one blank line of separation before the new block.
fn insert_block_in_section(lines: &[&str], start: usize, block: &str) -> String {
    let next_h2 = {
        let mut found = lines.len();
        let mut fence: Option<String> = None;
        for (i, l) in lines.iter().enumerate().skip(start + 1) {
            if let Some(marker) = detect_fence_marker(l) {
                match &fence {
                    Some(open) if open == &marker => fence = None,
                    None => fence = Some(marker),
                    _ => {}
                }
                continue;
            }
            if fence.is_some() {
                continue;
            }
            if is_any_h2(l) {
                found = i;
                break;
            }
        }
        found
    };

    // Find the last non-blank line within (start, next_h2). We splice
    // in immediately after it so the new block sits flush against
    // existing content rather than after a tail of blank lines.
    let mut splice = start + 1;
    for i in (start + 1..next_h2).rev() {
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
    }
    out.push_str("## ");
    out.push_str(section);
    out.push_str("\n\n");
    out.push_str(block.trim_end_matches('\n'));
    out.push('\n');
    out
}

/// Canonicalise the body so `serialize_item` produces a `---\n\n<body>`
/// shape (one blank line between frontmatter close and the first body
/// line). Without this, appending to a legacy file that omitted the
/// blank line leaves the file in a state `issuectl fmt` would still
/// want to change — breaking idempotency under normal use.
pub fn canonicalise_body_leading(body: &str) -> String {
    let stripped = body.trim_start_matches('\n');
    if stripped.is_empty() {
        String::new()
    } else {
        format!("\n{stripped}")
    }
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

// ── Public reader API ───────────────────────────────────────────────────

/// One block parsed out of a section.
#[allow(dead_code)] // sister tickets (`decide`, `agent-run`) consume this
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Raw timestamp text from the H3 heading (caller decides whether
    /// to parse it as RFC-3339).
    pub timestamp: String,
    /// Author identifier without the leading `@`.
    pub author: String,
    /// Block body, leading/trailing blank lines trimmed.
    pub body: String,
}

/// Parse all blocks under the H2 section named `section`. Returns
/// an empty vec if the section is absent. Headings inside fenced
/// code blocks are treated as content (matching the writer).
///
/// Block heading shape: `### <ts> · @<author>`. The middle-dot
/// separator is U+00B7 with single ASCII spaces on each side. Lines
/// that don't match the shape are skipped (with their content folded
/// into the previous block's body) so a partially-malformed history
/// is read forgivingly rather than producing zero results.
#[allow(dead_code)] // sister tickets (`decide`, `agent-run`) consume this
pub fn parse_section(body: &str, section: &str) -> Vec<Block> {
    let lines: Vec<&str> = body.split('\n').collect();
    let Some(start) = scan_outside_fences(&lines, |_, l| is_h2_named(l, section))
        .into_iter()
        .next()
    else {
        return Vec::new();
    };

    let next_h2 = {
        let mut found = lines.len();
        let mut fence: Option<String> = None;
        for (i, l) in lines.iter().enumerate().skip(start + 1) {
            if let Some(marker) = detect_fence_marker(l) {
                match &fence {
                    Some(open) if open == &marker => fence = None,
                    None => fence = Some(marker),
                    _ => {}
                }
                continue;
            }
            if fence.is_some() {
                continue;
            }
            if is_any_h2(l) {
                found = i;
                break;
            }
        }
        found
    };

    let block_starts = {
        let span = &lines[start + 1..next_h2];
        let mut hits = Vec::new();
        let mut fence: Option<String> = None;
        for (i, l) in span.iter().enumerate() {
            if let Some(marker) = detect_fence_marker(l) {
                match &fence {
                    Some(open) if open == &marker => fence = None,
                    None => fence = Some(marker),
                    _ => {}
                }
                continue;
            }
            if fence.is_some() {
                continue;
            }
            if is_h3(l) {
                hits.push(i);
            }
        }
        hits
    };

    let span = &lines[start + 1..next_h2];
    let mut out = Vec::with_capacity(block_starts.len());
    for (idx, &h_idx) in block_starts.iter().enumerate() {
        let end_idx = block_starts.get(idx + 1).copied().unwrap_or(span.len());
        let heading = span[h_idx];
        let Some(parsed) = parse_block_heading(heading) else {
            continue;
        };
        let body_lines = &span[h_idx + 1..end_idx];
        let body_text = trim_blank_borders(body_lines);
        out.push(Block {
            timestamp: parsed.0,
            author: parsed.1,
            body: body_text,
        });
    }
    out
}

#[allow(dead_code)]
fn parse_block_heading(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("### ")?.trim_end();
    // Split on " · " (space + middle-dot + space).
    let (ts, author_with_at) = rest.split_once(" · ")?;
    let author = author_with_at.trim().strip_prefix('@')?;
    if ts.is_empty() || author.is_empty() {
        return None;
    }
    Some((ts.trim().to_string(), author.to_string()))
}

#[allow(dead_code)]
fn trim_blank_borders(lines: &[&str]) -> String {
    let first = lines.iter().position(|l| !l.trim().is_empty());
    let last = lines.iter().rposition(|l| !l.trim().is_empty());
    match (first, last) {
        (Some(a), Some(b)) => lines[a..=b].join("\n"),
        _ => String::new(),
    }
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
        assert_eq!(out.matches("## Comments").count(), 1);
    }

    #[test]
    fn fenced_h2_inside_block_does_not_terminate_section() {
        // Regression for C1 from review-body-sections.md: a `## …`
        // line inside a fenced code block must NOT be treated as a
        // section boundary, otherwise the next append corrupts the
        // user's code.
        let body = "\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @bob\n\n\
            ```bash\n\
            ## this is a bash comment\n\
            echo hi\n\
            ```\n";
        let out = append_block(body, COMMENTS, "### 2026-05-02T00:00:00Z · @alice\n\nsecond\n");
        // Code block stays intact and the new block is appended
        // *after* the bash fence, not inside it.
        let bash_line = out.find("## this is a bash comment").unwrap();
        let new_block = out.find("@alice").unwrap();
        assert!(
            bash_line < new_block,
            "new block must land after the user's code fence, got:\n{out}"
        );
        // The fence is still closed properly: ```\n```\n preserved
        assert!(out.contains("```bash\n## this is a bash comment\necho hi\n```"));
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
    }

    #[test]
    fn append_idempotent_under_fmt() {
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
    fn canonicalise_body_leading_inserts_one_newline() {
        // C3 fix: bodies that came from a no-blank-line legacy file
        // must end up with a single leading `\n` so serialize_item
        // produces `---\n\n<body>` instead of `---\n<body>`.
        assert_eq!(canonicalise_body_leading("# T\n"), "\n# T\n");
        assert_eq!(canonicalise_body_leading("\n# T\n"), "\n# T\n");
        assert_eq!(canonicalise_body_leading("\n\n\n# T\n"), "\n# T\n");
        assert_eq!(canonicalise_body_leading(""), "");
        assert_eq!(canonicalise_body_leading("\n\n"), "");
    }

    // ── validation ──────────────────────────────────────────────────

    #[test]
    fn validate_author_rejects_newlines_and_at() {
        assert!(validate_author("alice").is_ok());
        assert!(validate_author("agent-claude_4-7").is_ok());
        assert!(validate_author("").is_err());
        assert!(validate_author("alice\n## Pwned").is_err());
        assert!(validate_author("@alice").is_err());
        assert!(validate_author("al ice").is_err());
        assert!(validate_author("a·b").is_err());
        assert!(validate_author("alice\rwith-cr").is_err());
    }

    #[test]
    fn validate_message_rejects_unfenced_h2_h3() {
        assert!(validate_message("plain text").is_ok());
        assert!(validate_message("multi\nline\n").is_ok());
        // C2: forging headings outside a fence is rejected.
        assert!(validate_message("normal\n\n## Decisions\n\nfake").is_err());
        assert!(validate_message("### 2020-01-01 · @evil\n\nforged").is_err());
        // Quoting the same content inside a fence is fine — the
        // parser is fence-aware so the content cannot break out.
        assert!(validate_message("see this:\n```\n## bash comment\n```\nok").is_ok());
        assert!(validate_message("").is_err());
    }

    // ── parser ──────────────────────────────────────────────────────

    #[test]
    fn parse_section_returns_blocks_in_order() {
        let body = "\n# T\n\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @bob\n\nfirst note\n\n\
            ### 2026-05-02T00:00:00Z · @alice\n\nsecond note\n\n## Decisions\n\n\
            ### 2026-05-03T00:00:00Z · @cara\n\npicked X\n";
        let blocks = parse_section(body, COMMENTS);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].timestamp, "2026-05-01T00:00:00Z");
        assert_eq!(blocks[0].author, "bob");
        assert_eq!(blocks[0].body, "first note");
        assert_eq!(blocks[1].author, "alice");
        assert_eq!(blocks[1].body, "second note");
        // Decisions parses independently
        let decs = parse_section(body, DECISIONS);
        assert_eq!(decs.len(), 1);
        assert_eq!(decs[0].author, "cara");
    }

    #[test]
    fn parse_section_round_trips_with_append() {
        let body0 = "\n# T\n";
        let block = render_note_block("2026-05-07T12:00:00Z", "alice", "hello world");
        let body1 = append_block(body0, COMMENTS, &block);
        let blocks = parse_section(&body1, COMMENTS);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].timestamp, "2026-05-07T12:00:00Z");
        assert_eq!(blocks[0].author, "alice");
        assert_eq!(blocks[0].body, "hello world");
    }

    #[test]
    fn parse_section_ignores_h3_inside_code_fence() {
        // A user might paste an example block heading inside a code
        // fence — that's content, not a real block.
        let body = "\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @bob\n\n\
            example:\n```\n### 2020-01-01 · @ghost\n```\n";
        let blocks = parse_section(body, COMMENTS);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].author, "bob");
    }

    #[test]
    fn parse_missing_section_returns_empty() {
        let body = "\n# T\n\nNo sections.\n";
        assert!(parse_section(body, COMMENTS).is_empty());
    }

    #[test]
    fn render_note_block_strips_trailing_newlines() {
        let b = render_note_block("2026-05-07T12:00:00Z", "alice", "hello\n\n");
        assert!(b.starts_with("### 2026-05-07T12:00:00Z · @alice\n\n"));
        assert!(b.ends_with("hello\n"));
    }
}
