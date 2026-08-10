//! Canonical hash for issue files (design doc §3.2).
//!
//! Used as the `version` token on the wire. Computed over a normalised
//! projection of the issue (sorted-key JSON of frontmatter excluding
//! `updated:`, plus CRLF-normalised body) so a no-op resave that only
//! bumps `updated:` does not produce a new version.
//!
//! ## Token format
//!
//! Tokens are emitted as `sha256:v1:<64hex>`. The `v1` segment is a
//! scheme version: when the canonical projection changes in a way
//! that invalidates outstanding tokens, bump it (`v2`, `v3`, ...).
//! Tokens are compared as opaque strings on the hot path — a stale
//! `v1` token presented to a future `v2` binary still surfaces as
//! a plain `VersionMismatch`, not as a typed "old-scheme" error.
//! The marker exists for forensics today (logs and bug reports can
//! distinguish schemes at a glance) and as the foundation for a
//! later `classify(token)` helper if/when the v2 transition needs
//! to reject old-scheme tokens with a distinct error path.
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
    format!("sha256:v1:{}", hex::encode(h.finalize()))
}

/// Project the issue's frontmatter into a canonical JSON object.
/// `updated:` is excluded — it is bumped on every save and would
/// re-introduce false-409s. Status comes straight from frontmatter:
/// the post-flat-layout repo has no parallel folder axis to reconcile.
///
/// `title` is included unconditionally even though `Issue.title` is a
/// non-optional `String` — the parser materialises an absent H1 as
/// `""`, so "no title", `title: ""`, and `title: ~` all collapse to
/// the same projection entry. This is intentional: the body bytes
/// already carry the H1 text, so the empty-vs-absent distinction is
/// preserved at the body level. Treating title as a presence-bearing
/// optional here would only add asymmetry without information gain.
fn canonical_frontmatter_value(issue: &Issue) -> Value {
    let mut m = Map::new();
    m.insert("type".into(), Value::String(issue.issue_type.clone()));
    m.insert("status".into(), Value::String(issue.status.clone()));
    m.insert("priority".into(), Value::String(issue.priority.clone()));
    m.insert("title".into(), Value::String(issue.title.clone()));
    if let Some(v) = &issue.created {
        m.insert("created".into(), Value::String(v.clone()));
    }
    if let Some(v) = &issue.closed {
        m.insert("closed".into(), Value::String(v.clone()));
    }
    // Projected under the same `closed_by` key it used to occupy via
    // `extra`, so promoting it to a typed field leaves the hash of any
    // existing issue unchanged (backward-compatible version tokens).
    if let Some(v) = &issue.closed_by {
        m.insert("closed_by".into(), Value::String(v.clone()));
    }
    // Scheduling-DAG fields, projected under the same keys they occupied
    // via `extra` before promotion — so an issue that never set them adds
    // no entry and hashes identically to the pre-field shape (see
    // `no_lane_collision_hashes_identically`), exactly like `closed_by`.
    if let Some(v) = &issue.lane {
        m.insert("lane".into(), Value::String(v.clone()));
    }
    if let Some(v) = &issue.collision {
        m.insert(
            "collision".into(),
            Value::Array(v.iter().cloned().map(Value::String).collect()),
        );
    }
    // Projected under the same `lane_seq` key it occupied via `extra`
    // before promotion — an integer, so an issue that never set it adds no
    // entry and hashes identically (see `no_lane_seq_hashes_identically`),
    // exactly like `lane`/`collision`.
    if let Some(v) = &issue.lane_seq {
        m.insert("lane_seq".into(), Value::Number((*v).into()));
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
        // `extra` is JSON-shaped (parser converted at the YAML
        // boundary). The parser pre-sorts nested maps, but we
        // re-canonicalise here defensively: an `Issue` constructed
        // any other way (test fixture, future deserialisation under
        // `serde_json/preserve_order`) might carry an
        // insertion-ordered `Map` whose hash would diverge from a
        // logically equal one. The hash is the optimistic-
        // concurrency primitive — it must not trust upstream
        // invariants.
        m.insert(k.clone(), canonicalise_json(v));
    }
    // serde_json::Map preserves insertion order. To get a canonical
    // sort independent of insertion order we collect into a BTreeMap
    // and rebuild.
    let sorted: std::collections::BTreeMap<String, Value> = m.into_iter().collect();
    Value::Object(sorted.into_iter().collect())
}

