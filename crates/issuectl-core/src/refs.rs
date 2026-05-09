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

#[cfg(test)]
mod tests {
    use super::*;

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
