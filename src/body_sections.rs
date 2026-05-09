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
/// Canonical section for `note --decision` blocks.
pub const DECISIONS: &str = "Decisions";
/// Canonical section for `note --agent-run` blocks.
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
///
/// Round-2 finding O9: previously `trim()` was applied before the
/// whitespace check, so `" alice "` slipped through and rendered as
/// a malformed heading. The author must be canonical on input —
/// callers needing to be permissive should `.trim()` first.
pub fn validate_author(author: &str) -> Result<()> {
    if author.is_empty() {
        bail!("author cannot be empty");
    }
    if author != author.trim() {
        bail!("author cannot have leading or trailing whitespace");
    }
    for ch in author.chars() {
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
/// a fenced code block — the parser is fence-aware. Round-2 finding
/// O10: an unclosed fence would silently swallow later blocks once
/// appended, so we reject those too.
pub fn validate_message(message: &str) -> Result<()> {
    if message.trim().is_empty() {
        bail!("message cannot be empty");
    }
    let lines: Vec<&str> = message.split('\n').collect();
    let mut injected: Option<String> = None;
    let trailing_fence = scan_with_fence_state(&lines, |_, l| {
        if injected.is_some() {
            return;
        }
        if is_any_h2(l) || is_h3(l) {
            injected = Some(l.to_string());
        }
    });
    if let Some(line) = injected {
        bail!(
            "message line {line:?} begins with `## ` or `### ` outside a code fence; \
             this would break out of the comment block — wrap it in a code fence"
        );
    }
    if trailing_fence.is_some() {
        bail!(
            "message contains an unclosed fenced code block; close it before appending \
             so future blocks are not swallowed"
        );
    }
    Ok(())
}

/// Render a block heading + body for a `Comments`-style section.
/// Calls the validators internally so the shape on disk cannot drift
/// from the input contract (round-2 finding O8). Callers that have
/// already validated should pass canonical input — the call is
/// cheap.
pub fn render_note_block(ts: &str, author: &str, message: &str) -> Result<String> {
    validate_author(author)?;
    validate_message(message)?;
    Ok(format!(
        "### {ts} · @{author}\n\n{}\n",
        message.trim_end_matches('\n')
    ))
}

// ── Heading / fence detection ───────────────────────────────────────────
//
// CommonMark fence rules we honour:
//   - opening fence: ≥3 backticks or tildes, <4 leading spaces;
//   - closing fence: same fence char, length ≥ opener, <4 leading
//     spaces, and only whitespace after the fence run.
//
// The whole-line state machine lives in `mark_fence_state` so every
// caller (writer, reader, doctor migration, validators) shares one
// implementation. Splitting opener-detection from close-matching is
// what fixes round-2 finding G3/O2: a `` ```` `` close after a
// `` ``` `` open used to leave the fence dangling.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fence {
    ch: char,
    len: usize,
}

pub(crate) fn opening_fence(line: &str) -> Option<Fence> {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    if indent >= 4 {
        return None;
    }
    let ch = trimmed.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|c| *c == ch).count();
    if len < 3 {
        return None;
    }
    Some(Fence { ch, len })
}

pub(crate) fn closes_fence(line: &str, open: Fence) -> bool {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    if indent >= 4 {
        return false;
    }
    let mut chars = trimmed.chars();
    if chars.next() != Some(open.ch) {
        return false;
    }
    let run = trimmed.chars().take_while(|c| *c == open.ch).count();
    if run < open.len {
        return false;
    }
    let after = &trimmed[open.ch.len_utf8() * run..];
    after.trim().is_empty()
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

fn is_h3(line: &str) -> bool {
    line.starts_with("### ") && !line.starts_with("#### ")
}

/// Iterate body lines, yielding `(line_index, line)` for every line
/// that lies outside a fenced code block. Used by `mutate.rs` to make
/// the new `check` checkbox-toggle verb fence-aware so it doesn't
/// silently mutate documentation snippets like `- [ ] example`
/// inside a markdown code fence.
pub(crate) fn lines_outside_fences(body: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = body.split('\n').collect();
    let mut out = Vec::new();
    scan_outside_fences(&lines, |i, l| {
        out.push((i, l.to_string()));
        false
    });
    out
}

/// Walk `lines` and call `f(i, line)` for every line that lies
/// outside a fenced code block. Returns indices where `f` returned
/// true. Single source of truth for fence-aware scanning — every
/// other function in this module routes through here.
fn scan_outside_fences<F: FnMut(usize, &str) -> bool>(
    lines: &[&str],
    mut f: F,
) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut fence: Option<Fence> = None;
    for (i, l) in lines.iter().enumerate() {
        match fence {
            Some(open) if closes_fence(l, open) => fence = None,
            Some(_) => {}
            None => {
                if let Some(o) = opening_fence(l) {
                    fence = Some(o);
                } else if f(i, l) {
                    hits.push(i);
                }
            }
        }
    }
    hits
}

