//! Shared frontmatter/body splitter for `item.md` files.
//!
//! Three modules used to carry their own slightly-divergent splitters
//! (`write::split_text`, `fmt::split_for_fmt`, `merge_driver::parse_sides`).
//! That drift is exactly the sort of thing a merge driver and a
//! formatter must agree on, so this module consolidates the rule:
//!
//! - The opening delimiter is exactly `---` on its own line (optional
//!   leading whitespace stripped first — files saved by editors with
//!   BOM/CRLF still parse).
//! - The closing delimiter is `---` on its own line (`\n---\n` or
//!   `\n---\r\n` or `\n---` at EOF). A `---` embedded in the middle of
//!   a YAML block scalar therefore can't accidentally end the block.
//! - The body returned starts immediately after the closing delimiter's
//!   trailing newline (one newline consumed).

/// Split result. `frontmatter` is the YAML text without the `---`
/// delimiters; `body` is everything after the closing delimiter's
/// trailing newline.
pub struct Split<'a> {
    pub frontmatter: Option<&'a str>,
    pub body: &'a str,
}

/// Line-based splitter. Closing `---` must occupy its own line *and*
/// must not be inside a fenced code block. The fence-awareness exists
/// because YAML frontmatter is sometimes used to document issuectl
/// itself: an issue body that contains a `\`\`\`yaml ... \`\`\`` block
/// (or even a stray `\n---\n` line inside any fence) would otherwise
/// short-circuit the closing marker — and if the user forgot the real
/// closing `---`, body content would leak into the parsed mapping and
/// surface as bogus "unknown frontmatter keys" warnings (regression
/// `virtually-callous-rainstorm`).
pub fn split(text: &str) -> Split<'_> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with("---") {
        return Split {
            frontmatter: None,
            body: text,
        };
    }
    // Opening delimiter must be its own line: "---" possibly followed
    // by \r\n or \n.
    let after_open = match trimmed.as_bytes().get(3) {
        Some(b'\n') => &trimmed[4..],
        Some(b'\r') if trimmed.as_bytes().get(4) == Some(&b'\n') => &trimmed[5..],
        // `---<EOF>` — a one-liner with no body
        None => {
            return Split {
                frontmatter: Some(""),
                body: "",
            };
        }
        // `--- foo` is not a frontmatter delimiter.
        _ => {
            return Split {
                frontmatter: None,
                body: text,
            };
        }
    };

    // Walk line-by-line tracking fence state so a `---` line inside a
    // fenced code block can never close the frontmatter.
    use crate::body_sections::{closes_fence, opening_fence, Fence};
    let mut fence: Option<Fence> = None;
    // Byte offset of the start of the current line within `after_open`.
    let mut line_start: usize = 0;
    while line_start <= after_open.len() {
        // Slice out the current line (without the trailing newline).
        let rest = &after_open[line_start..];
        let (line_without_nl, line_len_with_nl) = match rest.find('\n') {
            Some(nl) => {
                let mut end = nl;
                if end > 0 && rest.as_bytes()[end - 1] == b'\r' {
                    end -= 1;
                }
                (&rest[..end], nl + 1)
            }
            None => (rest, rest.len()),
        };

        match fence {
            Some(open) if closes_fence(line_without_nl, open) => {
                fence = None;
            }
            Some(_) => { /* inside fence, skip marker matching */ }
            None => {
                if let Some(o) = opening_fence(line_without_nl) {
                    fence = Some(o);
                } else if line_without_nl == "---" {
                    let after_marker_start = line_start + line_without_nl.len();
                    let body_start = match after_open.as_bytes().get(after_marker_start) {
                        None => {
                            return Split {
                                frontmatter: Some(&after_open[..line_start.saturating_sub(1)]),
                                body: "",
                            }
                        }
                        Some(b'\n') => after_marker_start + 1,
                        Some(b'\r')
                            if after_open.as_bytes().get(after_marker_start + 1)
                                == Some(&b'\n') =>
                        {
                            after_marker_start + 2
                        }
                        _ => after_marker_start,
                    };
                    // Frontmatter content excludes the trailing newline
                    // before the closing marker.
                    let fm_end = line_start.saturating_sub(1);
                    return Split {
                        frontmatter: Some(&after_open[..fm_end]),
                        body: &after_open[body_start..],
                    };
                }
            }
        }

        if line_len_with_nl == 0 {
            break;
        }
        line_start += line_len_with_nl;
    }
    // No closing delimiter — treat the whole input as body.
    Split {
        frontmatter: None,
        body: text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_split() {
        let s = split("---\nstatus: open\n---\n# T\n");
        assert_eq!(s.frontmatter, Some("status: open"));
        assert_eq!(s.body, "# T\n");
    }

    #[test]
    fn blank_line_before_body() {
        let s = split("---\nstatus: open\n---\n\n# T\n");
        assert_eq!(s.frontmatter, Some("status: open"));
        assert_eq!(s.body, "\n# T\n");
    }

    #[test]
    fn no_frontmatter_returns_full_text() {
        let s = split("# Just body\n");
        assert!(s.frontmatter.is_none());
        assert_eq!(s.body, "# Just body\n");
    }

    #[test]
    fn malformed_opening_is_not_frontmatter() {
        let s = split("--- not a fence\nstatus: open\n");
        assert!(s.frontmatter.is_none());
    }

    #[test]
    fn embedded_dashes_in_body_not_consumed_as_close() {
        let s = split("---\nfoo: bar\n---\nbody --- still body\n");
        assert_eq!(s.frontmatter, Some("foo: bar"));
        assert_eq!(s.body, "body --- still body\n");
    }

    #[test]
    fn embedded_dashes_in_block_scalar_not_consumed() {
        // A YAML block scalar that happens to contain `\n---foo` text
        // must not be misread as the close marker — close requires the
        // line to be exactly `---`.
        let s = split("---\nnote: |\n  ---foo\nstatus: open\n---\n# T\n");
        assert_eq!(s.frontmatter, Some("note: |\n  ---foo\nstatus: open"),);
    }

    #[test]
    fn crlf_terminators_handled() {
        let s = split("---\r\nstatus: open\r\n---\r\n# T\r\n");
        assert_eq!(s.frontmatter, Some("status: open\r"));
        assert_eq!(s.body, "# T\r\n");
    }

    #[test]
    fn unterminated_frontmatter_returns_no_split() {
        let s = split("---\nstatus: open\n");
        assert!(s.frontmatter.is_none());
    }
}
