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

/// Line-based splitter. Closing `---` must occupy its own line.
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

    // Find the closing delimiter on its own line.
    let mut search_from = 0;
    while let Some(pos) = after_open[search_from..].find("\n---") {
        let abs = search_from + pos;
        let end_of_marker = abs + 4; // past "\n---"
        let next = after_open.as_bytes().get(end_of_marker);
        match next {
            None => {
                return Split {
                    frontmatter: Some(&after_open[..abs]),
                    body: "",
                };
            }
            Some(b'\n') => {
                let body = &after_open[end_of_marker + 1..];
                return Split {
                    frontmatter: Some(&after_open[..abs]),
                    body,
                };
            }
            Some(b'\r') if after_open.as_bytes().get(end_of_marker + 1) == Some(&b'\n') => {
                let body = &after_open[end_of_marker + 2..];
                return Split {
                    frontmatter: Some(&after_open[..abs]),
                    body,
                };
            }
            _ => {
                // Embedded `\n---foo` — keep scanning past this position.
                search_from = end_of_marker;
            }
        }
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
        assert_eq!(
            s.frontmatter,
            Some("note: |\n  ---foo\nstatus: open"),
        );
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
