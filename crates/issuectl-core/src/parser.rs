use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Deserializer};

#[derive(Debug, Deserialize, Default)]
pub struct Frontmatter {
    pub created: Option<String>,
    pub updated: Option<String>,
    #[serde(rename = "type")]
    pub issue_type: Option<String>,
    pub reporter: Option<String>,
    pub assignee: Option<String>,
    pub owner: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    /// Epic reference. Accepts either a slug string or a legacy numeric value
    /// (the latter is retained only so that `issuectl doctor --fix` can read
    /// pre-migration files).
    #[serde(default, deserialize_with = "deser_epic")]
    pub epic: Option<String>,
    pub related: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,
    pub closed: Option<String>,
    pub commits: Option<Vec<super::models::Commit>>,
    /// Slug stored in frontmatter (post-migration files). Authoritative
    /// identifier is still the directory name; this is mirrored for clarity.
    #[allow(dead_code)]
    pub slug: Option<String>,
    /// Legacy numeric id; preserved only so doctor can read pre-migration files.
    #[allow(dead_code)]
    pub number: Option<u32>,
    /// Any frontmatter keys outside the schema above. Captured so they
    /// participate in `canonical_hash` (design doc §3.2) — without this
    /// a user-added key like `triage:` would not contribute to the
    /// version, and a writer that doesn't touch it could silently
    /// overwrite a concurrent edit. BTreeMap keeps the projection
    /// stable across processes.
    ///
    /// WARNING: `context.rs::read_blocked_by` reads `blocked_by` out of
    /// the `Issue.extra` map that this field feeds. Promoting
    /// `blocked_by` to a typed field above would make serde consume it
    /// here before `extra` is built, silently dropping it from the
    /// context bundle. If you add it, update `read_blocked_by` to read
    /// the typed field instead.
    ///
    /// `closed_by` is likewise intentionally NOT a typed field here even
    /// though `Issue::closed_by` is: a typed `Option<String>` would make
    /// a hand-edited non-string `closed_by:` fail the whole-frontmatter
    /// deserialize (defaulting every other field). Instead the string
    /// value is lifted out of this map into the typed slot after parsing
    /// (see `parse_item_md_text_with_warnings`), leaving any non-string
    /// value safely in `extra`.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, serde_yaml::Value>,
}

fn deser_epic<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    use serde::de::Error;
    let v = Option::<serde_yaml::Value>::deserialize(d)?;
    let Some(val) = v else { return Ok(None) };
    match val {
        serde_yaml::Value::Null => Ok(None),
        serde_yaml::Value::String(s) => {
            if s.trim().is_empty() {
                Err(D::Error::custom(
                    "epic: blank string is not a valid slug; \
                     omit the field or use `epic: ~` to clear",
                ))
            } else {
                Ok(Some(s))
            }
        }
        // Legacy *integer* refs (pre-slug repos) are still accepted so
        // `issuectl doctor --fix` can read them; a warning is emitted
        // downstream in `parse_item_md_text_with_warnings`. Floats
        // (`epic: 3.14`) are rejected — there is no legacy in which an
        // issue ID is a float.
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Some(i.to_string()))
            } else if let Some(u) = n.as_u64() {
                Ok(Some(u.to_string()))
            } else {
                Err(D::Error::custom(
                    "epic: legacy numeric refs must be integers; floats are not valid",
                ))
            }
        }
        serde_yaml::Value::Bool(_)
        | serde_yaml::Value::Sequence(_)
        | serde_yaml::Value::Mapping(_)
        | serde_yaml::Value::Tagged(_) => Err(D::Error::custom(format!(
            "epic: expected a slug string (e.g. `epic: my-slug` or `epic: @my-slug`), \
             got {}",
            shape_name(&val),
        ))),
    }
}

fn shape_name(v: &serde_yaml::Value) -> &'static str {
    match v {
        serde_yaml::Value::Null => "null",
        serde_yaml::Value::Bool(_) => "bool",
        serde_yaml::Value::Number(_) => "number",
        serde_yaml::Value::String(_) => "string",
        serde_yaml::Value::Sequence(_) => "sequence",
        serde_yaml::Value::Mapping(_) => "mapping",
        serde_yaml::Value::Tagged(_) => "tagged value",
    }
}

