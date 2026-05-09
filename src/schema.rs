//! Frontmatter schema — declares which fields exist, which are required,
//! and (optionally) constrains their values to an enum. Lives at
//! `issues/.schema.yaml`. Auto-written on first use; safe to edit and
//! commit.
//!
//! # Design notes (v1)
//!
//! - **Format: YAML.** Mirrors the YAML frontmatter that issues already
//!   use, supports comments, human-editable. TOML would also work but
//!   would force two formats on the repo; JSON has no comments.
//! - **Path: `issues/.schema.yaml`.** Co-located with what it describes.
//!   The leading dot keeps it unobtrusive in `ls`; existing `read_dir`
//!   callers all gate on `is_dir()` so a regular file in `issues/` is
//!   already filtered out of slug discovery.
//! - **Custom fields are additive.** A user-declared field beyond the
//!   built-in set is allowed; if marked `required: true`, `doctor` and
//!   the mutation paths flag issues that lack it. Unknown fields in an
//!   issue are not errors — agents can attach extra metadata freely.
//! - **Built-in `required` can be overridden.** The default schema marks
//!   `created`, `type`, `status`, `priority` required; an editor can
//!   relax any of them.
//! - **Enums in v1.** A field may declare `enum: [..]` of allowed string
//!   values; for list-shaped fields (`list: true`), the constraint
//!   applies element-wise. Not full JSON Schema — that's deferred.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_yaml::{Mapping, Value};

pub const SCHEMA_RELATIVE_PATH: &str = "issues/.schema.yaml";

pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub fields: BTreeMap<String, FieldSpec>,
    /// Required H2 body sections per issue type. Lives in
    /// `issues/.schema.yaml` because body shape is a structural
    /// declaration about what an issue *is*, sibling to the
    /// frontmatter-shape declarations in `fields`. Heading names are
    /// matched verbatim (case-sensitive) against `## <name>` lines
    /// outside fenced code blocks. Empty / absent ⇒ no body-section
    /// requirement for that type.
    #[serde(default)]
    pub body_sections: BTreeMap<String, Vec<String>>,
}

