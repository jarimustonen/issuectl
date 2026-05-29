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
/// email local part like `jari@old-host`) is skipped so we only touch
/// standalone `@slug` mentions.
pub(crate) fn rewrite_body_refs(body: &str, old: &str, new: &str) -> (String, usize) {
    let mut out = String::with_capacity(body.len());
    let mut count = 0usize;
    let mut prev: Option<char> = None;
    let mut chars = body.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if ch == '@' {
            let prev_ok = prev.map(|c| !c.is_alphanumeric()).unwrap_or(true);
            let rest = &body[idx + ch.len_utf8()..];
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
    (out, count)
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
    fn rewrite_body_refs_skips_email_local_parts() {
        let (out, n) = rewrite_body_refs(
            "mail jari@old-tame-fox but ping @old-tame-fox",
            "old-tame-fox",
            "new-wild-elk",
        );
        assert_eq!(n, 1);
        assert_eq!(out, "mail jari@old-tame-fox but ping @new-wild-elk");
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

    #[test]
    fn normalize_related_rejects_garbage() {
        assert!(normalize_related_refs(&["not a slug".to_string()]).is_err());
        assert!(normalize_related_refs(&["@".to_string()]).is_err());
        assert!(normalize_related_refs(&["#abc".to_string()]).is_err());
        assert!(normalize_related_refs(&["foo".to_string()]).is_err());
    }
}