/// Lossy parse result with per-issue warnings collected instead of
/// stderr-printed. The web API surfaces these in the response so the UI
/// can flag broken issues; the CLI continues to use the wrapper below
/// which prints them to stderr for backwards compatibility.
pub struct ParsedItem {
    pub issue: crate::models::Issue,
    pub warnings: Vec<String>,
    /// Raw frontmatter mapping. `None` when the file has no
    /// `---...---` block or the YAML is unparseable. Exposed so
    /// callers (e.g. `doctor`) can do mapping-level checks without
    /// re-parsing the YAML.
    pub mapping: Option<serde_yaml::Mapping>,
    /// True when text exists but no `---...---` block was present.
    pub fm_missing: bool,
    /// `Some(msg)` when the frontmatter block was found but YAML
    /// parsing into a `Mapping` failed.
    pub fm_yaml_error: Option<String>,
    /// `Some(msg)` when the YAML parsed as a mapping but the typed
    /// `Frontmatter` deserialisation failed (wrong shape — e.g.
    /// `created: [1,2,3]` where a string was expected). Treated as a
    /// HARD parse error by `doctor` — doctor cannot safely rewrite
    /// frontmatter whose typed shape it could not understand.
    pub fm_typed_error: Option<String>,
}

impl ParsedItem {
    /// True when the frontmatter could not be read as a typed
    /// `Frontmatter`. Used by `doctor` to classify parse errors as
    /// HARD (block `--fix`) vs SOFT (proceed; migration may heal).
    /// Replaces the older substring-match on warning text.
    pub fn has_hard_frontmatter_error(&self) -> bool {
        self.fm_missing || self.fm_yaml_error.is_some() || self.fm_typed_error.is_some()
    }
}

pub fn parse_item_md_with_warnings(path: &Path, slug: &str, folder: &str) -> ParsedItem {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            return ParsedItem {
                issue: default_issue(slug, folder),
                warnings: vec![format!("cannot read {}: {}", path.display(), e)],
                mapping: None,
                fm_missing: false,
                fm_yaml_error: None,
                fm_typed_error: None,
            };
        }
    };
    parse_item_md_text_with_warnings(&text, slug, folder, path)
}

