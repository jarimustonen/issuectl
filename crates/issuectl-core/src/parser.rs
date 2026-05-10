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
    #[serde(flatten)]
    pub unknown: BTreeMap<String, serde_yaml::Value>,
}

fn deser_epic<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = Option::<serde_yaml::Value>::deserialize(d)?;
    Ok(v.and_then(|val| match val {
        serde_yaml::Value::String(s) => Some(s),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }))
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
    // unconditional stderr print — the doctor --fix pass migrates these
    // and the web UI flags them inline.
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

fn extract_title(body: Option<&str>) -> String {
    let body = match body {
        Some(b) => b,
        None => return String::new(),
    };
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return strip_legacy_title_number(rest).trim().to_string();
        }
    }
    String::new()
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
}
