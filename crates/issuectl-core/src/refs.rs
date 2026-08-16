//! Issue-reference normalization. `@slug` / bare `slug` / legacy `#NN`
//! all flow through here on their way into frontmatter `related:` lists.
//! Used by `cmd_new` (CLI), `mutate::new_issue` (HTTP create), and
//! `mutate::update_issue` (HTTP update + CLI update).

use anyhow::{bail, Result};

use crate::slug;

/// Normalize a `--related`/`--add-related` reference. Accepts `@slug`, bare
/// `slug`, or legacy `#NN`. Output is canonical `@slug` form (or `#NN` if the
/// input was numeric — preserved verbatim so doctor can detect and migrate).
pub(crate) fn normalize_related_refs(refs: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        let trimmed = r.trim();
        if trimmed.is_empty() {
            bail!("related reference cannot be empty");
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
                bail!("related reference {:?} looks like #NN but isn't numeric", r);
            }
            out.push(format!("#{rest}"));
            continue;
        }
        let stripped = trimmed.strip_prefix('@').unwrap_or(trimmed);
        if !slug::is_valid(stripped) {
            bail!(
                "related reference must be @slug or a kebab-case slug, got {:?}",
                r
            );
        }
        out.push(format!("@{stripped}"));
    }
    Ok(out)
}

/// Rewrite a single frontmatter slug reference when it points at `old`.
/// Accepts canonical `@slug` and bare `slug` forms; the `@` prefix is
/// preserved on the rewritten value. Legacy numeric `#NN` refs and any
/// ref that doesn't match `old` are left untouched (returns `None`).
/// Used by `repo::rename_issue` to retarget `epic:` / `related:` /
/// `blocked_by:` entries across the store.
pub(crate) fn rewrite_slug_ref(raw: &str, old: &str, new: &str) -> Option<String> {
    let trimmed = raw.trim();
    let (prefix, bare) = match trimmed.strip_prefix('@') {
        Some(rest) => ("@", rest),
        None => ("", trimmed),
    };
    if bare == old {
        Some(format!("{prefix}{new}"))
    } else {
        None
    }
}

/// Rewrite `@old` body references to `@new`, returning the new body and
/// the number of occurrences replaced. A reference is the maximal run of
/// kebab-slug characters (`[a-z0-9-]`) immediately following an `@`; the
/// maximal-munch means `@old-suffix` is NOT matched when `old` is the
/// rename source. An `@` directly preceded by an alphanumeric (e.g. an
/// email local part like `alice@old-host`) is skipped so we only touch
/// standalone `@slug` mentions.
///
/// Fenced code blocks, inline code spans (`` `…` ``), and markdown
/// link URLs (`](…)`) are left verbatim — that's where users paste
/// literal examples of the old slug. The skip rule is shared with
/// `doctor::rewrite_text` via
/// `body_sections::rewrite_outside_code_and_urls`.
pub(crate) fn rewrite_body_refs(body: &str, old: &str, new: &str) -> (String, usize) {
    // Fast path: nothing to rewrite if the slug doesn't appear at all.
    if !body.contains(old) {
        return (body.to_string(), 0);
    }
    let mut count = 0usize;
    let out = crate::body_sections::rewrite_outside_code_and_urls(
        body,
        crate::body_sections::RewriteSkips::code_and_urls(),
        |seg| {
            let mut buf = String::with_capacity(seg.len());
            count += rewrite_line_refs(seg, old, new, &mut buf);
            buf
        },
    );
    (out, count)
}

/// Rewrite `@old` mentions in a single line, appending to `out`. Returns
/// the number of replacements. `prev` resets per line, so a line-leading
/// `@old` is a valid mention.
fn rewrite_line_refs(line: &str, old: &str, new: &str, out: &mut String) -> usize {
    let mut count = 0usize;
    let mut prev: Option<char> = None;
    let mut chars = line.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if ch == '@' {
            let prev_ok = prev.map(|c| !c.is_alphanumeric()).unwrap_or(true);
            let rest = &line[idx + ch.len_utf8()..];
            let tok_len: usize = rest
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
                .map(|c| c.len_utf8())
                .sum();
            let token = &rest[..tok_len];
            if prev_ok && token == old {
                out.push('@');
                out.push_str(new);
                count += 1;
                for _ in 0..token.chars().count() {
                    chars.next();
                }
                prev = new.chars().last();
                continue;
            }
        }
        out.push(ch);
        prev = Some(ch);
    }
    count
}

/// A repo-relative file reference extracted from a markdown body. The
/// `path` is the issue-relative target; `has_line_anchor` is true when
/// the original URL ended with a GitHub-style line anchor like
/// `#L10-L20`. The consumer uses the anchor flag to disambiguate
/// genuine missing attachments from cross-file code permalinks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyRef {
    pub path: String,
    pub has_line_anchor: bool,
}