/// Variant of `parse_item_md_with_warnings` that takes already-loaded
/// text. The watcher uses this so a single read of `item.md` produces
/// both the parsed `Issue` and the canonical hash, eliminating TOCTOU
/// between separate read syscalls.
pub fn parse_item_md_text_with_warnings(
    text: &str,
    slug: &str,
    folder: &str,
    source: &Path,
) -> ParsedItem {
    let mut warnings = Vec::new();
    let (frontmatter, body) = split_frontmatter(text);
    // D7: parse YAML once into `Mapping`, then derive the typed
    // `Frontmatter` from the parsed value rather than re-parsing the
    // text. Both products are exposed via `ParsedItem` so callers like
    // `doctor` don't have to parse the same string twice.
    let mut mapping: Option<serde_yaml::Mapping> = None;
    let mut fm_missing = false;
    let mut fm_yaml_error: Option<String> = None;
    let mut fm_typed_error: Option<String> = None;
    let fm = match frontmatter {
        None => {
            fm_missing = true;
            Frontmatter::default()
        }
        Some(yaml_text) => match serde_yaml::from_str::<serde_yaml::Mapping>(yaml_text) {
            Ok(m) => {
                let value = serde_yaml::Value::Mapping(m.clone());
                mapping = Some(m);
                match serde_yaml::from_value::<Frontmatter>(value) {
                    Ok(fm) => fm,
                    Err(e) => {
                        let msg =
                            format!("invalid YAML frontmatter in {}: {}", source.display(), e);
                        fm_typed_error = Some(msg.clone());
                        warnings.push(msg);
                        Frontmatter::default()
                    }
                }
            }
            Err(e) => {
                fm_yaml_error = Some(format!("invalid frontmatter YAML: {e}"));
                warnings.push(format!(
                    "invalid YAML frontmatter in {}: {}",
                    source.display(),
                    e
                ));
                Frontmatter::default()
            }
        },
    };

    // Surface legacy numeric epic refs as a warning instead of an
    // unconditional stderr print — the doctor --fix pass migrates these.
    if let Some(ref e) = fm.epic {
        if !e.is_empty() && e.chars().all(|c| c.is_ascii_digit()) {
            warnings.push(format!(
                "{}: epic: {} is a legacy numeric ref — run `issuectl doctor --fix`",
                source.display(),
                e
            ));
        }
    }

    // Convert unknown frontmatter values from YAML AST to JSON AST
    // at this boundary. JSON cannot represent every YAML construct
    // (non-string mapping keys, tags, etc.); converting here lets us
    // surface those as warnings instead of panicking deep in
    // canonical_hash or the HTTP serializer. Failed entries are
    // dropped from `extra` and reported via the warnings list, which
    // mutate.rs treats as `MutateError::Corrupt` — the file isn't
    // overwritten and the user can fix it.
    let mut extra = BTreeMap::new();
    for (k, v) in fm.unknown {
        match yaml_to_canonical_json(&v) {
            Ok(json) => {
                extra.insert(k, json);
            }
            Err(e) => warnings.push(format!(
                "{}: unsupported value for unknown frontmatter key {:?}: {}",
                source.display(),
                k,
                e
            )),
        }
    }

    // Promote `closed_by` from the unknown-key map into the typed
    // `Issue::closed_by` field. Deliberately NOT a typed `Frontmatter`
    // field (same reasoning as `blocked_by` — see the `unknown` doc):
    // declaring it there would make serde reject a hand-edited non-string
    // `closed_by:` at the whole-frontmatter level, defaulting every other
    // typed field. Lifting a *string* value here (and only a string)
    // keeps the domain model typed while leaving any non-string legacy
    // value in `extra`, where it stays readable and hashes exactly as it
    // did before promotion. Removing the promoted string from `extra`
    // gives the field one representation on the wire and in the hash.
    let closed_by = match extra.get("closed_by") {
        Some(serde_json::Value::String(_)) => match extra.remove("closed_by") {
            Some(serde_json::Value::String(s)) => Some(s),
            _ => None,
        },
        _ => None,
    };

    // Promote the scheduling-DAG fields out of the unknown-key map into
    // their typed slots, mirroring `closed_by` above. Only a well-typed
    // shape is lifted — a *string* `lane:` and a *list of strings*
    // `collision:`; any other shape (a hand-edited `lane: [oops]`, a
    // `collision: bare-string`) stays in `extra` where it remains
    // readable and hashes exactly as it did before promotion. Removing
    // the lifted value from `extra` keeps one representation on the wire
    // and in the hash.
    // Lift only a *well-formed, non-empty* string. A hand-edited
    // `lane: ""` / `lane: "  "` (or a non-string shape) is treated as
    // malformed and left in `extra` — never promoted to a real empty
    // lane, which would otherwise pollute the scheduling view.
    let lane = match extra.get("lane") {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => match extra.remove("lane") {
            Some(serde_json::Value::String(s)) => Some(s),
            _ => None,
        },
        _ => None,
    };
    // Lift only a non-empty list whose every element is a non-empty
    // string. An empty list, a non-string element, or a whitespace-only
    // token leaves the whole value in `extra` (malformed / no-op).
    let collision = match extra.get("collision") {
        Some(serde_json::Value::Array(items))
            if !items.is_empty()
                && items
                    .iter()
                    .all(|v| matches!(v, serde_json::Value::String(s) if !s.trim().is_empty())) =>
        {
            match extra.remove("collision") {
                Some(serde_json::Value::Array(items)) => Some(
                    items
                        .into_iter()
                        .filter_map(|v| match v {
                            serde_json::Value::String(s) => Some(s),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            }
        }
        _ => None,
    };
    // Promote an *integer* `lane_seq:` out of `extra` into its typed slot,
    // mirroring `lane` above. Only a JSON integer is lifted — a float, a
    // string, or any other shape stays in `extra` where it remains
    // readable and hashes exactly as it did before promotion.
    let lane_seq = match extra.get("lane_seq") {
        Some(serde_json::Value::Number(n)) if n.is_i64() => {
            let v = n.as_i64();
            extra.remove("lane_seq");
            v
        }
        _ => None,
    };
    // A present-but-unliftable `lane_seq` (a string, float, list, or an
    // integer outside `i64` range) is left in `extra` and silently has no
    // scheduling effect. Surface it as a load warning — mirroring the
    // legacy-epic warning above — so a typo like `lane_seq: "10"` doesn't
    // quietly defeat the intended intra-lane ordering.
    if lane_seq.is_none() && extra.contains_key("lane_seq") {
        warnings.push(format!(
            "{}: lane_seq must be an integer to affect `issuectl dag` ordering — \
             the current value is ignored (set it with `issuectl update --lane-seq <int>`)",
            source.display()
        ));
    }

    let title = extract_title(body);
    let issue = crate::models::Issue {
        slug: slug.to_string(),
        folder: folder.to_string(),
        created: fm.created,
        status: fm.status.unwrap_or_else(|| "open".to_string()),
        updated: fm.updated,
        priority: fm.priority.unwrap_or_else(|| "normal".to_string()),
        issue_type: fm.issue_type.unwrap_or_else(|| "bug".to_string()),
        reporter: fm.reporter,
        assignee: fm.assignee,
        owner: fm.owner,
        epic: fm.epic,
        related: fm.related,
        labels: fm.labels,
        closed: fm.closed,
        closed_by,
        lane,
        collision,
        lane_seq,
        commits: fm.commits,
        extra,
        title,
        body: body.unwrap_or_default().trim().to_string(),
    };
    ParsedItem {
        issue,
        warnings,
        mapping,
        fm_missing,
        fm_yaml_error,
        fm_typed_error,
    }
}

/// Convert a `serde_yaml::Value` into a JSON-shaped value with
/// recursively sorted maps. Mapping keys that are not strings, YAML
/// tags, and any other YAML construct that JSON cannot represent
/// produce an `Err` — callers should record a parse warning and
/// drop the offending entry. Recursive sort guarantees the output
/// bytes are stable across processes regardless of insertion order
/// (matches design doc §3.2's "sorted-key JSON" contract).
pub(crate) fn yaml_to_canonical_json(v: &serde_yaml::Value) -> Result<serde_json::Value, String> {
    use serde_json::Value as J;
    match v {
        serde_yaml::Value::Null => Ok(J::Null),
        serde_yaml::Value::Bool(b) => Ok(J::Bool(*b)),
        serde_yaml::Value::String(s) => Ok(J::String(s.clone())),
        serde_yaml::Value::Number(n) => {
            // Non-finite floats (NaN, Infinity) cannot round-trip
            // through JSON; reject explicitly. `serde_json::to_value`
            // happens to map NaN → Null silently, which would let two
            // distinct frontmatters hash identically — surfacing an
            // error here is the correct contract.
            if let Some(i) = n.as_i64() {
                Ok(J::Number(serde_json::Number::from(i)))
            } else if let Some(u) = n.as_u64() {
                Ok(J::Number(serde_json::Number::from(u)))
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map(J::Number)
                    .ok_or_else(|| format!("non-finite float ({f}) is not JSON-representable"))
            } else {
                Err("YAML number is not representable as i64/u64/f64".into())
            }
        }
        serde_yaml::Value::Sequence(xs) => xs
            .iter()
            .map(yaml_to_canonical_json)
            .collect::<Result<Vec<_>, _>>()
            .map(J::Array),
        serde_yaml::Value::Mapping(m) => {
            let mut sorted: BTreeMap<String, J> = BTreeMap::new();
            for (k, vv) in m {
                let key = match k {
                    serde_yaml::Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "mapping key {:?} must be a string in canonical JSON",
                            other
                        ));
                    }
                };
                sorted.insert(key, yaml_to_canonical_json(vv)?);
            }
            Ok(J::Object(sorted.into_iter().collect()))
        }
        serde_yaml::Value::Tagged(t) => Err(format!(
            "YAML tag {} is not supported in unknown frontmatter keys",
            t.tag
        )),
    }
}