/// Like `scan_outside_fences` but reports whether a fence is still
/// open at EOF — used by `validate_message` to reject unclosed
/// fences (round-2 finding O10) and by callers that need to check
/// "the section truly ends at EOF" rather than "got swallowed by an
/// unclosed fence".
fn scan_with_fence_state<F: FnMut(usize, &str)>(lines: &[&str], mut f: F) -> Option<Fence> {
    let mut fence: Option<Fence> = None;
    for (i, l) in lines.iter().enumerate() {
        match fence {
            Some(open) if closes_fence(l, open) => fence = None,
            Some(_) => {}
            None => {
                if let Some(o) = opening_fence(l) {
                    fence = Some(o);
                } else {
                    f(i, l);
                }
            }
        }
    }
    fence
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
    let after = &lines[start + 1..];
    let next_h2 = scan_outside_fences(after, |_, l| is_any_h2(l))
        .into_iter()
        .next()
        .map(|off| start + 1 + off)
        .unwrap_or(lines.len());

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
/// separator is U+00B7 with single ASCII spaces on each side. Only
/// well-formed H3 headings start a new block — a malformed `###`
/// line is folded into the *previous* block's body so legacy /
/// hand-edited content is preserved (round-2 finding G2/O3).
#[allow(dead_code)] // sister tickets (`decide`, `agent-run`) consume this
pub fn parse_section(body: &str, section: &str) -> Vec<Block> {
    let lines: Vec<&str> = body.split('\n').collect();
    let section_starts = scan_outside_fences(&lines, |_, l| is_h2_named(l, section));
    let Some(&start) = section_starts.first() else {
        return Vec::new();
    };

    // Section ends at the next H2 outside any fence, or at EOF.
    let after = &lines[start + 1..];
    let next_offset = scan_outside_fences(after, |_, l| is_any_h2(l))
        .into_iter()
        .next()
        .unwrap_or(after.len());
    let span = &after[..next_offset];

    // Only *valid* block headings become boundaries — malformed H3
    // lines pass through as body content of the previous block. This
    // is what the docstring promises; the previous version dropped
    // the body of any malformed H3 entirely.
    let valid_block_starts: Vec<(usize, (String, String))> =
        scan_outside_fences(span, |_, l| is_h3(l))
            .into_iter()
            .filter_map(|i| parse_block_heading(span[i]).map(|p| (i, p)))
            .collect();

    let mut out = Vec::with_capacity(valid_block_starts.len());
    for (idx, (h_idx, parsed)) in valid_block_starts.iter().enumerate() {
        let end_idx = valid_block_starts
            .get(idx + 1)
            .map(|(i, _)| *i)
            .unwrap_or(span.len());
        let body_lines = &span[*h_idx + 1..end_idx];
        let body_text = trim_blank_borders(body_lines);
        out.push(Block {
            timestamp: parsed.0.clone(),
            author: parsed.1.clone(),
            body: body_text,
        });
    }
    out
}

/// Extract the raw text between `## <section>` and the next H2 (or EOF),
/// fence-aware. Returns `None` if the section is absent. Leading and
/// trailing blank lines are trimmed; interior text is preserved verbatim
/// so list bullets, fenced code blocks, etc. round-trip unchanged.
///
/// Used by `issuectl context` to lift sections like `Acceptance Criteria`
/// or `Quick Test` out of an issue body without re-parsing markdown.
pub fn extract_section_text(body: &str, section: &str) -> Option<String> {
    all_h2_sections(body).remove(section)
}

/// Collect every fence-aware `## <name>` section in `body` into a map
/// from heading name (verbatim, exact case) to the trimmed text between
/// it and the next H2. If a heading appears more than once, the first
/// occurrence wins — matching `extract_section_text`.
pub fn all_h2_sections(body: &str) -> std::collections::BTreeMap<String, String> {
    use std::collections::BTreeMap;
    let lines: Vec<&str> = body.split('\n').collect();
    let h2_indices = scan_outside_fences(&lines, |_, l| is_any_h2(l));
    let mut out = BTreeMap::new();
    for (i, idx) in h2_indices.iter().enumerate() {
        let name = match lines[*idx].strip_prefix("## ") {
            Some(rest) => rest.trim_end().to_string(),
            None => continue,
        };
        let next = h2_indices
            .get(i + 1)
            .copied()
            .unwrap_or(lines.len());
        let body_lines = &lines[*idx + 1..next];
        let text = trim_blank_borders(body_lines);
        out.entry(name).or_insert(text);
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
        let block = render_note_block("2026-05-07T12:00:00Z", "alice", "hello world").unwrap();
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
        let b = render_note_block("2026-05-07T12:00:00Z", "alice", "hello\n\n").unwrap();
        assert!(b.starts_with("### 2026-05-07T12:00:00Z · @alice\n\n"));
        assert!(b.ends_with("hello\n"));
    }

    #[test]
    fn render_note_block_enforces_validators() {
        // Round-2 finding O8: the renderer must reject what the
        // validators reject so callers can't emit malformed
        // headings by going around them.
        assert!(render_note_block("ts", " alice ", "x").is_err());
        assert!(render_note_block("ts", "alice", "## Decisions\n").is_err());
        assert!(render_note_block("ts", "alice\n## Pwned", "x").is_err());
    }

    #[test]
    fn fence_close_can_be_longer_than_open() {
        // Round-2 finding G3/O2: per CommonMark, a closing fence
        // may be longer than the opener. Previously the strict
        // length match left the fence dangling, so headings after
        // the longer close were misclassified as code-block content.
        let body = "\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @bob\n\n\
            ```rust\n\
            code\n\
            ````\n\
            ## Decisions\n\n\
            ### 2026-05-02T00:00:00Z · @cara\n\npicked X\n";
        let coms = parse_section(body, COMMENTS);
        let decs = parse_section(body, DECISIONS);
        assert_eq!(coms.len(), 1, "Comments section parsed correctly");
        assert_eq!(decs.len(), 1, "Decisions parsed after longer close fence");
        assert_eq!(decs[0].author, "cara");
    }

    #[test]
    fn close_fence_must_be_only_whitespace_after_run() {
        // ` ``` not a close ` is content per CommonMark — the
        // shared scanner must NOT treat it as closing the fence.
        let body = "\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @bob\n\n\
            ```\n\
            ``` not a close\n\
            ## still inside fence\n\
            ```\n\
            \n## Decisions\n\n\
            ### 2026-05-02T00:00:00Z · @cara\n\npicked\n";
        let decs = parse_section(body, DECISIONS);
        assert_eq!(decs.len(), 1);
        assert_eq!(decs[0].author, "cara");
    }

    #[test]
    fn parse_section_folds_malformed_h3_into_previous_block() {
        // Round-2 finding G2/O3: malformed H3 lines must not become
        // boundaries — their content folds into the previous
        // block's body rather than vanishing.
        let body = "\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @alice\n\n\
            first\n\n\
            ### not a valid block heading\n\n\
            legacy text that should remain visible\n\n\
            ### 2026-05-02T00:00:00Z · @bob\n\n\
            second\n";
        let blocks = parse_section(body, COMMENTS);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].author, "alice");
        assert!(
            blocks[0].body.contains("first") && blocks[0].body.contains("legacy text"),
            "alice's body should fold malformed-h3 content, got:\n{}",
            blocks[0].body
        );
        assert_eq!(blocks[1].author, "bob");
    }

    #[test]
    fn validate_message_rejects_unclosed_fence() {
        // Round-2 finding O10: an unclosed fence in the message
        // would silently swallow future blocks once appended.
        assert!(validate_message("```rust\nunclosed").is_err());
        assert!(validate_message("```rust\nclosed\n```\n").is_ok());
    }

    #[test]
    fn validate_author_rejects_leading_trailing_whitespace() {
        // Round-2 finding O9.
        assert!(validate_author(" alice").is_err());
        assert!(validate_author("alice ").is_err());
        assert!(validate_author("alice").is_ok());
    }
}