/// Recursively rebuild a `serde_json::Value` so every map iterates
/// in sorted-key order. Idempotent. Used at hash time so the
/// canonical projection does not depend on whether `serde_json`
/// was compiled with `preserve_order`, nor on how the input
/// `Value` happened to be constructed.
fn canonicalise_json(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let sorted: std::collections::BTreeMap<String, Value> = m
                .iter()
                .map(|(k, vv)| (k.clone(), canonicalise_json(vv)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(canonicalise_json).collect()),
        other => other.clone(),
    }
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
            closed_by: None,
            lane: None,
            collision: None,
            lane_seq: None,
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
    fn canonical_hash_changes_when_title_changes() {
        // Title is frontmatter-derived and a hand-edit must invalidate
        // any in-flight `expected_version` token.
        let mut a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        a.title = "old title".into();
        b.title = "new title".into();
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn title_change_not_masked_by_updated_exclusion() {
        // `updated:` is excluded from the hash; pin that excluding
        // `updated` doesn't accidentally mask a coincident title change.
        let mut a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        a.title = "old".into();
        a.updated = Some("2026-05-06".into());
        b.title = "new".into();
        b.updated = Some("2099-01-01".into());
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn parsed_frontmatter_title_change_changes_hash() {
        // Guard against parser regressions: future refactors that stop
        // populating `Issue.title` from the H1 would still pass the
        // direct-mutation test above. This one goes through the parser.
        use crate::parser::parse_item_md_text_with_warnings;
        use std::path::Path;
        let item_a = "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-05-06\n---\n\n# Old title\n\nbody\n";
        let item_b = "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-05-06\n---\n\n# New title\n\nbody\n";
        let a = parse_item_md_text_with_warnings(item_a, "foo", "open", Path::new("a.md"));
        let b = parse_item_md_text_with_warnings(item_b, "foo", "open", Path::new("b.md"));
        assert_eq!(a.issue.title, "Old title");
        assert_eq!(b.issue.title, "New title");
        assert_ne!(canonical_hash(&a.issue), canonical_hash(&b.issue));
    }

    #[test]
    fn golden_hash_with_title() {
        // Frozen vector: any future drift in the canonical projection,
        // serialisation, or hash framing flips this. Update the
        // expected value with intent (and document the version-token
        // break in the commit message).
        let mut i = issue("foo", "open", "open", "body");
        i.title = "Example title".into();
        assert_eq!(
            canonical_hash(&i),
            "sha256:v1:342ad2308c37e7e3c443bef5d2243800e723955d5d28d75fb6a69de05143d5c4"
        );
    }

    #[test]
    fn typed_closed_by_hashes_same_as_legacy_extra() {
        // Backward compat: promoting `closed_by` from `extra` to a typed
        // field must not change the version token of any existing issue.
        // An issue carrying the closer in the typed slot must hash
        // identically to the pre-promotion shape (same key in `extra`).
        let mut typed = issue("foo", "closed", "done", "body");
        typed.closed_by = Some("jari".to_string());
        let mut legacy = issue("foo", "closed", "done", "body");
        legacy
            .extra
            .insert("closed_by".into(), serde_json::Value::String("jari".into()));
        assert_eq!(canonical_hash(&typed), canonical_hash(&legacy));
    }

    #[test]
    fn no_lane_collision_hashes_identically() {
        // Load-bearing regression: adding the `lane`/`collision` fields
        // must not churn the version token of any issue that sets
        // neither. This frozen vector was computed against the projection
        // BEFORE the two fields were added; if projecting an all-`None`
        // issue ever inserts a `lane`/`collision` map entry, this flips.
        // (`golden_hash_with_title` pins the same guarantee for a
        // title-bearing issue.)
        let base = issue("foo", "open", "open", "body");
        assert_eq!(base.lane, None);
        assert_eq!(base.collision, None);
        // The projected object must carry no scheduling keys at all, so
        // the byte input to the hash is identical to the pre-field shape.
        let projected = canonical_frontmatter_value(&base);
        let obj = projected.as_object().expect("projection is an object");
        assert!(!obj.contains_key("lane"), "absent lane must not project");
        assert!(
            !obj.contains_key("collision"),
            "absent collision must not project"
        );
        // Frozen pre-fields vector: this hash was produced by the
        // projection BEFORE `lane`/`collision` existed. If projecting an
        // all-`None` issue ever inserts a scheduling key, the byte input
        // changes and this flips — the actual backward-compat guard, not a
        // tautology.
        assert_eq!(
            canonical_hash(&base),
            "sha256:v1:6be6f7521e3e0f1390a8271a959e792c98a97b440909134e04fad66c8dc8b4dd"
        );
    }

    #[test]
    fn lane_presence_changes_hash() {
        let a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        b.lane = Some("schema".to_string());
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn lane_value_changes_hash() {
        let mut a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        a.lane = Some("schema".to_string());
        b.lane = Some("main-rs".to_string());
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn collision_presence_and_value_change_hash() {
        let a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        b.collision = Some(vec!["a.rs".to_string()]);
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
        let mut c = issue("foo", "open", "open", "body");
        c.collision = Some(vec!["b.rs".to_string()]);
        assert_ne!(canonical_hash(&b), canonical_hash(&c));
    }

    #[test]
    fn typed_lane_collision_hash_same_as_legacy_extra() {
        // Backward compat, mirroring `typed_closed_by_hashes_same_as_legacy_extra`:
        // an issue carrying `lane`/`collision` in the typed slots must hash
        // identically to the pre-promotion shape (same keys in `extra`).
        let mut typed = issue("foo", "open", "open", "body");
        typed.lane = Some("schema".to_string());
        typed.collision = Some(vec!["a.rs".to_string(), "b.rs".to_string()]);
        let mut legacy = issue("foo", "open", "open", "body");
        legacy
            .extra
            .insert("lane".into(), serde_json::Value::String("schema".into()));
        legacy
            .extra
            .insert("collision".into(), serde_json::json!(["a.rs", "b.rs"]));
        assert_eq!(canonical_hash(&typed), canonical_hash(&legacy));
    }

    #[test]
    fn no_lane_seq_hashes_identically() {
        // Load-bearing regression, mirroring `no_lane_collision_hashes_identically`:
        // adding the `lane_seq` field must not churn the version token of
        // any issue that never sets it. An all-`None` projection must carry
        // no `lane_seq` key, so its hash matches the pre-field byte input.
        let base = issue("foo", "open", "open", "body");
        assert_eq!(base.lane_seq, None);
        let projected = canonical_frontmatter_value(&base);
        let obj = projected.as_object().expect("projection is an object");
        assert!(
            !obj.contains_key("lane_seq"),
            "absent lane_seq must not project"
        );
        // Same frozen pre-fields vector as `no_lane_collision_hashes_identically`:
        // `lane_seq` is projected only when `Some`, so it cannot perturb the
        // all-`None` shape.
        assert_eq!(
            canonical_hash(&base),
            "sha256:v1:6be6f7521e3e0f1390a8271a959e792c98a97b440909134e04fad66c8dc8b4dd"
        );
    }

    #[test]
    fn lane_seq_presence_and_value_change_hash() {
        let a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        b.lane_seq = Some(10);
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
        let mut c = issue("foo", "open", "open", "body");
        c.lane_seq = Some(20);
        assert_ne!(canonical_hash(&b), canonical_hash(&c));
    }

    #[test]
    fn typed_lane_seq_hash_same_as_legacy_extra() {
        // Backward compat: an issue carrying `lane_seq` in the typed slot
        // must hash identically to the pre-promotion shape, where it rode
        // through `extra` as a JSON number. Covers positive, negative,
        // zero, and both `i64` extremes so the projection is checked across
        // the sign boundary, not just for one value.
        for v in [7i64, -3, 0, i64::MAX, i64::MIN] {
            let mut typed = issue("foo", "open", "open", "body");
            typed.lane_seq = Some(v);
            let mut legacy = issue("foo", "open", "open", "body");
            legacy.extra.insert("lane_seq".into(), serde_json::json!(v));
            assert_eq!(
                canonical_hash(&typed),
                canonical_hash(&legacy),
                "typed lane_seq {v} must hash like the legacy extra shape"
            );
        }
    }

    #[test]
    fn parsed_yaml_lane_seq_hashes_like_legacy_extra() {
        // Guards against the "legacy-compat passes only by coincidence"
        // trap: prove the actual parser YAML→typed path yields the same
        // canonical bytes as an issue that carried `lane_seq` in `extra`
        // before promotion. If serde_yaml ever deserialised `lane_seq: 7`
        // as a float (not an integer), the parser would not lift it and
        // this equality would break — catching the divergence.
        use crate::parser::parse_item_md_text_with_warnings;
        use std::path::Path;
        let text = "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-05-06\nlane_seq: 7\n---\n\n# T\n\nbody\n";
        let parsed = parse_item_md_text_with_warnings(text, "foo", "open", Path::new("a.md"));
        assert_eq!(parsed.issue.lane_seq, Some(7));

        // Build the legacy shape from the *same* parsed issue (identical
        // body/title/dates), moving `lane_seq` back into `extra` — so the
        // only thing under test is the frontmatter projection of that one
        // field, not any body difference.
        let mut legacy = parsed.issue.clone();
        legacy.lane_seq = None;
        legacy.extra.insert("lane_seq".into(), serde_json::json!(7));
        assert_eq!(canonical_hash(&parsed.issue), canonical_hash(&legacy));
    }

    #[test]
    fn closed_by_value_changes_hash() {
        let mut a = issue("foo", "closed", "done", "body");
        let mut b = issue("foo", "closed", "done", "body");
        a.closed_by = Some("jari".to_string());
        b.closed_by = Some("alice".to_string());
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn closed_by_presence_changes_hash() {
        let a = issue("foo", "closed", "done", "body");
        let mut b = issue("foo", "closed", "done", "body");
        b.closed_by = Some("jari".to_string());
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
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
            .insert("triage".into(), serde_json::Value::String("alice".into()));
        b.extra
            .insert("triage".into(), serde_json::Value::String("bob".into()));
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn unknown_key_presence_changes_hash() {
        let a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        b.extra
            .insert("reviewer".into(), serde_json::Value::String("dana".into()));
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn unknown_top_level_btreemap_insertion_order_does_not_affect_hash() {
        // BTreeMap iteration is sorted, so insertion order at the
        // call site cannot perturb the hash. The interesting
        // canonicalisation case (nested maps under an unknown key)
        // is exercised by `unknown_nested_map_order_does_not_affect_hash`
        // below.
        let mut a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        a.extra
            .insert("triage".into(), serde_json::Value::String("x".into()));
        a.extra
            .insert("reviewer".into(), serde_json::Value::String("y".into()));
        b.extra
            .insert("reviewer".into(), serde_json::Value::String("y".into()));
        b.extra
            .insert("triage".into(), serde_json::Value::String("x".into()));
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn unknown_key_removal_changes_hash() {
        // Mirrors `unknown_key_presence_changes_hash` from the other
        // direction. A concurrent writer who deletes a custom key
        // must invalidate stale `expected_version`s.
        let mut a = issue("foo", "open", "open", "body");
        a.extra
            .insert("triage".into(), serde_json::Value::String("alice".into()));
        let b = issue("foo", "open", "open", "body");
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn unknown_nested_map_value_changes_hash() {
        let mut a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        a.extra.insert(
            "triage".into(),
            serde_json::json!({ "reviewer": "alice", "eta": "2026-06-01" }),
        );
        b.extra.insert(
            "triage".into(),
            serde_json::json!({ "reviewer": "bob", "eta": "2026-06-01" }),
        );
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn unknown_nested_map_order_does_not_affect_hash() {
        // Two YAML documents differing only in nested-key insertion
        // order must produce the same canonical hash. Today this
        // works in practice because `serde_json::Map` defaults to
        // BTreeMap-backed (sorted) storage, but a future
        // `preserve_order` feature flip in the dep graph would
        // silently break it. The parser pre-sorts via
        // `yaml_to_canonical_json`, locking the invariant in.
        let yaml_a = "x:\n  zebra: 1\n  apple: 2\n  mango: 3\n";
        let yaml_b = "x:\n  mango: 3\n  apple: 2\n  zebra: 1\n";
        let va: serde_yaml::Value = serde_yaml::from_str(yaml_a).unwrap();
        let vb: serde_yaml::Value = serde_yaml::from_str(yaml_b).unwrap();
        // Round-trip both through the parser's YAML→JSON converter
        // by faking an issue with one unknown key.
        let mut a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        a.extra.insert(
            "triage".into(),
            crate::parser::yaml_to_canonical_json(&va).unwrap(),
        );
        b.extra.insert(
            "triage".into(),
            crate::parser::yaml_to_canonical_json(&vb).unwrap(),
        );
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn manually_unsorted_nested_json_map_in_extra_canonicalises() {
        // The parser pre-sorts nested maps via `yaml_to_canonical_json`,
        // so the YAML→JSON path is order-stable. This test bypasses
        // the parser and constructs an `extra` entry directly with a
        // deliberately insertion-ordered `serde_json::Map` to verify
        // the *hash-time* recursive canonicalisation
        // (`canonicalise_json`) actually re-sorts. Without that
        // defence, an `Issue` built from any non-parser source
        // (future API deserialisation, test fixture, etc.) could
        // hash differently from a logically equal one.
        let mut a = issue("foo", "open", "open", "body");
        let mut b = issue("foo", "open", "open", "body");
        let mut unsorted = serde_json::Map::new();
        unsorted.insert("zebra".into(), serde_json::json!(1));
        unsorted.insert("apple".into(), serde_json::json!(2));
        let mut sorted = serde_json::Map::new();
        sorted.insert("apple".into(), serde_json::json!(2));
        sorted.insert("zebra".into(), serde_json::json!(1));
        a.extra
            .insert("triage".into(), serde_json::Value::Object(unsorted));
        b.extra
            .insert("triage".into(), serde_json::Value::Object(sorted));
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn unknown_scalar_types_round_trip_through_yaml_to_canonical_json() {
        // The yaml→canonical-json conversion is exercised at the
        // type level for booleans, integers, floats, sequences, and
        // null — earlier tests only used string scalars.
        use crate::parser::yaml_to_canonical_json;
        let cases = [
            ("true", serde_json::json!(true)),
            ("42", serde_json::json!(42)),
            ("3.5", serde_json::json!(3.5)),
            ("~", serde_json::Value::Null),
            ("[a, b]", serde_json::json!(["a", "b"])),
            ("{a: 1, b: [c]}", serde_json::json!({ "a": 1, "b": ["c"] })),
        ];
        for (yaml, expected) in cases {
            let v: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
            let got = yaml_to_canonical_json(&v).unwrap();
            assert_eq!(got, expected, "yaml {:?} produced {:?}", yaml, got);
        }
    }

    #[test]
    fn unknown_key_set_to_null_hashes_distinct_from_absent() {
        // `triage: ~` (explicit null) and an absent `triage:` key
        // must hash differently — the projection includes an entry
        // for the explicit-null case, which is the user-visible
        // semantic difference between "intentionally cleared" and
        // "never set".
        let mut a = issue("foo", "open", "open", "body");
        let b = issue("foo", "open", "open", "body");
        a.extra.insert("triage".into(), serde_json::Value::Null);
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn yaml_tag_in_unknown_value_is_rejected() {
        use crate::parser::yaml_to_canonical_json;
        let v: serde_yaml::Value = serde_yaml::from_str("!mytag foo").unwrap();
        let err = yaml_to_canonical_json(&v).unwrap_err();
        assert!(
            err.contains("tag") && err.contains("mytag"),
            "expected tag error, got: {err}"
        );
    }

    #[test]
    fn version_token_carries_scheme_marker() {
        // The `v1` segment lets future projection breaks bump to `v2`
        // and reject stale tokens explicitly, instead of silently
        // colliding with the new namespace.
        let i = issue("foo", "open", "open", "body");
        let v = canonical_hash(&i);
        let parts: Vec<&str> = v.splitn(3, ':').collect();
        assert_eq!(parts.len(), 3, "expected algo:version:hex, got {v}");
        assert_eq!(parts[0], "sha256");
        assert_eq!(parts[1], "v1");
        assert_eq!(parts[2].len(), 64);
        assert!(parts[2].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_is_stable() {
        let i = issue("foo", "open", "open", "body");
        // Hash is deterministic — running it twice must give the same
        // 64-hex string. Format itself is stable across runs.
        assert_eq!(canonical_hash(&i), canonical_hash(&i));
        assert!(canonical_hash(&i).starts_with("sha256:v1:"));
        assert_eq!(canonical_hash(&i).len(), "sha256:v1:".len() + 64);
    }
}