fn default_issue(slug: &str, folder: &str) -> crate::models::Issue {
    crate::models::Issue {
        slug: slug.to_string(),
        folder: folder.to_string(),
        created: None,
        status: "open".to_string(),
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
        extra: BTreeMap::new(),
        title: String::new(),
        body: String::new(),
    }
}

pub fn parse_item_md(path: &Path, slug: &str, folder: &str) -> crate::models::Issue {
    let parsed = parse_item_md_with_warnings(path, slug, folder);
    for w in &parsed.warnings {
        eprintln!("Warning: {w}");
    }
    parsed.issue
}

/// Split a markdown text into frontmatter and body. The opening and
/// closing `---` markers must each occupy their own line — anything
/// like `---foo` or `----` is treated as body content, not a marker.
/// This delegates to the shared strict splitter in `item_text` so the
/// reader, writer, formatter and merge driver agree on exactly the
/// same boundary.
pub(crate) fn split_frontmatter(text: &str) -> (Option<&str>, Option<&str>) {
    let split = crate::item_text::split(text);
    (split.frontmatter, Some(split.body))
}

pub(crate) fn extract_title(body: Option<&str>) -> String {
    let Some(body) = body else {
        return String::new();
    };
    crate::body_sections::title_heading(body)
        .map(|(_, title)| strip_legacy_title_number(title).trim().to_string())
        .unwrap_or_default()
}

fn strip_legacy_title_number(title: &str) -> &str {
    // Legacy headings are `# E10. Title` or `# 10. Title`. The `E`
    // prefix is meaningful only when the rest parses as `<digits>. <rest>`.
    // Returning the un-stripped original on the no-match path keeps a
    // plain title like `# Esimiehen …` intact.
    let candidate = title.strip_prefix('E').unwrap_or(title);
    let Some((number, rest)) = candidate.split_once(". ") else {
        return title;
    };
    if !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit()) {
        rest
    } else {
        title
    }
}

/// Parse a legacy `<NN>-<slug>` directory name into its numeric prefix and
/// trailing slug. Used only by `issuectl doctor` for migration.
pub fn parse_legacy_dir(dirname: &str) -> Option<(u32, String)> {
    let hyphen = dirname.find('-')?;
    let num_part = &dirname[..hyphen];
    let number: u32 = num_part.parse().ok()?;
    let slug = dirname[hyphen + 1..].to_string();
    Some((number, slug))
}

#[cfg(test)]
mod tests {
    use super::strip_legacy_title_number;

    #[test]
    fn strips_legacy_e_prefix_form() {
        assert_eq!(strip_legacy_title_number("E10. Foo bar"), "Foo bar");
    }