/// Extract repo-relative file references from a markdown body — the
/// targets of inline images (`![alt](path)`) and links (`[text](path)`)
/// that point at the issue's own files (typically under `attachments/`
/// or `fixtures/`). Used by `doctor` to flag references that no longer
/// resolve to a file on disk.
///
/// Filtered OUT (not returned): absolute URLs (`https://`, `mailto:`,
/// any `scheme://`), root-absolute paths (`/etc/...`), bare anchors
/// (`#section`), and paths that escape the issue directory (`../`).
/// A leading `./` is stripped. Targets are de-angle-bracketed
/// (`<path>`) and any trailing `"title"` is dropped — both handled by
/// the CommonMark parser, not this function. Fenced and indented code
/// blocks plus code spans are skipped because pulldown-cmark never
/// emits `Tag::Link`/`Tag::Image` inside them.
///
/// Reference-style links (`[a][ref]` with `[ref]: target` defs) ARE
/// followed by the parser and surface here — a behavior change versus
/// the previous regex scan, which only matched inline `[text](url)`.
/// Likewise CommonMark autolinks `<https://…>` are emitted as links
/// and filtered out by the `scheme:` check.
pub fn extract_relative_body_refs(body: &str) -> Vec<BodyRef> {
    use pulldown_cmark::{Event, Options, Parser, Tag};

    // A CommonMark event walk skips code spans and fenced/indented code
    // blocks for free: pulldown-cmark only emits literal text events
    // inside those, never `Tag::Link`/`Tag::Image`. This avoids the
    // class of false positives where prose describes link syntax inside
    // backticks (e.g. `` `![alt](path)` ``).
    // Only `ENABLE_TABLES` is needed — links inside table cells produce
    // `Tag::Link`. Footnote/strikethrough/tasklist extensions don't
    // emit `Tag::Link` of their own, so leave them off to keep the
    // parser surface minimal.
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);

    let mut out = Vec::new();
    for ev in Parser::new_ext(body, opts) {
        let dest = match ev {
            Event::Start(Tag::Link { dest_url, .. })
            | Event::Start(Tag::Image { dest_url, .. }) => dest_url,
            _ => continue,
        };
        if let Some(r) = normalize_relative_ref(&dest) {
            out.push(r);
        }
    }
    out
}

/// Normalize a raw markdown link target to a repo-relative `BodyRef`,
/// or `None` if it is not an intra-issue relative reference (URL,
/// anchor, absolute path, or a `../` escape). The fragment is only
/// stripped from the path when it matches a GitHub-style line anchor
/// (`L10` / `L10-L20`); other `#fragments` are left attached so files
/// with literal `#` in the name survive the existence check unchanged.
fn normalize_relative_ref(raw: &str) -> Option<BodyRef> {
    let t = raw.trim();
    if t.is_empty() || t.starts_with('#') || t.starts_with('/') {
        return None;
    }
    // Any `scheme:` prefix (http:, https:, mailto:, tel:, data:, ...).
    if t.contains("://") || t.starts_with("mailto:") || t.starts_with("tel:") {
        return None;
    }
    // Reject Windows-style separators outright: on a Unix `Path` a
    // backslash is an ordinary character, so `..\..\x` would slip past
    // the component check below and let a body ref probe for files
    // outside the issue directory (an existence-check info leak).
    if t.contains('\\') {
        return None;
    }
    // Strip a trailing GitHub-style line anchor (`#L10`, `#L10-L20`)
    // because those are cross-file code permalinks, not part of the
    // attachment filename. Any other `#…` is kept verbatim so a
    // filename literally containing `#` is not silently rewritten.
    let (path_part, has_line_anchor) = match t.split_once('#') {
        Some((path, frag)) if is_line_anchor(frag) => (path, true),
        _ => (t, false),
    };
    if path_part.is_empty() {
        return None;
    }
    let stripped = path_part.strip_prefix("./").unwrap_or(path_part);
    if stripped.is_empty() {
        return None;
    }
    // Refuse any `..` segment so the reference cannot escape the issue
    // directory once joined. `Path::components` normalises `a/../b`-style
    // forms that a substring check would miss.
    use std::path::{Component, Path};
    if Path::new(stripped).components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }
    Some(BodyRef {
        path: stripped.to_string(),
        has_line_anchor,
    })
}