fn default_version() -> u32 {
    SUPPORTED_SCHEMA_VERSION
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSpec {
    #[serde(default)]
    pub required: bool,
    /// Allowed string values. For list fields, applies element-wise.
    #[serde(default, rename = "enum")]
    pub allowed: Option<Vec<String>>,
    /// Hint that the field is a list; affects how `enum` is interpreted
    /// and how "missing/empty" is decided.
    #[serde(default)]
    pub list: bool,
}

pub fn schema_path(root: &Path) -> PathBuf {
    root.join(SCHEMA_RELATIVE_PATH)
}

/// Default schema YAML (also serves as the bootstrap content). Public
/// so docs/tests can reference it.
pub const DEFAULT_SCHEMA_YAML: &str = r#"# issuectl frontmatter schema (v1) — auto-generated; safe to edit and commit.
#
# Each field declares whether it is required and (optionally) an `enum`
# of allowed values. Declare a custom field below to make `doctor` and
# the mutation commands enforce it.
#
# - `required: true`   → an issue missing this field fails validation.
# - `enum: [a, b, c]`  → only those values are accepted.
# - `list: true`       → field is a list; `enum` applies element-wise.
#
# Merge semantics: this file is layered on top of the built-in default
# schema. To override a built-in field, redeclare it here (your full
# `FieldSpec` replaces the built-in one). To add a custom field, just
# add an entry. Unknown fields in an issue are NOT errors, so agents
# can attach extra metadata freely.
#
# `created` is intentionally NOT required by default — repos that
# pre-date schema enforcement may have issues without it. Set it to
# `required: true` once you are sure all issues have it (run
# `issuectl doctor` first).
version: 1
fields:
  type:
    required: true
    enum: [bug, task, feature, improvement, chore, epic]
  status:
    required: true
    enum:
      - open
      - in-progress
      - testing
      - done
      - fixed
      - wontfix
      - duplicate
      - cannot-reproduce
      - obsolete
  priority:
    required: true
    enum: [normal, high]
  created:
    required: false
  updated:
    required: false
  reporter:
    required: false
  assignee:
    required: false
  owner:
    required: false
  epic:
    required: false
  related:
    required: false
    list: true
  blocked_by:
    required: false
    list: true
  labels:
    required: false
    list: true
    # enum: [infra, frontend, backend]   # uncomment to constrain labels
  closed:
    required: false
  slug:
    required: false
  # `commits` is intentionally not declared: it is a list of mapping
  # entries (`{hash, summary}`), which the v1 schema's scalar/list-of-
  # string model cannot describe. Unknown fields are allowed, so it
  # passes validation either way.
"#;

pub fn default_schema() -> Schema {
    serde_yaml::from_str(DEFAULT_SCHEMA_YAML).expect("built-in default schema parses")
}

/// Load the schema from `issues/.schema.yaml`, layered on top of the
/// built-in default. The user's `fields` entries replace built-in
/// entries with the same name (whole-`FieldSpec` replacement, not
/// property-level merge), so a user can relax `type.required` or
/// constrain `labels.enum` without losing the rest of the defaults.
/// Returns the default schema unchanged when the file is missing.
///
/// Returns `Arc<Schema>` so server mode can share a single parsed
/// snapshot across requests via `repo_config::RepoConfigCache`. CLI
/// callers pay only an `Arc` allocation — the schema itself is parsed
/// once per command. When a cache is active on the current thread, the
/// cached `Arc` is returned without re-parsing.
pub fn load(root: &Path) -> Result<Arc<Schema>> {
    if let Some(cache) = crate::repo_config::current() {
        // The cache is bound to the `AppState` root in server mode.
        // `mutate::*` callers pass that same root, so the cache's
        // bound root governs which file is read. `debug_assert`
        // surfaces a future bug where a caller passes a different
        // root while a cache is active.
        debug_assert_eq!(
            root,
            cache.root(),
            "schema::load called with root that disagrees with the active RepoConfigCache",
        );
        return cache.schema();
    }
    Ok(Arc::new(load_uncached(root)?))
}

/// Direct, unconditional parse of `issues/.schema.yaml`. Used by
/// `repo_config::RepoConfigCache` to populate cache entries — calling
/// `load` from inside the cache would re-enter the thread-local and
/// defeat the point. Also the fallback `load` uses when no cache is
/// active.
pub(crate) fn load_uncached(root: &Path) -> Result<Schema> {
    let path = schema_path(root);
    if !path.is_file() {
        return Ok(default_schema());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let user: Schema = serde_yaml::from_str(&text)
        .with_context(|| format!("cannot parse {}", path.display()))?;
    if user.version != SUPPORTED_SCHEMA_VERSION {
        anyhow::bail!(
            "{}: unsupported schema version {} (this build supports {})",
            path.display(),
            user.version,
            SUPPORTED_SCHEMA_VERSION
        );
    }
    let mut merged = default_schema();
    for (name, spec) in user.fields {
        merged.fields.insert(name, spec);
    }
    // Body-section declarations replace per-type — same merge semantics
    // as `fields`. A user redeclaring `bug:` overrides the default
    // section list for that type wholesale; types not mentioned by the
    // user keep the default (today: empty).
    for (issue_type, sections) in user.body_sections {
        merged.body_sections.insert(issue_type, sections);
    }
    validate_body_sections(&merged.body_sections).with_context(|| format!("{}", path.display()))?;
    validate_loadability(&merged).with_context(|| format!("{}", path.display()))?;
    Ok(merged)
}

/// Reject body-section declarations that would render to malformed
/// markdown (newlines/tabs/control chars in heading names) or that
/// repeat a heading inside the same type. Empty section names are
/// rejected too — `## ` with no text is not a usable section.
fn validate_body_sections(sections: &BTreeMap<String, Vec<String>>) -> Result<()> {
    for (issue_type, names) in sections {
        if issue_type.trim().is_empty() {
            anyhow::bail!("body_sections: type key cannot be empty");
        }
        let mut seen = std::collections::BTreeSet::new();
        for name in names {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                anyhow::bail!(
                    "body_sections.{issue_type}: section name cannot be empty",
                );
            }
            if name.chars().any(|c| c == '\n' || c == '\r' || c.is_control()) {
                anyhow::bail!(
                    "body_sections.{issue_type}: section name {name:?} contains a newline or control character",
                );
            }
            if !seen.insert(name.clone()) {
                anyhow::bail!(
                    "body_sections.{issue_type}: section {name:?} declared twice",
                );
            }
        }
    }
    Ok(())
}

/// Reject schema configurations that would make `issuectl new`
/// permanently fail. v1 cannot satisfy:
///
/// - `slug: required: true` — slug is the directory name, not
///   frontmatter, so the new-issue mapping never has a `slug` field.
/// - any custom field (i.e. anything not in `BUILTIN_LIST_FIELDS`)
///   declared as `required: true, list: true` — `--field key=value`
///   is scalar-only.
///
/// Returning the error from `load()` means the user gets a clear
/// message at the moment they edit `.schema.yaml`, rather than a
/// confusing "missing required field" or "expected sequence" later.
fn validate_loadability(schema: &Schema) -> Result<()> {
    const BUILTIN_LIST_FIELDS: &[&str] = &["labels", "related"];
    for (name, spec) in &schema.fields {
        if name == "slug" && spec.required {
            anyhow::bail!(
                "schema cannot require `slug` — slug is the directory name, not a frontmatter field"
            );
        }
        if spec.required && spec.list && !BUILTIN_LIST_FIELDS.contains(&name.as_str()) {
            anyhow::bail!(
                "schema cannot require list field {name:?} — v1 `--field` only supplies scalar values; use one of the built-in list flags (--label / --related) for {:?}",
                BUILTIN_LIST_FIELDS
            );
        }
    }
    Ok(())
}

/// Write the default schema if the file is missing. Atomic and
/// idempotent: stages bytes in a tempfile in the same directory then
/// `persist_noclobber`s into place, so an interrupted write leaves
/// nothing visible at the final path. Returns `true` if a fresh file
/// was written by this call (callers use that to clear stale
/// `schema_parse_error` state in the doctor report).
pub fn ensure_default_written(root: &Path) -> Result<bool> {
    use std::io::Write;
    let path = schema_path(root);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("schema path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;
    if path.exists() {
        return Ok(false);
    }
    let mut tmp = tempfile::Builder::new()
        .prefix(".issuectl-schema-")
        .tempfile_in(parent)
        .with_context(|| format!("cannot create temp schema in {}", parent.display()))?;
    tmp.as_file_mut()
        .write_all(DEFAULT_SCHEMA_YAML.as_bytes())
        .with_context(|| format!("cannot write temp schema for {}", path.display()))?;
    tmp.as_file()
        .sync_all()
        .with_context(|| format!("cannot fsync temp schema for {}", path.display()))?;
    match tmp.persist_noclobber(&path) {
        Ok(_) => Ok(true),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(anyhow::Error::from(e.error))
            .with_context(|| format!("cannot persist {}", path.display())),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationKind {
    MissingRequired {
        field: String,
    },
    InvalidEnum {
        field: String,
        value: String,
        allowed: Vec<String>,
    },
    /// Field value does not match the declared shape (scalar vs list,
    /// or a non-string element appeared in a string-typed field).
    WrongType {
        field: String,
        expected: &'static str,
        actual: &'static str,
    },
}

impl ViolationKind {
    pub fn message(&self) -> String {
        match self {
            ViolationKind::MissingRequired { field } => {
                format!("missing required field {field:?}")
            }
            ViolationKind::InvalidEnum {
                field,
                value,
                allowed,
            } => {
                format!(
                    "field {field:?} = {value:?} is not in allowed set [{}]",
                    allowed.join(", ")
                )
            }
            ViolationKind::WrongType {
                field,
                expected,
                actual,
            } => {
                format!("field {field:?} expected {expected}, got {actual}")
            }
        }
    }
}

fn yaml_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Sequence(_) => "sequence",
        Value::Mapping(_) => "mapping",
        Value::Tagged(_) => "tagged",
    }
}

/// Validate a frontmatter mapping against the schema. Returns an empty
/// vector when valid. Type-mismatches are surfaced as `WrongType`
/// violations rather than silently passing — a `type: 42` (yaml number)
/// against `enum: [bug, ...]` would otherwise sneak through because
/// `as_str()` is `None` for numbers.
pub fn validate(schema: &Schema, fm: &Mapping) -> Vec<ViolationKind> {
    let mut out = Vec::new();
    for (name, spec) in &schema.fields {
        let key = Value::String(name.clone());
        let value = fm.get(&key);
        let present = value.map(|v| !is_yaml_empty(v, spec.list)).unwrap_or(false);
        if spec.required && !present {
            out.push(ViolationKind::MissingRequired {
                field: name.clone(),
            });
            continue;
        }
        let Some(v) = value else {
            continue;
        };
        if matches!(v, Value::Null) {
            continue;
        }
        if spec.list {
            let Value::Sequence(seq) = v else {
                out.push(ViolationKind::WrongType {
                    field: name.clone(),
                    expected: "sequence",
                    actual: yaml_type(v),
                });
                continue;
            };
            for item in seq {
                let Value::String(s) = item else {
                    out.push(ViolationKind::WrongType {
                        field: name.clone(),
                        expected: "string-list element",
                        actual: yaml_type(item),
                    });
                    continue;
                };
                if let Some(allowed) = &spec.allowed {
                    if !allowed.iter().any(|a| a == s) {
                        out.push(ViolationKind::InvalidEnum {
                            field: name.clone(),
                            value: s.clone(),
                            allowed: allowed.clone(),
                        });
                    }
                }
            }
        } else {
            let Some(s) = v.as_str() else {
                out.push(ViolationKind::WrongType {
                    field: name.clone(),
                    expected: "string",
                    actual: yaml_type(v),
                });
                continue;
            };
            if let Some(allowed) = &spec.allowed {
                if !s.is_empty() && !allowed.iter().any(|a| a == s) {
                    out.push(ViolationKind::InvalidEnum {
                        field: name.clone(),
                        value: s.to_string(),
                        allowed: allowed.clone(),
                    });
                }
            }
        }
    }
    out
}

/// Project the `status` field's allowed-value enum into a set. Used
/// by `transitions::validate_status_refs` to catch typo'd status
/// names in `.issuectl/transitions.yaml`. Falls back to
/// `crate::all_statuses()` when the schema's `status` field has no
/// `enum:` constraint — same lenient default as elsewhere.
pub fn status_universe(schema: &Schema) -> std::collections::BTreeSet<String> {
    schema
        .fields
        .get("status")
        .and_then(|spec| spec.allowed.as_ref())
        .map(|allowed| allowed.iter().cloned().collect())
        .unwrap_or_else(|| crate::all_statuses().iter().map(|s| s.to_string()).collect())
}

/// Required H2 section names for an issue type. Empty slice when
/// the type isn't declared. Returned as a slice so callers can iterate
/// without cloning.
pub fn required_sections_for_type<'a>(schema: &'a Schema, issue_type: &str) -> &'a [String] {
    schema
        .body_sections
        .get(issue_type)
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}