    #[test]
    fn strips_legacy_numeric_form() {
        assert_eq!(strip_legacy_title_number("10. Foo bar"), "Foo bar");
    }

    #[test]
    fn keeps_plain_title_starting_with_e() {
        // Regression: `# Esimiehen ...` was being rendered as
        // `simiehen ...` because `E` was stripped unconditionally.
        assert_eq!(
            strip_legacy_title_number("Esimiehen ennakkolupa-flow"),
            "Esimiehen ennakkolupa-flow"
        );
    }

    #[test]
    fn keeps_plain_title_without_legacy_shape() {
        assert_eq!(strip_legacy_title_number("Foo bar"), "Foo bar");
    }

    #[test]
    fn keeps_e_prefixed_title_without_dot_number() {
        assert_eq!(strip_legacy_title_number("Eager parser"), "Eager parser");
    }

    use super::{parse_item_md_text_with_warnings, split_frontmatter};
    use std::path::Path;

    #[test]
    fn closed_by_lifts_into_typed_field_not_extra() {
        // Legacy/round-trip: a `closed_by:` frontmatter key (how the
        // pre-typed-field close path wrote it) must land in the typed
        // `Issue::closed_by` slot and NOT in `extra`, so it has exactly
        // one representation on the wire and in the hash.
        let text = "---\ntype: bug\nstatus: wontfix\npriority: normal\n\
                    closed: 2026-05-06\nclosed_by: alice\n---\n\n# Title\n";
        let parsed =
            parse_item_md_text_with_warnings(text, "some-slug", "closed", Path::new("<test>"));
        assert_eq!(parsed.issue.closed_by.as_deref(), Some("alice"));
        assert!(
            !parsed.issue.extra.contains_key("closed_by"),
            "closed_by must not remain in extra: {:?}",
            parsed.issue.extra
        );
        assert!(
            parsed.warnings.is_empty(),
            "warnings: {:?}",
            parsed.warnings
        );
    }

    #[test]
    fn absent_closed_by_is_none() {
        let text = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# Title\n";
        let parsed =
            parse_item_md_text_with_warnings(text, "some-slug", "open", Path::new("<test>"));
        assert_eq!(parsed.issue.closed_by, None);
    }

    #[test]
    fn non_string_closed_by_does_not_break_the_typed_parse() {
        // Regression: `closed_by` is lifted from `extra` rather than
        // typed on `Frontmatter`, so a hand-edited non-string value must
        // NOT fail the whole-frontmatter deserialize (which would default
        // every other field). Every typed field still parses; the
        // malformed `closed_by` stays in `extra` (readable + hashed as
        // before) and the typed slot is left empty.
        let text = "---\ntype: feature\nstatus: done\npriority: high\n\
                    closed: 2026-05-06\nclosed_by: 42\n---\n\n# Title\n";
        let parsed =
            parse_item_md_text_with_warnings(text, "some-slug", "closed", Path::new("<test>"));
        assert_eq!(parsed.issue.issue_type, "feature", "type must survive");
        assert_eq!(parsed.issue.status, "done", "status must survive");
        assert_eq!(parsed.issue.priority, "high", "priority must survive");
        assert_eq!(parsed.issue.closed.as_deref(), Some("2026-05-06"));
        assert_eq!(parsed.issue.closed_by, None, "non-string not lifted");
        assert!(
            parsed.issue.extra.contains_key("closed_by"),
            "non-string closed_by stays in extra"
        );
    }

    #[test]
    fn lane_and_collision_lift_into_typed_fields_not_extra() {
        // A string `lane:` and a list-of-strings `collision:` are promoted
        // into the typed slots and stripped from `extra`, exactly like
        // `closed_by`, so there is one wire/hash representation.
        let text = "---\ntype: bug\nstatus: open\npriority: normal\n\
                    lane: schema\ncollision:\n  - a.rs\n  - b.rs\n---\n\n# Title\n";
        let parsed =
            parse_item_md_text_with_warnings(text, "some-slug", "open", Path::new("<test>"));
        assert_eq!(parsed.issue.lane.as_deref(), Some("schema"));
        assert_eq!(
            parsed.issue.collision.as_deref(),
            Some(&["a.rs".to_string(), "b.rs".to_string()][..])
        );
        assert!(
            !parsed.issue.extra.contains_key("lane"),
            "lane must not remain in extra: {:?}",
            parsed.issue.extra
        );
        assert!(
            !parsed.issue.extra.contains_key("collision"),
            "collision must not remain in extra: {:?}",
            parsed.issue.extra
        );
    }

    #[test]
    fn absent_lane_collision_are_none() {
        let text = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# Title\n";
        let parsed =
            parse_item_md_text_with_warnings(text, "some-slug", "open", Path::new("<test>"));
        assert_eq!(parsed.issue.lane, None);
        assert_eq!(parsed.issue.collision, None);
        assert_eq!(parsed.issue.lane_seq, None);
    }

