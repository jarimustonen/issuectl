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

use std::sync::OnceLock;

use anyhow::{bail, Result};
use chrono::{SecondsFormat, Utc};
use regex::Regex;

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

/// Reject messages that cannot be safely appended. Unfenced `## `/
/// `### ` heading lines are *not* rejected here — they are legitimate
/// note content and are demoted to H4+ by [`demote_managed_headings`]
/// at render time so they cannot be mistaken for a managed section /
/// block boundary. The one structural hazard demotion can't fix is an
/// unclosed fence (round-2 finding O10): it would silently swallow
/// later blocks once appended, so we still reject those.
pub fn validate_message(message: &str) -> Result<()> {
    if message.trim().is_empty() {
        bail!("message cannot be empty");
    }
    let lines: Vec<&str> = message.split('\n').collect();
    let trailing_fence = scan_with_fence_state(&lines, |_, _| {});
    if trailing_fence.is_some() {
        bail!(
            "message contains an unclosed fenced code block; close it before appending \
             so future blocks are not swallowed"
        );
    }
    Ok(())
}

/// Demote every unfenced `## …` / `### …` heading in a note message so
/// it cannot collide with the reserved section model once embedded in
/// a `### <ts> · @<author>` block. `## …` becomes `#### …` and `### …`
/// becomes `##### …` — both pushed to H4+, i.e. strictly deeper than
/// the H3 block heading and the H2 section heading, so the writer /
/// reader fence-aware scanners never misread them as a `## <section>`
/// boundary or a new block. Adding a fixed two `#` levels preserves
/// the user's relative heading hierarchy.
///
/// Headings inside a fenced code block are content and pass through
/// verbatim — the parser is fence-aware, so they can't break out.
/// This replaces the old hard rejection of unfenced H2/H3 (callers
/// used to pre-demote or fence such lines by hand); structured notes
/// with markdown subheadings now round-trip intact.
pub fn demote_managed_headings(message: &str) -> String {
    let lines: Vec<&str> = message.split('\n').collect();
    let mut out = String::with_capacity(message.len() + 16);
    let mut fence: Option<Fence> = None;
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match fence {
            Some(open) => {
                if closes_fence(l, open) {
                    fence = None;
                }
                out.push_str(l);
            }
            None => {
                if let Some(o) = opening_fence(l) {
                    fence = Some(o);
                    out.push_str(l);
                } else if is_any_h2(l) || is_h3(l) {
                    // `## X` → `#### X`, `### X` → `##### X`.
                    out.push_str("##");
                    out.push_str(l);
                } else {
                    out.push_str(l);
                }
            }
        }
    }
    out
}

