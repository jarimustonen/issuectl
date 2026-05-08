//! Canonical hash for issue files (design doc §3.2).
//!
//! Used as the `version` token on the wire. Computed over a normalised
//! projection of the issue (sorted-key JSON of frontmatter excluding
//! `updated:`, plus CRLF-normalised body) so a no-op resave that only
//! bumps `updated:` does not produce a new version.
//!
//! Post-flat-layout (issue `awfully-faint-sound`): status is taken
//! directly from frontmatter — the on-disk path no longer participates
//! in identity, so there is no separate "directory authoritative"
//! projection. One source of truth: `fm.status`.
//!
//! Unknown frontmatter keys (anything outside the schema in
//! `parser::Frontmatter`) are projected through `Issue::extra` and
//! sorted into the canonical map alongside the typed fields. This
//! closes the silent-overwrite gap where a writer that doesn't touch
//! a user-added key like `triage:` would otherwise produce a matching
//! version hash even after the key changed under it.

use std::borrow::Cow;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::models::Issue;

/// Compute the canonical SHA-256 hash for an `Issue`. Same input
/// produces the same output across processes and milestones.
pub fn canonical_hash(issue: &Issue) -> String {
    let json = canonical_frontmatter_value(issue);
    let mut h = Sha256::new();
    // Use serde_json::to_vec with sort_keys via BTreeMap-backed map
    // construction below; the projection function already builds the
    // map with deterministic ordering.
    h.update(serde_json::to_vec(&json).expect("canonical projection cannot fail to serialize"));
    h.update(b"\n---\n");
    h.update(normalize_body(&issue.body).as_bytes());
    format!("sha256:{}", hex::encode(h.finalize()))
}

/// Project the issue's frontmatter into a canonical JSON object.
/// `updated:` is excluded — it is bumped on every save and would
/// re-introduce false-409s. Status comes straight from frontmatter:
/// the post-flat-layout repo has no parallel folder axis to reconcile.
fn canonical_frontmatter_value(issue: &Issue) -> Value {
    let mut m = Map::new();
    m.insert("type".into(), Value::String(issue.issue_type.clone()));
    m.insert("status".into(), Value::String(issue.status.clone()));
    m.insert("priority".into(), Value::String(issue.priority.clone()));
    if let Some(v) = &issue.created {
        m.insert("created".into(), Value::String(v.clone()));
    }
    if let Some(v) = &issue.closed {
        m.insert("closed".into(), Value::String(v.clone()));
    }
    if let Some(v) = &issue.reporter {
        m.insert("reporter".into(), Value::String(v.clone()));
    }
    if let Some(v) = &issue.assignee {
        m.insert("assignee".into(), Value::String(v.clone()));
    }
    if let Some(v) = &issue.owner {
        m.insert("owner".into(), Value::String(v.clone()));
    }
    if let Some(v) = &issue.epic {
        m.insert("epic".into(), Value::String(v.clone()));
    }
    if let Some(v) = &issue.labels {
        m.insert(
            "labels".into(),
            Value::Array(v.iter().cloned().map(Value::String).collect()),
        );
    }
    if let Some(v) = &issue.related {
        m.insert(
            "related".into(),
            Value::Array(v.iter().cloned().map(Value::String).collect()),
        );
    }
    if let Some(v) = &issue.commits {
        m.insert(
            "commits".into(),
            serde_json::to_value(v).expect("Commit serializes"),
        );
    }
    for (k, v) in &issue.extra {
        // `extra` only contains keys outside the schema (serde flatten
        // catches the leftovers in the parser). A YAML value with a
        // non-string mapping key would fail to serialize as JSON and
        // panic here — that is a malformed frontmatter file the user
        // should fix; treating it as a corrupt-file panic is louder
        // than silently dropping the field, which is the bug we are
        // closing.
        m.insert(
            k.clone(),
            serde_json::to_value(v).expect("frontmatter values JSON-serialise"),
        );
    }
    // serde_json::Map preserves insertion order. To get a canonical
    // sort independent of insertion order we collect into a BTreeMap
    // and rebuild.
    let sorted: std::collections::BTreeMap<String, Value> = m.into_iter().collect();
    Value::Object(sorted.into_iter().collect())
}

/// Normalize CRLF→LF and trim only trailing newlines (NOT arbitrary
/// Unicode whitespace — `trim_end()` would strip nbsp / U+2028 / etc.
/// which can be legitimate body content).
fn normalize_body(body: &str) -> Cow<'_, str> {
    let crlf_normalized = if body.contains('\r') {
        Cow::Owned(body.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(body)
    };
    Cow::Owned(crlf_normalized.trim_end_matches('\n').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Issue;

    fn issue(slug: &str, folder: &str, status: &str, body: &str) -> Issue {
        Issue {
            slug: slug.to_string(),
            folder: folder.to_string(),
            created: Some("2026-05-06".to_string()),
            status: status.to_string(),
            updated: None,
            priority: "normal".to_string(),
            issue_type: "bug".to_string(),
            reporter: None,
            assignee: None,
            owner: None,
            epic: None,
            related: None,
            labels: None,
            closed: None,
            commits: None,
            title: String::new(),
            body: body.to_string(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn updated_field_excluded_from_hash() {
        let mut a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        a.updated = Some("2026-05-06".to_string());
        b.updated = Some("2099-01-01".to_string());
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn body_trailing_newlines_normalized() {
        let a = issue("foo", "open", "open", "body");
        let b = issue("foo", "open", "open", "body\n\n\n");
        let c = issue("foo", "open", "open", "body\r\n");
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
        assert_eq!(canonical_hash(&a), canonical_hash(&c));
    }

    #[test]
    fn different_status_changes_hash() {
        let a = issue("foo", "open", "open", "body");
        let b = issue("foo", "open", "in-progress", "body");
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn nbsp_in_body_is_preserved() {
        // U+00A0 NBSP must not be trimmed — only ASCII '\n' and '\r' are
        // candidates for the trailing-newline trim.
        let a = issue("foo", "open", "open", "body\u{00A0}");
        let b = issue("foo", "open", "open", "body");
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn unknown_key_value_changes_hash() {
        let mut a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        a.extra
            .insert("triage".into(), serde_yaml::Value::String("alice".into()));
        b.extra
            .insert("triage".into(), serde_yaml::Value::String("bob".into()));
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn unknown_key_presence_changes_hash() {
        let a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        b.extra
            .insert("reviewer".into(), serde_yaml::Value::String("dana".into()));
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn unknown_key_order_does_not_affect_hash() {
        // BTreeMap iteration is sorted, so insertion order at the
        // call site cannot perturb the hash. Belt-and-braces: assert
        // it directly.
        let mut a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        a.extra
            .insert("triage".into(), serde_yaml::Value::String("x".into()));
        a.extra
            .insert("reviewer".into(), serde_yaml::Value::String("y".into()));
        b.extra
            .insert("reviewer".into(), serde_yaml::Value::String("y".into()));
        b.extra
            .insert("triage".into(), serde_yaml::Value::String("x".into()));
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn hash_is_stable() {
        let i = issue("foo", "open", "open", "body");
        // Hash is deterministic — running it twice must give the same
        // 64-hex string. Format itself is stable across runs.
        assert_eq!(canonical_hash(&i), canonical_hash(&i));
        assert!(canonical_hash(&i).starts_with("sha256:"));
        assert_eq!(canonical_hash(&i).len(), 7 + 64);
    }
}