    #[test]
    fn integer_lane_seq_lifts_into_typed_field_not_extra() {
        // An integer `lane_seq:` is promoted into the typed slot and
        // stripped from `extra`, mirroring `lane`. A non-integer shape
        // (float / string) stays in `extra`, readable and hashed as-is.
        let text = "---\ntype: bug\nstatus: open\npriority: normal\nlane_seq: 20\n---\n\n# Title\n";
        let parsed =
            parse_item_md_text_with_warnings(text, "some-slug", "open", Path::new("<test>"));
        assert_eq!(parsed.issue.lane_seq, Some(20));
        assert!(
            !parsed.issue.extra.contains_key("lane_seq"),
            "lane_seq must not remain in extra: {:?}",
            parsed.issue.extra
        );

        // Non-integer shapes stay in `extra` AND raise a load warning so
        // the silent-no-op is surfaced. `9223372036854775808` is
        // `i64::MAX + 1` — an unsigned value `is_i64()` rejects, so it is
        // treated as unliftable exactly like a float/string/list.
        for bad in [
            "lane_seq: 1.5",
            "lane_seq: \"3\"",
            "lane_seq: [1]",
            "lane_seq: 9223372036854775808",
        ] {
            let text =
                format!("---\ntype: bug\nstatus: open\npriority: normal\n{bad}\n---\n\n# T\n");
            let p = parse_item_md_text_with_warnings(&text, "s", "open", Path::new("<t>"));
            assert_eq!(
                p.issue.lane_seq, None,
                "non-integer lane_seq not lifted: {bad}"
            );
            assert!(
                p.issue.extra.contains_key("lane_seq"),
                "malformed lane_seq stays in extra: {bad}"
            );
            assert!(
                p.warnings.iter().any(|w| w.contains("lane_seq")),
                "malformed lane_seq must raise a warning: {bad}"
            );
        }

        // A negative integer is a valid `i64` — lifted (higher precedence),
        // no warning.
        let neg = "---\ntype: bug\nstatus: open\npriority: normal\nlane_seq: -3\n---\n\n# T\n";
        let p = parse_item_md_text_with_warnings(neg, "s", "open", Path::new("<t>"));
        assert_eq!(p.issue.lane_seq, Some(-3));
        assert!(!p.issue.extra.contains_key("lane_seq"));
        assert!(!p.warnings.iter().any(|w| w.contains("lane_seq")));
    }

    #[test]
    fn malformed_lane_collision_stay_in_extra() {
        // A non-string `lane:` and a non-list / non-string-element
        // `collision:` must NOT break the typed parse and must be left in
        // `extra` (readable, hashed as-is) — mirroring the non-string
        // `closed_by` tolerance.
        let text = "---\ntype: bug\nstatus: open\npriority: normal\n\
                    lane: [oops]\ncollision: bare\n---\n\n# Title\n";
        let parsed =
            parse_item_md_text_with_warnings(text, "some-slug", "open", Path::new("<test>"));
        assert_eq!(parsed.issue.lane, None, "list lane not lifted");
        assert_eq!(parsed.issue.collision, None, "scalar collision not lifted");
        assert!(parsed.issue.extra.contains_key("lane"));
        assert!(parsed.issue.extra.contains_key("collision"));

        // A collision list containing a non-string element also stays put.
        let text2 = "---\ntype: bug\nstatus: open\npriority: normal\n\
                     collision:\n  - ok\n  - 42\n---\n\n# Title\n";
        let parsed2 =
            parse_item_md_text_with_warnings(text2, "some-slug", "open", Path::new("<test>"));
        assert_eq!(parsed2.issue.collision, None, "mixed-type list not lifted");
        assert!(parsed2.issue.extra.contains_key("collision"));
    }

    #[test]
    fn empty_or_whitespace_lane_collision_not_lifted() {
        // Hand-edited empty/whitespace scheduling fields must NOT become a
        // real empty lane or an empty typed collision list — they stay in
        // `extra` as malformed input.
        let empty_lane = "---\ntype: bug\nstatus: open\npriority: normal\n\
                          lane: \"\"\n---\n\n# T\n";
        let p = parse_item_md_text_with_warnings(empty_lane, "s", "open", Path::new("<t>"));
        assert_eq!(p.issue.lane, None, "empty lane not lifted");
        assert!(p.issue.extra.contains_key("lane"));

        let ws_lane = "---\ntype: bug\nstatus: open\npriority: normal\n\
                       lane: \"   \"\n---\n\n# T\n";
        let p = parse_item_md_text_with_warnings(ws_lane, "s", "open", Path::new("<t>"));
        assert_eq!(p.issue.lane, None, "whitespace lane not lifted");

        let empty_list = "---\ntype: bug\nstatus: open\npriority: normal\n\
                          collision: []\n---\n\n# T\n";
        let p = parse_item_md_text_with_warnings(empty_list, "s", "open", Path::new("<t>"));
        assert_eq!(p.issue.collision, None, "empty collision list not lifted");

        let ws_token = "---\ntype: bug\nstatus: open\npriority: normal\n\
                        collision:\n  - ok\n  - \"  \"\n---\n\n# T\n";
        let p = parse_item_md_text_with_warnings(ws_token, "s", "open", Path::new("<t>"));
        assert_eq!(p.issue.collision, None, "whitespace token blocks lift");
        assert!(p.issue.extra.contains_key("collision"));
    }