/// Render a block heading + body for a `Comments`-style section.
/// Calls the validators internally so the shape on disk cannot drift
/// from the input contract (round-2 finding O8). Callers that have
/// already validated should pass canonical input — the call is
/// cheap.
pub fn render_note_block(ts: &str, author: &str, message: &str) -> Result<String> {
    validate_author(author)?;
    validate_message(message)?;
    let demoted = demote_managed_headings(message);
    Ok(format!(
        "### {ts} · @{author}\n\n{}\n",
        demoted.trim_end_matches('\n')
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

/// Which markdown constructs the rewriter should treat as off-limits
/// in addition to fenced code blocks (which are always skipped).
/// `refs::rewrite_body_refs` enables both — a literal `` `@slug` `` or
/// a URL fragment is documentation, not a live mention. `doctor`'s
/// `#NN`+path rewriter enables only `inline_code`: intra-repo link
/// URLs like `[t](../old-slug/item.md)` are exactly what doctor must
/// rewrite when a directory is renamed.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RewriteSkips {
    pub inline_code: bool,
    pub link_urls: bool,
}

impl RewriteSkips {
    pub fn code_and_urls() -> Self {
        Self {
            inline_code: true,
            link_urls: true,
        }
    }
    pub fn code_only() -> Self {
        Self {
            inline_code: true,
            link_urls: false,
        }
    }
}

/// Walk `body` and apply `transform` to every span of plain prose
/// text — i.e. text that is NOT inside a fenced code block, and
/// (depending on `skips`) NOT inside an inline code span (`` `…` ``,
/// `` ``…`` ``, `` ```…``` ``) or a markdown link URL (`](…)`).
/// Skip regions are emitted verbatim. The shared implementation
/// behind `refs::rewrite_body_refs` and `doctor::rewrite_text` so the
/// two callers cannot drift on fence/inline-code/URL detection even
/// though they differ on which subset of those they skip.
///
/// `transform` is called with each prose segment as `&str` and must
/// return its replacement; segment boundaries fall at fence lines
/// and at the start/end of every skip region, so a token straddling
/// such a boundary is never rewritten (and is also never a real
/// token in the source).
pub(crate) fn rewrite_outside_code_and_urls<F>(
    body: &str,
    skips: RewriteSkips,
    mut transform: F,
) -> String
where
    F: FnMut(&str) -> String,
{
    let mut out = String::with_capacity(body.len());
    let mut fence: Option<Fence> = None;
    for raw in body.split_inclusive('\n') {
        let (line, nl) = match raw.strip_suffix('\n') {
            Some(stripped) => (stripped, "\n"),
            None => (raw, ""),
        };
        match fence {
            Some(open) => {
                if closes_fence(line, open) {
                    fence = None;
                }
                out.push_str(line);
                out.push_str(nl);
            }
            None => {
                if let Some(o) = opening_fence(line) {
                    fence = Some(o);
                    out.push_str(line);
                    out.push_str(nl);
                } else {
                    rewrite_line_outside_code_and_urls(line, skips, &mut transform, &mut out);
                    out.push_str(nl);
                }
            }
        }
    }
    out
}

fn skip_regions_regex(skips: RewriteSkips) -> Option<&'static Regex> {
    // Four pre-compiled variants — we never need both "no skips" (no
    // walker call at all) and intermediate combinations are cheap.
    // Order matters in each pattern: try the longest backtick run
    // first so a `` `` `` span isn't truncated as two single-backtick
    // spans. Link URLs are matched as `](…)` — we don't re-check the
    // `[…]` half because a bare `](url)` without a preceding bracket
    // is vanishingly rare in prose, and matching the full link form
    // would either need balanced brackets (regex can't) or
    // false-negative on `[a [b] c](url)`.
    static CODE_AND_URLS: OnceLock<Regex> = OnceLock::new();
    static CODE_ONLY: OnceLock<Regex> = OnceLock::new();
    static URLS_ONLY: OnceLock<Regex> = OnceLock::new();
    match (skips.inline_code, skips.link_urls) {
        (true, true) => Some(CODE_AND_URLS.get_or_init(|| {
            Regex::new(r"```[^`\n]+```|``[^`\n]+``|`[^`\n]+`|\]\([^)\n]*\)")
                .expect("valid skip regex")
        })),
        (true, false) => Some(CODE_ONLY.get_or_init(|| {
            Regex::new(r"```[^`\n]+```|``[^`\n]+``|`[^`\n]+`").expect("valid skip regex")
        })),
        (false, true) => {
            Some(URLS_ONLY.get_or_init(|| Regex::new(r"\]\([^)\n]*\)").expect("valid skip regex")))
        }
        (false, false) => None,
    }
}

fn rewrite_line_outside_code_and_urls<F>(
    line: &str,
    skips: RewriteSkips,
    transform: &mut F,
    out: &mut String,
) where
    F: FnMut(&str) -> String,
{
    let Some(re) = skip_regions_regex(skips) else {
        out.push_str(&transform(line));
        return;
    };
    let mut last = 0usize;
    for m in re.find_iter(line) {
        if m.start() > last {
            out.push_str(&transform(&line[last..m.start()]));
        }
        out.push_str(m.as_str());
        last = m.end();
    }
    if last < line.len() {
        out.push_str(&transform(&line[last..]));
    }
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

/// Visit every body line that lies outside a fenced code block. The
/// closure receives `(line_index, line)` and is called in source
/// order. Used by `mutate.rs` to make the `check` checkbox-toggle
/// verb fence-aware without copying every line — issues commonly
/// have hundreds of lines and the previous helper allocated one
/// `String` per non-fenced line for no reason.
///
/// Known limitation: matches CommonMark only at the document top
/// level. Fences nested inside list items at 4+ spaces of
/// indentation aren't recognised as code blocks, because
/// `opening_fence` rejects `indent >= 4` to follow the spec for
/// indented code. This is a deliberate scope choice — full
/// container parsing (lists / blockquotes) would need a real
/// CommonMark frontend, which is out of scope for the current
/// mutation CLI.
pub(crate) fn for_each_line_outside_fences<F: FnMut(usize, &str)>(body: &str, mut f: F) {
    let lines: Vec<&str> = body.split('\n').collect();
    scan_outside_fences(&lines, |i, l| {
        f(i, l);
        false
    });
}

/// Walk `lines` and call `f(i, line)` for every line that lies
/// outside a fenced code block. Returns indices where `f` returned
/// true. Single source of truth for fence-aware scanning — every
/// other function in this module routes through here.
fn scan_outside_fences<F: FnMut(usize, &str) -> bool>(lines: &[&str], mut f: F) -> Vec<usize> {
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

/// A parse warning surfaced by [`parse_section`]. Distinct shapes
/// previously collapsed into "empty vec" — sister tickets (`decide`,
/// `agent-run`) need to tell them apart so a hand-edited section with
/// an unclosed fence isn't reported the same way as a missing section.
#[allow(dead_code)] // sister tickets consume the diagnostic surface
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseWarning {
    /// An `###` line inside the section did not match
    /// `### <ts> · @<author>`. The line content is never lost — if a
    /// valid block precedes it, the line and any following content
    /// fold into that block's body; otherwise the line orphans
    /// (sister-ticket consumers may want to surface preamble content
    /// out-of-band). `folded_into_previous_block` distinguishes the
    /// two cases so a consumer can decide whether the warning is
    /// "informational, content was preserved" or "content may be
    /// inaccessible without re-parsing the section raw text".
    MalformedBlockHeading {
        /// 0-based line index inside the body.
        line_no: usize,
        /// Raw line text, trimmed of trailing whitespace.
        line: String,
        /// `true` when the line falls under an earlier valid block
        /// heading and its content is preserved in that block's body.
        /// `false` when the malformed line appears before any valid
        /// `### <ts> · @<author>` heading and so its content is not
        /// represented in `ParsedSection.blocks`.
        folded_into_previous_block: bool,
    },
    /// A code fence opened inside the section but never closed before
    /// EOF. Anything after the opener is consumed as fence content,
    /// which can swallow subsequent block headings entirely.
    UnclosedFence {
        /// 0-based line index of the opening fence.
        line_no: usize,
    },
    /// More than one `## <section>` heading was present at H2 level.
    /// The first occurrence wins (matching `extract_section_text`);
    /// later occurrences are reported here so callers can flag the
    /// duplicate without parsing the body twice.
    DuplicateSection {
        /// 0-based line index of the duplicate heading.
        line_no: usize,
    },
}

/// Result of [`parse_section`]. Disambiguates the cases that used to
/// collapse into "empty `Vec<Block>`":
///
/// | case                                  | `found` | `blocks` | `warnings`                          |
/// |---------------------------------------|---------|----------|-------------------------------------|
/// | section absent                        | `false` | empty    | empty                               |
/// | section present, no blocks            | `true`  | empty    | empty                               |
/// | section present, all H3 malformed     | `true`  | empty    | `MalformedBlockHeading` × N         |
/// | section swallowed by unclosed fence   | `true`  | empty    | `UnclosedFence`                     |
/// | duplicate `## <section>` headings     | `true`  | (first)  | `DuplicateSection` × N — also       |
/// |                                       |         |          | exposed via `duplicate_section_count`|
#[allow(dead_code)] // sister tickets (`decide`, `agent-run`) consume this
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedSection {
    /// True when the named H2 heading was found at least once.
    pub found: bool,
    /// Successfully parsed blocks, in source order.
    pub blocks: Vec<Block>,
    /// Soft-failure diagnostics — see [`ParseWarning`]. An empty list
    /// means the section was either absent or cleanly parsed.
    pub warnings: Vec<ParseWarning>,
}

impl ParsedSection {
    /// Count of duplicate `## <section>` headings beyond the first.
    /// Derived from `warnings` so the two cannot drift; prefer this
    /// over filtering `warnings` at the call site.
    #[allow(dead_code)]
    pub fn duplicate_section_count(&self) -> usize {
        self.warnings
            .iter()
            .filter(|w| matches!(w, ParseWarning::DuplicateSection { .. }))
            .count()
    }
}

/// Parse all blocks under the H2 section named `section`. Headings
/// inside fenced code blocks are treated as content (matching the
/// writer).
///
/// Block heading shape: `### <ts> · @<author>`. The middle-dot
/// separator is U+00B7 with single ASCII spaces on each side. Only
/// well-formed H3 headings start a new block — a malformed `###`
/// line is folded into the *previous* block's body so legacy /
/// hand-edited content is preserved (round-2 finding G2/O3) and a
/// `MalformedBlockHeading` warning is recorded.
///
/// Returns a [`ParsedSection`] that distinguishes "section absent"
/// from "section present but empty" from "section present but
/// malformed" — see the table on `ParsedSection`.
#[allow(dead_code)] // sister tickets (`decide`, `agent-run`) consume this
pub fn parse_section(body: &str, section: &str) -> ParsedSection {
    let lines: Vec<&str> = body.split('\n').collect();
    let section_starts = scan_outside_fences(&lines, |_, l| is_h2_named(l, section));
    let Some(&start) = section_starts.first() else {
        return ParsedSection::default();
    };

    let mut warnings = Vec::new();
    for &dup in section_starts.iter().skip(1) {
        warnings.push(ParseWarning::DuplicateSection { line_no: dup });
    }

    // Section ends at the next H2 outside any fence, or at EOF.
    let after = &lines[start + 1..];
    let next_offset = scan_outside_fences(after, |_, l| is_any_h2(l))
        .into_iter()
        .next()
        .unwrap_or(after.len());
    let span = &after[..next_offset];

    // Detect an unclosed fence inside the section span. Without this,
    // a writer who forgot the closing ``` silently has their later
    // blocks eaten as fence content; surface it explicitly so callers
    // can flag the issue rather than reporting "no blocks".
    if let Some(open_idx) = unclosed_fence_index(span) {
        warnings.push(ParseWarning::UnclosedFence {
            line_no: start + 1 + open_idx,
        });
    }

    // Only *valid* block headings become boundaries — malformed H3
    // lines pass through as body content of the previous block. The
    // malformed lines are recorded as warnings so a caller can
    // surface them; the body content remains visible to the user.
    let h3_indices = scan_outside_fences(span, |_, l| is_h3(l));
    let mut valid_block_starts: Vec<(usize, (String, String))> = Vec::new();
    let mut seen_valid = false;
    for i in &h3_indices {
        match parse_block_heading(span[*i]) {
            Some(parsed) => {
                seen_valid = true;
                valid_block_starts.push((*i, parsed));
            }
            None => warnings.push(ParseWarning::MalformedBlockHeading {
                line_no: start + 1 + *i,
                line: span[*i].trim_end().to_string(),
                folded_into_previous_block: seen_valid,
            }),
        }
    }

    let mut blocks = Vec::with_capacity(valid_block_starts.len());
    for (idx, (h_idx, parsed)) in valid_block_starts.iter().enumerate() {
        let end_idx = valid_block_starts
            .get(idx + 1)
            .map(|(i, _)| *i)
            .unwrap_or(span.len());
        let body_lines = &span[*h_idx + 1..end_idx];
        let body_text = trim_blank_borders(body_lines);
        blocks.push(Block {
            timestamp: parsed.0.clone(),
            author: parsed.1.clone(),
            body: body_text,
        });
    }

    ParsedSection {
        found: true,
        blocks,
        warnings,
    }
}

/// Line index of the *unclosed* opening fence in `lines`, or `None`
/// if every opener was matched by a closer. State-aware so a fence
/// that opens inside an already-open fence is not double-counted.
fn unclosed_fence_index(lines: &[&str]) -> Option<usize> {
    let mut fence: Option<Fence> = None;
    let mut open_at: Option<usize> = None;
    for (i, l) in lines.iter().enumerate() {
        match fence {
            Some(open) if closes_fence(l, open) => {
                fence = None;
                open_at = None;
            }
            Some(_) => {}
            None => {
                if let Some(o) = opening_fence(l) {
                    fence = Some(o);
                    open_at = Some(i);
                }
            }
        }
    }
    if fence.is_some() {
        open_at
    } else {
        None
    }
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
        let next = h2_indices.get(i + 1).copied().unwrap_or(lines.len());
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
        let out = append_block(
            body,
            COMMENTS,
            "### 2026-05-02T00:00:00Z · @alice\n\nsecond\n",
        );
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
        let out = append_block(
            body,
            COMMENTS,
            "### 2026-05-02T00:00:00Z · @alice\n\nsecond\n",
        );
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
        let out = append_block(
            body,
            COMMENTS,
            "### 2026-05-03T00:00:00Z · @alice\n\nsecond\n",
        );
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
        let formatted =
            crate::fmt::format_text(&format!("---\nstatus: open\n---\n{appended}")).unwrap();
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
    fn validate_message_accepts_unfenced_h2_h3() {
        assert!(validate_message("plain text").is_ok());
        assert!(validate_message("multi\nline\n").is_ok());
        // Legitimate structured notes with markdown subheadings are
        // accepted now — they are demoted at render time rather than
        // rejected (note-from-file-rejects-headings).
        assert!(validate_message("normal\n\n## Section\n\nbody").is_ok());
        assert!(validate_message("### Subheading\n\nbody").is_ok());
        // Quoting heading-shaped content inside a fence is fine too.
        assert!(validate_message("see this:\n```\n## bash comment\n```\nok").is_ok());
        // Empty and unclosed fences are still rejected.
        assert!(validate_message("").is_err());
        assert!(validate_message("```rust\nunclosed").is_err());
    }

    #[test]
    fn demote_managed_headings_pushes_h2_h3_below_block_level() {
        // Unfenced H2/H3 gain two levels; fenced content is untouched.
        let msg = "## Section\n\ntext\n\n### Sub\n\n```\n## in-fence\n### also-in-fence\n```\n";
        let out = demote_managed_headings(msg);
        assert!(out.contains("#### Section"));
        assert!(out.contains("##### Sub"));
        // In-fence heading-shaped lines pass through verbatim.
        assert!(out.contains("```\n## in-fence\n### also-in-fence\n```"));
        // Nothing left at H2 or H3 outside the fence.
        assert!(!is_any_h2("#### Section"));
        assert!(!is_h3("##### Sub"));
    }

    #[test]
    fn render_note_block_demotes_user_headings() {
        // A structured note with `##`/`###` renders without error and
        // the headings land at H4+ so they can't be a section boundary.
        let b = render_note_block(
            "2026-05-07T12:00:00Z",
            "alice",
            "## Findings\n\ndetail\n\n### Detail\n\nmore\n",
        )
        .unwrap();
        assert!(b.starts_with("### 2026-05-07T12:00:00Z · @alice\n\n"));
        assert!(b.contains("#### Findings"));
        assert!(b.contains("##### Detail"));
        assert!(!b.contains("\n## Findings"));
        assert!(!b.contains("\n### Detail"));
    }

    #[test]
    fn note_with_user_headings_round_trips_and_preserves_section() {
        // Regression for note-from-file-rejects-headings: appending a
        // note whose body contains `##`/`###` headings must not corrupt
        // the reserved `## Comments` section — the block still parses
        // back intact, and a following managed section is untouched.
        let body = "\n# T\n\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @bob\n\nfirst\n\n## Decisions\n\n\
            ### 2026-05-02T00:00:00Z · @cara\n\npicked X\n";
        let block = render_note_block(
            "2026-05-07T12:00:00Z",
            "alice",
            "## Section\n\nbody line\n\n### Subsection\n\nmore body\n",
        )
        .unwrap();
        let out = append_block(body, COMMENTS, &block);

        // Exactly one Comments and one Decisions section — the user
        // headings did not mint new H2 boundaries.
        assert_eq!(out.matches("\n## Comments").count(), 1, "body:\n{out}");
        assert_eq!(out.matches("\n## Decisions").count(), 1, "body:\n{out}");

        // Comments re-parses to two blocks (bob, then alice), with the
        // demoted headings preserved inside alice's body.
        let coms = parse_section(&out, COMMENTS);
        assert!(coms.warnings.is_empty(), "warnings={:?}", coms.warnings);
        assert_eq!(coms.blocks.len(), 2);
        assert_eq!(coms.blocks[0].author, "bob");
        assert_eq!(coms.blocks[1].author, "alice");
        assert!(coms.blocks[1].body.contains("#### Section"));
        assert!(coms.blocks[1].body.contains("##### Subsection"));
        assert!(coms.blocks[1].body.contains("body line"));

        // Decisions still parses independently and intact.
        let decs = parse_section(&out, DECISIONS);
        assert_eq!(decs.blocks.len(), 1);
        assert_eq!(decs.blocks[0].author, "cara");
    }

    // ── parser ──────────────────────────────────────────────────────

    #[test]
    fn parse_section_returns_blocks_in_order() {
        let body = "\n# T\n\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @bob\n\nfirst note\n\n\
            ### 2026-05-02T00:00:00Z · @alice\n\nsecond note\n\n## Decisions\n\n\
            ### 2026-05-03T00:00:00Z · @cara\n\npicked X\n";
        let parsed = parse_section(body, COMMENTS);
        assert!(parsed.found);
        assert!(parsed.warnings.is_empty(), "warnings={:?}", parsed.warnings);
        let blocks = &parsed.blocks;
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].timestamp, "2026-05-01T00:00:00Z");
        assert_eq!(blocks[0].author, "bob");
        assert_eq!(blocks[0].body, "first note");
        assert_eq!(blocks[1].author, "alice");
        assert_eq!(blocks[1].body, "second note");
        // Decisions parses independently
        let decs = parse_section(body, DECISIONS);
        assert!(decs.found);
        assert_eq!(decs.blocks.len(), 1);
        assert_eq!(decs.blocks[0].author, "cara");
    }

    #[test]
    fn parse_section_round_trips_with_append() {
        let body0 = "\n# T\n";
        let block = render_note_block("2026-05-07T12:00:00Z", "alice", "hello world").unwrap();
        let body1 = append_block(body0, COMMENTS, &block);
        let parsed = parse_section(&body1, COMMENTS);
        assert!(parsed.found);
        assert_eq!(parsed.blocks.len(), 1);
        assert_eq!(parsed.blocks[0].timestamp, "2026-05-07T12:00:00Z");
        assert_eq!(parsed.blocks[0].author, "alice");
        assert_eq!(parsed.blocks[0].body, "hello world");
    }

    #[test]
    fn parse_section_ignores_h3_inside_code_fence() {
        // A user might paste an example block heading inside a code
        // fence — that's content, not a real block.
        let body = "\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @bob\n\n\
            example:\n```\n### 2020-01-01 · @ghost\n```\n";
        let parsed = parse_section(body, COMMENTS);
        assert_eq!(parsed.blocks.len(), 1);
        assert_eq!(parsed.blocks[0].author, "bob");
    }

    #[test]
    fn parse_missing_section_returns_empty() {
        let body = "\n# T\n\nNo sections.\n";
        let parsed = parse_section(body, COMMENTS);
        assert!(!parsed.found);
        assert!(parsed.blocks.is_empty());
        assert!(parsed.warnings.is_empty());
        assert_eq!(parsed.duplicate_section_count(), 0);
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
        // An unfenced `## …` in the message is no longer rejected — it
        // is demoted to `#### …` (note-from-file-rejects-headings).
        assert!(render_note_block("ts", "alice", "## Decisions\n").is_ok());
        // An unclosed fence is still rejected.
        assert!(render_note_block("ts", "alice", "```rust\nunclosed").is_err());
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
        assert_eq!(coms.blocks.len(), 1, "Comments section parsed correctly");
        assert_eq!(
            decs.blocks.len(),
            1,
            "Decisions parsed after longer close fence"
        );
        assert_eq!(decs.blocks[0].author, "cara");
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
        assert_eq!(decs.blocks.len(), 1);
        assert_eq!(decs.blocks[0].author, "cara");
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
        let parsed = parse_section(body, COMMENTS);
        let blocks = &parsed.blocks;
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].author, "alice");
        assert!(
            blocks[0].body.contains("first") && blocks[0].body.contains("legacy text"),
            "alice's body should fold malformed-h3 content, got:\n{}",
            blocks[0].body
        );
        assert_eq!(blocks[1].author, "bob");
        // The malformed `### not a valid block heading` line is now
        // surfaced as a warning (still folded into the previous body
        // for content preservation).
        assert!(
            parsed
                .warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::MalformedBlockHeading { .. })),
            "expected a MalformedBlockHeading warning, got {:?}",
            parsed.warnings
        );
    }

    #[test]
    fn parse_section_present_but_empty_distinguishes_from_missing() {
        // Section heading exists but contains no H3 blocks. Was
        // indistinguishable from "no such section" before — sister
        // tickets infer different things from each.
        let body = "\n## Comments\n\n(none yet)\n";
        let parsed = parse_section(body, COMMENTS);
        assert!(parsed.found);
        assert!(parsed.blocks.is_empty());
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn parse_section_reports_unclosed_fence_inside_section() {
        // An unclosed ``` inside the section silently ate every later
        // block as fence content. Now flagged explicitly.
        let body = "\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @bob\n\nfirst\n\n\
            ```\nstill inside fence\n\
            ### 2026-05-02T00:00:00Z · @alice\n\nsecond\n";
        let parsed = parse_section(body, COMMENTS);
        assert!(parsed.found);
        // Only bob's block parses — alice is consumed by the open fence.
        assert_eq!(parsed.blocks.len(), 1);
        assert_eq!(parsed.blocks[0].author, "bob");
        assert!(
            parsed
                .warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::UnclosedFence { .. })),
            "expected UnclosedFence warning, got {:?}",
            parsed.warnings
        );
    }

    #[test]
    fn parse_section_reports_duplicate_section_headings() {
        // First wins for content (matching extract_section_text), but
        // duplicates are reported so callers can flag the corruption.
        let body = "\n## Comments\n\n\
            ### 2026-05-01T00:00:00Z · @bob\n\nfirst\n\n\
            ## Comments\n\n\
            ### 2026-05-02T00:00:00Z · @alice\n\nsecond\n";
        let parsed = parse_section(body, COMMENTS);
        assert!(parsed.found);
        assert_eq!(parsed.blocks.len(), 1);
        assert_eq!(parsed.blocks[0].author, "bob");
        assert_eq!(parsed.duplicate_section_count(), 1);
        assert!(
            parsed
                .warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::DuplicateSection { .. })),
            "expected DuplicateSection warning, got {:?}",
            parsed.warnings
        );
    }

    #[test]
    fn parse_section_all_malformed_h3_distinguishes_from_missing() {
        // Section present, every H3 line malformed. Was empty-vec
        // before; now found=true with malformed-heading warnings.
        let body = "\n## Comments\n\n\
            ### not a real heading\n\n\
            ### nor is this\n";
        let parsed = parse_section(body, COMMENTS);
        assert!(parsed.found);
        assert!(parsed.blocks.is_empty());
        let malformed = parsed
            .warnings
            .iter()
            .filter(|w| matches!(w, ParseWarning::MalformedBlockHeading { .. }))
            .count();
        assert_eq!(malformed, 2, "warnings={:?}", parsed.warnings);
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