/// `L10` or `L10-L20` — the shape GitHub permalinks use for line
/// ranges. Used to decide whether a `#fragment` was a code anchor
/// (strip + carry the flag) or part of the filename (leave alone).
fn is_line_anchor(frag: &str) -> bool {
    let Some(rest) = frag.strip_prefix('L') else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    match rest.split_once("-L") {
        Some((a, b)) => {
            !a.is_empty()
                && !b.is_empty()
                && a.chars().all(|c| c.is_ascii_digit())
                && b.chars().all(|c| c.is_ascii_digit())
        }
        None => rest.chars().all(|c| c.is_ascii_digit()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_slug_ref_handles_at_bare_and_misses() {
        assert_eq!(
            rewrite_slug_ref("@old-tame-fox", "old-tame-fox", "new-wild-elk"),
            Some("@new-wild-elk".to_string())
        );
        assert_eq!(
            rewrite_slug_ref("old-tame-fox", "old-tame-fox", "new-wild-elk"),
            Some("new-wild-elk".to_string())
        );
        assert_eq!(
            rewrite_slug_ref("@other-calm-owl", "old-tame-fox", "x"),
            None
        );
        // legacy numeric refs are never rewritten
        assert_eq!(rewrite_slug_ref("#7", "old-tame-fox", "x"), None);
    }

    #[test]
    fn rewrite_body_refs_matches_whole_token_only() {
        let (out, n) = rewrite_body_refs(
            "see @old-tame-fox and @old-tame-foxes plus @old-tame-fox.",
            "old-tame-fox",
            "new-wild-elk",
        );
        assert_eq!(n, 2);
        assert_eq!(
            out,
            "see @new-wild-elk and @old-tame-foxes plus @new-wild-elk."
        );
    }

    #[test]
    fn rewrite_body_refs_leaves_fenced_code_untouched() {
        let body =
            "ping @old-tame-fox\n```sh\nissuectl show @old-tame-fox\n```\nthen @old-tame-fox\n";
        let (out, n) = rewrite_body_refs(body, "old-tame-fox", "new-wild-elk");
        assert_eq!(n, 2);
        // mention inside the fence is preserved verbatim
        assert!(out.contains("issuectl show @old-tame-fox"));
        assert!(out.contains("ping @new-wild-elk"));
        assert!(out.contains("then @new-wild-elk"));
    }

    #[test]
    fn rewrite_body_refs_adjacent_mentions_follow_email_rule() {
        // The second `@old` is glued to a slug char in the input, so the
        // email-avoidance rule intentionally leaves it alone.
        let (out, n) =
            rewrite_body_refs("@old-tame-fox@old-tame-fox", "old-tame-fox", "new-wild-elk");
        assert_eq!(n, 1);
        assert_eq!(out, "@new-wild-elk@old-tame-fox");
    }

    #[test]
    fn rewrite_body_refs_leaves_inline_code_untouched() {
        let body = "see `@old-tame-fox` literal and @old-tame-fox real";
        let (out, n) = rewrite_body_refs(body, "old-tame-fox", "new-wild-elk");
        assert_eq!(n, 1);
        assert_eq!(out, "see `@old-tame-fox` literal and @new-wild-elk real");
    }

    #[test]
    fn rewrite_body_refs_leaves_link_urls_untouched() {
        // `](…)` URL contents must survive even when they look like an
        // `@slug` reference (e.g. a fragment or anchor naming the slug).
        let body = "see [old](https://example.com/@old-tame-fox) plus @old-tame-fox";
        let (out, n) = rewrite_body_refs(body, "old-tame-fox", "new-wild-elk");
        assert_eq!(n, 1);
        assert_eq!(
            out,
            "see [old](https://example.com/@old-tame-fox) plus @new-wild-elk"
        );
    }

    #[test]
    fn rewrite_body_refs_double_backtick_inline_code_skipped() {
        let body = "literal ``@old-tame-fox`` then @old-tame-fox";
        let (out, n) = rewrite_body_refs(body, "old-tame-fox", "new-wild-elk");
        assert_eq!(n, 1);
        assert_eq!(out, "literal ``@old-tame-fox`` then @new-wild-elk");
    }

    #[test]
    fn rewrite_body_refs_skips_email_local_parts() {
        let (out, n) = rewrite_body_refs(
            "mail alice@old-tame-fox but ping @old-tame-fox",
            "old-tame-fox",
            "new-wild-elk",
        );
        assert_eq!(n, 1);
        assert_eq!(out, "mail alice@old-tame-fox but ping @new-wild-elk");
    }

    #[test]
    fn normalize_related_accepts_at_and_bare_slug() {
        assert_eq!(
            normalize_related_refs(&["@extremely-quiet-otter".to_string()]).unwrap(),
            vec!["@extremely-quiet-otter".to_string()]
        );
        assert_eq!(
            normalize_related_refs(&["amber-loud-fox".to_string()]).unwrap(),
            vec!["@amber-loud-fox".to_string()]
        );
    }

    #[test]
    fn normalize_related_preserves_legacy_numeric() {
        assert_eq!(
            normalize_related_refs(&["#7".to_string()]).unwrap(),
            vec!["#7".to_string()]
        );
    }

    fn paths(refs: Vec<BodyRef>) -> Vec<String> {
        refs.into_iter().map(|r| r.path).collect()
    }

    #[test]
    fn extract_relative_body_refs_picks_images_and_links() {
        let body = "See ![shot](attachments/shot.avif) and [log](./fixtures/run.log).";
        assert_eq!(
            paths(extract_relative_body_refs(body)),
            vec![
                "attachments/shot.avif".to_string(),
                "fixtures/run.log".to_string()
            ]
        );
    }

    #[test]
    fn extract_relative_body_refs_skips_urls_anchors_and_escapes() {
        let body = "[site](https://example.com) [a](#sec) [abs](/etc/x) \
                    [mail](mailto:x@y.z) [up](../other/x.png)";
        assert!(extract_relative_body_refs(body).is_empty());
    }

    #[test]
    fn extract_relative_body_refs_rejects_backslash_and_normalised_escapes() {
        // Windows-style separator and an embedded `a/../` escape must not
        // slip through to a filesystem existence check.
        let body = "[a](..\\..\\secret) [b](attachments/../../etc/passwd) \
                    [c](attachments/./x.avif)";
        assert_eq!(
            paths(extract_relative_body_refs(body)),
            vec!["attachments/./x.avif".to_string()]
        );
    }

    #[test]
    fn extract_relative_body_refs_strips_angle_brackets_and_titles() {
        let body = "![s](<attachments/a.avif> \"a title\")";
        assert_eq!(
            paths(extract_relative_body_refs(body)),
            vec!["attachments/a.avif".to_string()]
        );
    }

    #[test]
    fn extract_relative_body_refs_ignores_fenced_code() {
        let body = "real ![x](attachments/x.avif)\n```\n![y](attachments/y.avif)\n```\n";
        assert_eq!(
            paths(extract_relative_body_refs(body)),
            vec!["attachments/x.avif".to_string()]
        );
    }

    #[test]
    fn extract_relative_body_refs_flags_github_line_anchor() {
        // `#L10-L20` is recognised as a line anchor and stripped; the
        // flag rides along so the consumer can treat the ref as a
        // cross-file code pointer.
        let refs = extract_relative_body_refs("see [x](foo.ts#L10-L20)");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "foo.ts");
        assert!(refs[0].has_line_anchor);

        let refs = extract_relative_body_refs("[y](bar.rs#L42)");
        assert_eq!(refs[0].path, "bar.rs");
        assert!(refs[0].has_line_anchor);
    }

    #[test]
    fn extract_relative_body_refs_preserves_literal_hash_in_filename() {
        // A `#fragment` that does not look like a GitHub line anchor
        // (e.g. `report#draft.pdf`) must NOT be stripped — otherwise a
        // file literally named `report#draft.pdf` is reported as a
        // broken ref to `report`, which is wrong on both ends.
        let refs = extract_relative_body_refs("[r](report#draft.pdf)");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "report#draft.pdf");
        assert!(!refs[0].has_line_anchor);
    }

    #[test]
    fn extract_relative_body_refs_reference_style_link_resolved() {
        // pulldown-cmark follows reference-style links and emits the
        // resolved target as a `Tag::Link`. Behavior change vs the old
        // regex (which only matched inline `[text](url)`). Pin it.
        let body = "see [doc][1]\n\n[1]: attachments/doc.pdf\n";
        assert_eq!(
            paths(extract_relative_body_refs(body)),
            vec!["attachments/doc.pdf".to_string()]
        );
    }

    #[test]
    fn extract_relative_body_refs_autolink_url_is_filtered() {
        // CommonMark autolinks `<https://…>` become Tag::Link in
        // pulldown-cmark (another behavior change from the regex era).
        // The `scheme:` check filters them; pin so a future relaxation
        // doesn't quietly start probing the filesystem for hostnames.
        let body = "ping <https://example.com> and <mailto:x@y.z>";
        assert!(extract_relative_body_refs(body).is_empty());
    }

    #[test]
    fn is_line_anchor_recognises_github_shapes() {
        assert!(is_line_anchor("L10"));
        assert!(is_line_anchor("L1-L99"));
        assert!(!is_line_anchor(""));
        assert!(!is_line_anchor("L"));
        assert!(!is_line_anchor("L10-"));
        assert!(!is_line_anchor("section"));
        assert!(!is_line_anchor("Labc"));
        assert!(!is_line_anchor("draft.pdf"));
    }

    #[test]
    fn normalize_related_rejects_garbage() {
        assert!(normalize_related_refs(&["not a slug".to_string()]).is_err());
        assert!(normalize_related_refs(&["@".to_string()]).is_err());
        assert!(normalize_related_refs(&["#abc".to_string()]).is_err());
        assert!(normalize_related_refs(&["foo".to_string()]).is_err());
    }
}