    #[test]
    fn split_frontmatter_does_not_leak_into_body_yaml_block() {
        // Regression: doctor flagged keys inside body ```yaml fenced
        // blocks as "unknown frontmatter keys" because some splitter
        // path matched `\n---` mid-body. Strict splitter must end
        // at the first `---` line that follows the opener and ignore
        // any subsequent `---` (whether in markdown rules or fenced
        // YAML).
        let text = "---\ntype: bug\nstatus: open\n---\n\n# Title\n\n```yaml\nshortname: foo\ncourse_id: 123\n```\n";
        let (fm, body) = split_frontmatter(text);
        let fm = fm.expect("frontmatter present");
        assert!(fm.contains("type: bug"), "fm={fm:?}");
        assert!(fm.contains("status: open"), "fm={fm:?}");
        assert!(
            !fm.contains("shortname"),
            "frontmatter leaked body keys: {fm:?}"
        );
        assert!(
            !fm.contains("course_id"),
            "frontmatter leaked body keys: {fm:?}"
        );
        let body = body.unwrap_or_default();
        assert!(body.contains("shortname: foo"));
        assert!(body.contains("course_id: 123"));
    }

    #[test]
    fn split_frontmatter_requires_closing_marker_on_its_own_line() {
        // `\n---foo` mid-body must NOT be treated as the closing
        // marker. Without strict line-bounding, a stray `---xyz`
        // inside body prose or a fenced block could end frontmatter
        // early, leaking body content into the YAML parse.
        let text =
            "---\nstatus: open\ntype: bug\npriority: normal\n---\n\n# Title\n\n```\n---xyz\n```\n";
        let (fm, _body) = split_frontmatter(text);
        let fm = fm.expect("frontmatter present");
        // Frontmatter must contain only the three real keys.
        assert!(fm.contains("status: open"));
        assert!(fm.contains("type: bug"));
        assert!(fm.contains("priority: normal"));
        assert!(!fm.contains("xyz"));
    }

    #[test]
    fn split_frontmatter_handles_missing_closing_marker_with_dashes_in_body() {
        // The real-world scenario behind virtually-callous-rainstorm:
        // when the user forgets the closing `---` of frontmatter and
        // the body contains a YAML fence with `---` inside, the lazy
        // splitter ate body content into the frontmatter, producing
        // bogus "unknown key" warnings. With the strict splitter, a
        // missing terminator means there is no frontmatter at all.
        let text = "---\nstatus: open\ntype: bug\n\n# Body\n\n```yaml\nshortname: foo\ncourse_id: 123\n---\n```\n";
        let (fm, _body) = split_frontmatter(text);
        // No real closing `---` line — frontmatter must be reported
        // as missing rather than swallowing body content.
        assert!(
            fm.is_none()
                || (!fm.unwrap().contains("shortname") && !fm.unwrap().contains("course_id")),
            "fm leaked body keys: {fm:?}"
        );
    }

    #[test]
    fn parse_item_does_not_pick_up_body_yaml_keys_as_unknown() {
        // End-to-end: doctor uses the parsed mapping to flag unknown
        // keys. Body YAML must NOT show up there.
        let text = "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T\n\n```yaml\nshortname: foo\ncourse_id: 123\n```\n";
        let parsed = parse_item_md_text_with_warnings(text, "slug", "flat", Path::new("/tmp/x.md"));
        let map = parsed.mapping.expect("mapping parsed");
        let keys: Vec<String> = map
            .keys()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        assert!(!keys.iter().any(|k| k == "shortname"), "keys={keys:?}");
        assert!(!keys.iter().any(|k| k == "course_id"), "keys={keys:?}");
    }

    #[test]
    fn deser_epic_accepts_string() {
        let text = "---\ntype: bug\nstatus: open\npriority: normal\nepic: my-epic\n---\n\nbody\n";
        let parsed = parse_item_md_text_with_warnings(text, "s", "flat", Path::new("/tmp/x.md"));
        assert_eq!(parsed.fm_typed_error, None);
        assert_eq!(parsed.issue.epic.as_deref(), Some("my-epic"));
    }