/// Names of required body sections (in declaration order) that are
/// *missing* from `body`. Heading detection routes through
/// `body_sections::all_h2_sections` so it is fence-aware (a `## …`
/// line inside a fenced code block does not count). Comparison is
/// case-sensitive — same as the writer.
pub fn missing_body_sections(schema: &Schema, issue_type: &str, body: &str) -> Vec<String> {
    let required = required_sections_for_type(schema, issue_type);
    if required.is_empty() {
        return Vec::new();
    }
    let present = crate::body_sections::all_h2_sections(body);
    required
        .iter()
        .filter(|name| !present.contains_key(name.as_str()))
        .cloned()
        .collect()
}

/// Render `## <name>\n\n` stubs for each section in `names`. Empty
/// input ⇒ empty string. Used by `cmd_new` to seed the required
/// scaffolding into newly-created issues.
pub fn stub_for_sections(names: &[String]) -> String {
    let mut out = String::new();
    for n in names {
        out.push_str("## ");
        out.push_str(n);
        out.push_str("\n\n");
    }
    out
}

fn is_yaml_empty(v: &Value, expect_list: bool) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Sequence(s) if expect_list => s.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_schema_parses() {
        let s = default_schema();
        assert!(s.fields.contains_key("type"));
        assert!(s.fields["type"].required);
        let allowed = s.fields["type"].allowed.as_ref().unwrap();
        assert!(allowed.iter().any(|a| a == "bug"));
    }

    #[test]
    fn ensure_default_written_creates_file() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        ensure_default_written(tmp.path()).unwrap();
        assert!(tmp.path().join("issues/.schema.yaml").is_file());
    }

    #[test]
    fn ensure_default_written_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        ensure_default_written(tmp.path()).unwrap();
        let path = tmp.path().join("issues/.schema.yaml");
        let custom = "version: 1\nfields:\n  type:\n    required: false\n";
        fs::write(&path, custom).unwrap();
        ensure_default_written(tmp.path()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), custom);
    }

    #[test]
    fn load_falls_back_to_default_when_missing() {
        let tmp = TempDir::new().unwrap();
        let s = load(tmp.path()).unwrap();
        assert!(s.fields.contains_key("status"));
    }

    #[test]
    fn validate_flags_missing_required_field() {
        let schema = default_schema();
        let fm: Mapping = serde_yaml::from_str("status: open\npriority: normal\n").unwrap();
        let v = validate(&schema, &fm);
        // Missing `type` (required by default schema).
        assert!(matches!(
            v.iter().find(|x| matches!(x, ViolationKind::MissingRequired { field } if field == "type")),
            Some(_)
        ));
    }

    #[test]
    fn validate_passes_when_all_required_present() {
        let schema = default_schema();
        let fm: Mapping =
            serde_yaml::from_str("type: bug\nstatus: open\npriority: normal\n").unwrap();
        let v = validate(&schema, &fm);
        assert!(v.is_empty(), "expected no violations, got {v:?}");
    }

    #[test]
    fn validate_flags_invalid_enum_scalar() {
        let schema = default_schema();
        let fm: Mapping =
            serde_yaml::from_str("type: not-a-type\nstatus: open\npriority: normal\n").unwrap();
        let v = validate(&schema, &fm);
        assert!(v
            .iter()
            .any(|x| matches!(x, ViolationKind::InvalidEnum { field, .. } if field == "type")));
    }

    #[test]
    fn validate_flags_invalid_enum_list_element() {
        let yaml = "version: 1\nfields:\n  labels:\n    list: true\n    enum: [infra, frontend]\n";
        let schema: Schema = serde_yaml::from_str(yaml).unwrap();
        let fm: Mapping = serde_yaml::from_str("labels:\n- infra\n- bogus\n").unwrap();
        let v = validate(&schema, &fm);
        assert_eq!(v.len(), 1);
        match &v[0] {
            ViolationKind::InvalidEnum { field, value, .. } => {
                assert_eq!(field, "labels");
                assert_eq!(value, "bogus");
            }
            other => panic!("expected InvalidEnum, got {other:?}"),
        }
    }

    #[test]
    fn validate_allows_unknown_fields() {
        let schema = default_schema();
        let fm: Mapping = serde_yaml::from_str(
            "type: bug\nstatus: open\npriority: normal\nteam: payments\n",
        )
        .unwrap();
        let v = validate(&schema, &fm);
        assert!(v.is_empty());
    }

    #[test]
    fn validate_supports_custom_required_field() {
        let yaml = "version: 1\nfields:\n  team:\n    required: true\n";
        let schema: Schema = serde_yaml::from_str(yaml).unwrap();
        let fm: Mapping = serde_yaml::from_str("type: bug\n").unwrap();
        let v = validate(&schema, &fm);
        assert_eq!(v.len(), 1);
        match &v[0] {
            ViolationKind::MissingRequired { field } => assert_eq!(field, "team"),
            other => panic!("expected MissingRequired, got {other:?}"),
        }
    }

    #[test]
    fn empty_string_counts_as_missing_for_required() {
        let schema = default_schema();
        let fm: Mapping =
            serde_yaml::from_str("type: \"\"\nstatus: open\npriority: normal\n").unwrap();
        let v = validate(&schema, &fm);
        assert!(v
            .iter()
            .any(|x| matches!(x, ViolationKind::MissingRequired { field } if field == "type")));
    }

    #[test]
    fn validate_rejects_non_string_scalar_for_enum_field() {
        let schema = default_schema();
        let fm: Mapping = serde_yaml::from_str("type: 42\nstatus: open\npriority: normal\n").unwrap();
        let v = validate(&schema, &fm);
        assert!(
            v.iter().any(|x| matches!(x, ViolationKind::WrongType { field, .. } if field == "type")),
            "expected WrongType for type: 42, got {v:?}"
        );
    }

    #[test]
    fn validate_rejects_scalar_for_list_field() {
        let yaml = "version: 1\nfields:\n  labels:\n    list: true\n";
        let schema: Schema = serde_yaml::from_str(yaml).unwrap();
        let fm: Mapping = serde_yaml::from_str("labels: not-a-list\n").unwrap();
        let v = validate(&schema, &fm);
        assert!(
            v.iter().any(|x| matches!(x, ViolationKind::WrongType { field, .. } if field == "labels")),
            "expected WrongType for scalar in list field, got {v:?}"
        );
    }

    #[test]
    fn validate_rejects_non_string_list_element() {
        let yaml = "version: 1\nfields:\n  labels:\n    list: true\n    enum: [infra]\n";
        let schema: Schema = serde_yaml::from_str(yaml).unwrap();
        let fm: Mapping = serde_yaml::from_str("labels:\n- infra\n- 7\n").unwrap();
        let v = validate(&schema, &fm);
        assert!(v
            .iter()
            .any(|x| matches!(x, ViolationKind::WrongType { field, .. } if field == "labels")));
    }

    #[test]
    fn load_merges_user_schema_with_defaults() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        // User adds a custom required field; built-in `type` enum must
        // still be enforced.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let schema = load(tmp.path()).unwrap();
        let fm: Mapping = serde_yaml::from_str(
            "type: nonsense\nstatus: open\npriority: normal\nteam: infra\n",
        )
        .unwrap();
        let v = validate(&schema, &fm);
        assert!(
            v.iter()
                .any(|x| matches!(x, ViolationKind::InvalidEnum { field, .. } if field == "type")),
            "built-in `type` enum must survive merge with custom user fields, got {v:?}"
        );
    }

    #[test]
    fn load_user_can_relax_builtin_required_via_merge() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: false\n",
        )
        .unwrap();
        let schema = load(tmp.path()).unwrap();
        // No `type` declared, but type was relaxed; status/priority
        // still required.
        let fm: Mapping = serde_yaml::from_str("status: open\npriority: normal\n").unwrap();
        assert!(validate(&schema, &fm).is_empty());
    }

    #[test]
    fn load_rejects_unsupported_version() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 999\nfields: {}\n",
        )
        .unwrap();
        let err = load(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("unsupported schema version"));
    }

    #[test]
    fn load_rejects_required_slug_field() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  slug:\n    required: true\n",
        )
        .unwrap();
        let err = load(tmp.path()).unwrap_err();
        let chain = format!("{err:#}");
        assert!(
            chain.contains("slug"),
            "expected slug-rejection message, got {chain}"
        );
    }

    #[test]
    fn load_rejects_required_custom_list_field() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  components:\n    required: true\n    list: true\n",
        )
        .unwrap();
        let err = load(tmp.path()).unwrap_err();
        let chain = format!("{err:#}");
        assert!(
            chain.contains("list"),
            "expected list-rejection message, got {chain}"
        );
    }

    #[test]
    fn load_allows_built_in_list_field_required() {
        // `labels` is a built-in list field that --label can populate,
        // so requiring it must be allowed.
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  labels:\n    required: true\n    list: true\n",
        )
        .unwrap();
        load(tmp.path()).unwrap();
    }

    #[test]
    fn merge_replaces_built_in_fieldspec_whole() {
        // Whole-FieldSpec replacement: relaxing `type.required` also
        // drops `type.enum` because the user's spec is the new source
        // of truth for that field. Documented in the design note; lock
        // the semantics in a test so a future "merge by property"
        // refactor can't ship silently.
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: false\n",
        )
        .unwrap();
        let schema = load(tmp.path()).unwrap();
        let type_spec = schema.fields.get("type").expect("type still present");
        assert!(
            type_spec.allowed.is_none(),
            "redeclaring `type` without `enum:` must drop the built-in enum (whole-spec replace)"
        );
    }

    #[test]
    fn load_rejects_unknown_fieldspec_keys() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    requried: true\n",
        )
        .unwrap();
        assert!(load(tmp.path()).is_err(), "typo'd `requried` must be rejected");
    }
}