    #[test]
    fn deser_epic_accepts_null_and_missing() {
        for fm in [
            "type: bug\nstatus: open\npriority: normal\nepic: ~\n",
            "type: bug\nstatus: open\npriority: normal\n",
        ] {
            let text = format!("---\n{fm}---\n\nbody\n");
            let parsed =
                parse_item_md_text_with_warnings(&text, "s", "flat", Path::new("/tmp/x.md"));
            assert_eq!(parsed.fm_typed_error, None, "fm={fm}");
            assert_eq!(parsed.issue.epic, None, "fm={fm}");
        }
    }

    #[test]
    fn deser_epic_rejects_empty_string() {
        let text = "---\ntype: bug\nstatus: open\npriority: normal\nepic: \"\"\n---\n\nbody\n";
        let parsed = parse_item_md_text_with_warnings(text, "s", "flat", Path::new("/tmp/x.md"));
        let err = parsed
            .fm_typed_error
            .as_deref()
            .expect("empty-string epic must fail typed parse");
        assert!(err.contains("epic"), "err={err}");
        assert!(err.contains("blank"), "err={err}");
    }

    #[test]
    fn deser_epic_rejects_whitespace_only_string() {
        // Same class as empty — `s.is_empty()` was the only previous
        // guard, leaving `epic: "   "` to be accepted as a real epic
        // string and contaminate canonical hashes.
        let text = "---\ntype: bug\nstatus: open\npriority: normal\nepic: \"   \"\n---\n\nbody\n";
        let parsed = parse_item_md_text_with_warnings(text, "s", "flat", Path::new("/tmp/x.md"));
        let err = parsed
            .fm_typed_error
            .as_deref()
            .expect("whitespace-only epic must fail typed parse");
        assert!(err.contains("epic"), "err={err}");
        assert!(err.contains("blank"), "err={err}");
    }

    #[test]
    fn deser_epic_rejects_float() {
        // The legacy-numeric escape hatch is for integer issue IDs
        // (`epic: 42`) so `doctor --fix` can read pre-slug repos. A
        // float-shaped epic is a YAML accident, not a legacy ref, and
        // `n.to_string()` would otherwise produce nonsense slugs like
        // `"3.14"` that fail downstream slug validation with a
        // confusing error.
        let text = "---\ntype: bug\nstatus: open\npriority: normal\nepic: 3.14\n---\n\nbody\n";
        let parsed = parse_item_md_text_with_warnings(text, "s", "flat", Path::new("/tmp/x.md"));
        let err = parsed
            .fm_typed_error
            .as_deref()
            .expect("float epic must fail typed parse");
        assert!(err.contains("epic"), "err={err}");
        assert!(err.contains("integer"), "err={err}");
    }

    #[test]
    fn deser_epic_rejects_sequence() {
        let text = "---\ntype: bug\nstatus: open\npriority: normal\nepic: [1, 2]\n---\n\nbody\n";
        let parsed = parse_item_md_text_with_warnings(text, "s", "flat", Path::new("/tmp/x.md"));
        let err = parsed
            .fm_typed_error
            .as_deref()
            .expect("sequence epic must fail typed parse");
        assert!(err.contains("epic"), "err={err}");
        assert!(err.contains("sequence"), "err={err}");
    }

    #[test]
    fn deser_epic_rejects_mapping() {
        let text = "---\ntype: bug\nstatus: open\npriority: normal\nepic: {a: 1}\n---\n\nbody\n";
        let parsed = parse_item_md_text_with_warnings(text, "s", "flat", Path::new("/tmp/x.md"));
        let err = parsed
            .fm_typed_error
            .as_deref()
            .expect("mapping epic must fail typed parse");
        assert!(err.contains("epic"), "err={err}");
        assert!(err.contains("mapping"), "err={err}");
    }

    #[test]
    fn deser_epic_rejects_bool() {
        let text = "---\ntype: bug\nstatus: open\npriority: normal\nepic: true\n---\n\nbody\n";
        let parsed = parse_item_md_text_with_warnings(text, "s", "flat", Path::new("/tmp/x.md"));
        let err = parsed
            .fm_typed_error
            .as_deref()
            .expect("bool epic must fail typed parse");
        assert!(err.contains("epic"), "err={err}");
        assert!(err.contains("bool"), "err={err}");
    }

    #[test]
    fn deser_epic_still_accepts_legacy_numeric() {
        // Numeric epic refs predate slugs; doctor --fix migrates them.
        // Strict-rejecting them here would brick affected repos before
        // the migration could run.
        let text = "---\ntype: bug\nstatus: open\npriority: normal\nepic: 42\n---\n\nbody\n";
        let parsed = parse_item_md_text_with_warnings(text, "s", "flat", Path::new("/tmp/x.md"));
        assert_eq!(parsed.fm_typed_error, None);
        assert_eq!(parsed.issue.epic.as_deref(), Some("42"));
    }
}
