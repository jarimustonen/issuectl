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
    /// Lifecycle classification for status values. An entry maps a
    /// status to `Active` or `Closing`; closing statuses get the
    /// directory bucketing, `closed:` stamp, and doctor consistency
    /// treatment that built-ins receive.
    ///
    /// Lookup order is **schema-first** — a user entry takes
    /// precedence over the built-in fallback. So a project that
    /// writes `status_classes: { done: active }` reclassifies the
    /// built-in `done` as active everywhere lifecycle decisions are
    /// made. This is intentional: it lets a workflow re-use built-in
    /// names with project-specific semantics. Built-in classification
    /// stays the default whenever this map is silent. A status in
    /// neither this map nor the built-in fallback defaults to
    /// `Active` (lenient — chosen so a stray typo doesn't get
    /// auto-stamped with `closed:`).
    #[serde(default)]
    pub status_classes: BTreeMap<String, StatusClass>,
    /// Legacy → canonical value remaps for `status`, consumed by
    /// `doctor --fix` to coerce pre-0.5.1 values during migration
    /// (e.g. `closed` → `done`). Built-in defaults live in
    /// `DEFAULT_SCHEMA_YAML`; a repo entry with the same key replaces
    /// the built-in. Deliberately *not* applied by the normal mutation
    /// commands — those still reject out-of-enum values so a typo can't
    /// slip through silently; only `doctor --fix` rewrites via this map.
    #[serde(default)]
    pub status_aliases: BTreeMap<String, String>,
    /// Legacy → canonical value remaps for `type`. Same semantics as
    /// [`status_aliases`](Self::status_aliases).
    #[serde(default)]
    pub type_aliases: BTreeMap<String, String>,
    /// Definition-of-Done gate configuration. When `strict` is true, a
    /// transition into a closing status with unchecked items in
    /// `## Acceptance Criteria` is rejected. When false (default),
    /// the same condition surfaces as a warning. Heading and parser
    /// live in [`crate::body`]; only the gate's *severity* is
    /// configurable. Zero frontmatter changes — the gate reads the
    /// body, not custom YAML fields.
    #[serde(default)]
    pub dod: DodConfig,
}

/// Severity knob for the Definition-of-Done gate. Default = warn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DodConfig {
    /// When true, the gate blocks the write; otherwise it surfaces as
    /// a non-fatal warning.
    #[serde(default)]
    pub strict: bool,
}

/// Lifecycle classification for a status value.
///
/// Two kinds today: `Active` (issue is still open / in-progress) and
/// `Closing` (issue is finished — `done`, `wontfix`, `archived`, etc.).
/// All `Closing` variants share lifecycle behaviour; the task brief
/// explicitly defers a multi-flavoured taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusClass {
    Active,
    Closing,
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
    /// Conditional requirement. The field may be optional by default
    /// (`required: false`) yet become required when a condition on the
    /// issue holds. v1 supports a single condition shape — the issue's
    /// `status` resolving to a given lifecycle class — which expresses
    /// the lifecycle rule "a closing status implies `closed:` is set"
    /// declaratively rather than leaving it as implicit doctor logic.
    #[serde(default)]
    pub required_when: Option<RequiredWhen>,
}

/// A conditional-requirement predicate for a [`FieldSpec`]. v1 only
/// supports gating on the issue's status lifecycle class; the struct
/// shape leaves room to add further conditions without a format break.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredWhen {
    /// The owning field is required when the issue's `status` resolves
    /// (via [`status_class`]) to this lifecycle class.
    #[serde(default)]
    pub status_class: Option<StatusClass>,
}

impl RequiredWhen {
    /// True when this condition is satisfied for an issue carrying
    /// `status`. Resolves `status` through the schema's lifecycle
    /// layering so a project override (`status_classes:`) is honoured.
    pub fn matches(&self, schema: &Schema, status: &str) -> bool {
        match self.status_class {
            Some(class) => status_class(schema, status) == class,
            None => false,
        }
    }
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
    enum: [low, normal, high]
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
  reviewer:
    required: false
  review_status:
    required: false
    enum: [requested, in-review, approved, changes-requested]
  # Lightweight estimates. Declare either `size` OR `estimate` on an
  # issue, not both — the schema can't express the mutual-exclusion
  # rule directly, but `issuectl workload` / `burndown` will flag any
  # issue carrying both. `size` is a fixed four-value enum; `estimate`
  # is a free-form numeric (story points) and is intentionally NOT
  # declared here because the v1 schema validator is string-typed and
  # would reject `estimate: 5` (a YAML number). Unknown fields pass
  # validation, so the numeric form rides through. See
  # `crates/issuectl-core/src/estimate.rs`.
  size:
    required: false
    enum: [S, M, L, XL]
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
    # `closed:` is conditionally required: any issue whose status is
    # classified `closing` (see `status_classes` below) must carry a
    # `closed:` date. Declared here so schema validation and `doctor`
    # enforce the same lifecycle rule instead of leaving it implicit.
    required_when:
      status_class: closing
  slug:
    required: false
  # `commits` is intentionally not declared: it is a list of mapping
  # entries (`{hash, summary}`), which the v1 schema's scalar/list-of-
  # string model cannot describe. Unknown fields are allowed, so it
  # passes validation either way.

# Lifecycle classification for status values. Built-in statuses
# (`open`, `in-progress`, `testing` → active; `done`, `fixed`, `wontfix`,
# `duplicate`, `cannot-reproduce`, `obsolete` → closing) are classified
# automatically. Add a status's class here to extend the taxonomy with
# a custom status (e.g. `archived`), or to *override* a built-in
# (e.g. `done: active` if your workflow treats `done` as in-progress).
# Closing statuses get the `closed:` stamp, end up in the closed
# bucket, and pass doctor's open/closed consistency check the same way
# `done` does.
#
# status_classes:
#   archived: closing
#   verified: active

# Legacy-value aliases for `doctor --fix` migration. When an issue's
# `status` or `type` equals a key below, `doctor --fix` rewrites it to
# the mapped canonical value (and, for a status that becomes a closing
# status, stamps a `closed:` date if one is missing). Built-in defaults
# cover the most common pre-0.5.1 values surfaced in migration feedback;
# add your own to extend, or restate a key to override its target. Only
# `doctor --fix` consumes this map — the regular mutation commands still
# reject out-of-enum values, so an unaliased typo never slips through.
#
# Only unambiguous synonyms are built in. Work-pause states such as
# `paused` / `blocked` are deliberately NOT mapped: the canonical status
# set has no equivalent, so coercing them to `in-progress` would
# misrepresent intent and lose information. A repo that wants them
# handled should add its own mapping (or a custom status via
# `status_classes`) below.
status_aliases:
  closed: done
  resolved: fixed
  in_progress: in-progress
type_aliases:
  enhancement: improvement
  refactor: chore
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
/// Always re-parses. Callers that want cross-request caching
/// (server mode) construct a `repo_config::RepoConfigCache` and
/// call its `schema()` method directly via the `ConfigSource`
/// trait. Returns `Arc<Schema>` so the result is interchangeable
/// with the cache's snapshot for downstream consumers.
pub fn load(root: &Path) -> Result<Arc<Schema>> {
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
    let user: Schema =
        serde_yaml::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))?;
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
    for (status, class) in user.status_classes {
        merged.status_classes.insert(status, class);
    }
    // Alias maps merge per-key over the built-in defaults, so a repo
    // can add a project-specific legacy value (or override a built-in
    // target) without restating the whole table.
    for (from, to) in user.status_aliases {
        merged.status_aliases.insert(from, to);
    }
    for (from, to) in user.type_aliases {
        merged.type_aliases.insert(from, to);
    }
    merged.dod = user.dod;
    // Drop inherited built-in aliases whose target fell outside a
    // user-narrowed field enum. The alias merge has no removal semantics,
    // so a project that narrows `status`/`type` cannot delete a now-stale
    // built-in alias; left live, `doctor --fix` would coerce a legacy
    // value to a target that immediately fails enum validation. Pruning
    // makes the stale built-in inert instead. User-authored aliases are
    // never pruned — an out-of-enum target there is a mistake, surfaced as
    // a hard error by `validate_loadability`.
    let builtin = default_schema();
    let status_allowed = merged.fields.get("status").and_then(|s| s.allowed.clone());
    prune_stale_builtin_aliases(
        &mut merged.status_aliases,
        status_allowed.as_deref(),
        &builtin.status_aliases,
    );
    let type_allowed = merged.fields.get("type").and_then(|s| s.allowed.clone());
    prune_stale_builtin_aliases(
        &mut merged.type_aliases,
        type_allowed.as_deref(),
        &builtin.type_aliases,
    );
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
                anyhow::bail!("body_sections.{issue_type}: section name cannot be empty",);
            }
            if name
                .chars()
                .any(|c| c == '\n' || c == '\r' || c.is_control())
            {
                anyhow::bail!(
                    "body_sections.{issue_type}: section name {name:?} contains a newline or control character",
                );
            }
            if !seen.insert(name.clone()) {
                anyhow::bail!("body_sections.{issue_type}: section {name:?} declared twice",);
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
    validate_alias_map(
        "status",
        schema
            .fields
            .get("status")
            .and_then(|s| s.allowed.as_deref()),
        &schema.status_aliases,
        "status_aliases",
    )?;
    validate_alias_map(
        "type",
        schema.fields.get("type").and_then(|s| s.allowed.as_deref()),
        &schema.type_aliases,
        "type_aliases",
    )?;
    Ok(())
}

/// Remove inherited built-in aliases whose target is no longer a member of
/// a (user-narrowed) field enum. An entry is removed only when it matches
/// the built-in default unchanged (`builtin[from] == to`) *and* its target
/// fell outside `allowed` — user-authored entries are left in place so
/// [`validate_alias_map`] can reject their out-of-enum targets. A field
/// with no enum (`allowed == None`) constrains nothing, so nothing is
/// pruned.
fn prune_stale_builtin_aliases(
    aliases: &mut BTreeMap<String, String>,
    allowed: Option<&[String]>,
    builtin: &BTreeMap<String, String>,
) {
    let Some(allowed) = allowed else {
        return;
    };
    aliases.retain(|from, to| {
        let inherited = builtin.get(from).map(|b| b == to).unwrap_or(false);
        let target_in_enum = allowed.iter().any(|a| a == to);
        !inherited || target_in_enum
    });
}

/// Reject alias tables that can never produce a valid coercion. An alias
/// `from → to` is consumed by `doctor --fix` (via [`would_coerce`]) to
/// rewrite a legacy value to `to`. Two shapes are nonsensical:
///
/// - **Alias chain.** `to` must not itself be an alias key. `would_coerce`
///   resolves a single hop, not transitively, so a chain `a → b → c` would
///   leave `a` coerced to `b` — still an alias, never reaching `c`. Always
///   rejected so the author points `a` directly at the canonical value.
///   (This also rejects a degenerate self-alias `a → a`.)
/// - **Target outside the field enum.** When `field` declares an `enum`,
///   `to` must be a member of it; otherwise `doctor --fix` would rewrite a
///   legacy value to something that immediately fails enum validation —
///   trading one violation for another. A field with no `enum` accepts any
///   value, so there is nothing to check.
///
/// Stale inherited built-in aliases are pruned by
/// [`prune_stale_builtin_aliases`] before this runs, so the enum-membership
/// check here only ever fires on a user-authored (or user-overridden)
/// target — exactly the mistake worth a hard error.
fn validate_alias_map(
    field: &str,
    allowed: Option<&[String]>,
    aliases: &BTreeMap<String, String>,
    map_name: &str,
) -> Result<()> {
    for (from, to) in aliases {
        if let Some(next) = aliases.get(to) {
            anyhow::bail!(
                "{map_name}: alias {from:?} → {to:?} cannot be used — {to:?} is itself an alias key \
                 (it maps to {next:?}); alias chains are not resolved, so map {from:?} directly to a canonical value"
            );
        }
        if let Some(allowed) = allowed {
            if !allowed.iter().any(|a| a == to) {
                anyhow::bail!(
                    "{map_name}: alias {from:?} → {to:?} targets {to:?}, which is not in the `{field}` enum [{}]; \
                     coercing to it would immediately fail validation",
                    allowed.join(", ")
                );
            }
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
    write_default(root, false)
}

/// Atomically write the default schema. With `force=false`, refuses to
/// clobber an existing schema (returns `Ok(false)`). With `force=true`,
/// replaces an existing schema in place via tempfile-rename, which is
/// what `issuectl init --force` uses to reset a corrupted scaffold.
/// Returns whether bytes were actually written.
pub fn write_default(root: &Path, force: bool) -> Result<bool> {
    use std::io::Write;
    let path = schema_path(root);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("schema path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
    if path.exists() && !force {
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
    if force {
        tmp.persist(&path)
            .map_err(|e| e.error)
            .with_context(|| format!("cannot persist {}", path.display()))?;
        Ok(true)
    } else {
        match tmp.persist_noclobber(&path) {
            Ok(_) => Ok(true),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(anyhow::Error::from(e.error))
                .with_context(|| format!("cannot persist {}", path.display())),
        }
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
    /// A field declared `required_when` was absent while its condition
    /// held — e.g. `closed:` missing on an issue with a closing status.
    RequiredWhen {
        field: String,
        status: String,
        status_class: StatusClass,
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
            ViolationKind::RequiredWhen {
                field,
                status,
                status_class,
            } => {
                let class = match status_class {
                    StatusClass::Active => "active",
                    StatusClass::Closing => "closing",
                };
                format!("field {field:?} is required when status {status:?} is {class}")
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
    // Conditional requirements. Evaluated in a second pass so a field
    // that is statically optional (`required: false`) but
    // `required_when` a status condition holds is flagged. A field
    // already flagged `MissingRequired` (static `required: true`) is
    // not double-reported.
    let status = fm
        .get(Value::String("status".into()))
        .and_then(|v| v.as_str());
    if let Some(status) = status {
        for (name, spec) in &schema.fields {
            let Some(rw) = &spec.required_when else {
                continue;
            };
            if spec.required {
                continue;
            }
            let present = fm
                .get(Value::String(name.clone()))
                .map(|v| !is_yaml_empty(v, spec.list))
                .unwrap_or(false);
            if present {
                continue;
            }
            if rw.matches(schema, status) {
                out.push(ViolationKind::RequiredWhen {
                    field: name.clone(),
                    status: status.to_string(),
                    status_class: status_class(schema, status),
                });
            }
        }
    }
    out
}

/// True when `field` is required for an issue whose `status` is the
/// given value, per the field's `required_when` declaration. Lets
/// callers (notably `doctor`) drive a consistency check off the schema
/// declaration rather than a hardcoded rule, so relaxing or removing
/// `required_when` in `.schema.yaml` also relaxes the check.
pub fn field_required_for_status(schema: &Schema, field: &str, status: &str) -> bool {
    schema
        .fields
        .get(field)
        .and_then(|spec| spec.required_when.as_ref())
        .map(|rw| rw.matches(schema, status))
        .unwrap_or(false)
}

/// Resolve a legacy `status` value to its canonical replacement via
/// the schema's `status_aliases`. `None` when the value is not an
/// alias key (already canonical, or simply unknown).
pub fn status_alias_target<'a>(schema: &'a Schema, value: &str) -> Option<&'a str> {
    schema.status_aliases.get(value).map(String::as_str)
}

/// Resolve a legacy `type` value to its canonical replacement via the
/// schema's `type_aliases`. See [`status_alias_target`].
pub fn type_alias_target<'a>(schema: &'a Schema, value: &str) -> Option<&'a str> {
    schema.type_aliases.get(value).map(String::as_str)
}

/// The canonical value `doctor --fix` would coerce `value` to for
/// `field`, or `None` when no coercion applies. Single source of truth
/// for coercion eligibility, shared by the doctor scan (which records
/// the pending rewrite) and the apply pass. Only `status` and `type`
/// carry alias tables today. A coercion applies only when the field
/// declares an `enum`, the value is NOT already in it, and an alias
/// maps it — so a value that is canonical for this repo (even one that
/// collides with a built-in alias key) is never silently rewritten, and
/// a field with no `enum` constraint has nothing to migrate toward.
pub fn would_coerce<'a>(schema: &'a Schema, field: &str, value: &str) -> Option<&'a str> {
    let allowed = schema.fields.get(field)?.allowed.as_ref()?;
    if allowed.iter().any(|a| a == value) {
        return None;
    }
    match field {
        "status" => status_alias_target(schema, value),
        "type" => type_alias_target(schema, value),
        _ => None,
    }
}

/// Lifecycle classification for a status value.
///
/// Lookup order:
/// 1. The schema's `status_classes:` map — wins when present, so the
///    user can override built-in classifications (`done: active` is a
///    valid project policy).
/// 2. Built-in fallback (`issue_fields::is_closing_status`) — covers
///    `done`/`fixed`/`wontfix`/etc. when the schema is silent.
/// 3. Default to `Active` for anything unknown to both. This is the
///    lenient choice: a stray status (typo, removed enum value)
///    doesn't get auto-stamped with `closed:` or banished to the
///    closed bucket.
pub fn status_class(schema: &Schema, status: &str) -> StatusClass {
    if let Some(class) = schema.status_classes.get(status) {
        return *class;
    }
    if crate::issue_fields::is_closing_status(status) {
        StatusClass::Closing
    } else {
        StatusClass::Active
    }
}

/// Convenience wrapper: `true` when `status` should receive the
/// closing-side lifecycle treatment (directory bucket, `closed:`
/// stamp, doctor consistency).
pub fn is_closing(schema: &Schema, status: &str) -> bool {
    status_class(schema, status) == StatusClass::Closing
}

/// Project the `status` field's allowed-value enum into a set. Used
/// by `transitions::validate_status_refs` to catch typo'd status
/// names in `.issuectl/transitions.yaml`. Falls back to
/// `crate::issue_fields::all_statuses()` when the schema's `status` field has no
/// `enum:` constraint — same lenient default as elsewhere.
pub fn status_universe(schema: &Schema) -> std::collections::BTreeSet<String> {
    schema
        .fields
        .get("status")
        .and_then(|spec| spec.allowed.as_ref())
        .map(|allowed| allowed.iter().cloned().collect())
        .unwrap_or_else(|| {
            crate::issue_fields::all_statuses()
                .iter()
                .map(|s| s.to_string())
                .collect()
        })
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
            v.iter()
                .find(|x| matches!(x, ViolationKind::MissingRequired { field } if field == "type")),
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
        let fm: Mapping =
            serde_yaml::from_str("type: bug\nstatus: open\npriority: normal\nteam: payments\n")
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
        let fm: Mapping =
            serde_yaml::from_str("type: 42\nstatus: open\npriority: normal\n").unwrap();
        let v = validate(&schema, &fm);
        assert!(
            v.iter()
                .any(|x| matches!(x, ViolationKind::WrongType { field, .. } if field == "type")),
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
            v.iter()
                .any(|x| matches!(x, ViolationKind::WrongType { field, .. } if field == "labels")),
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
        let fm: Mapping =
            serde_yaml::from_str("type: nonsense\nstatus: open\npriority: normal\nteam: infra\n")
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
    fn status_class_built_in_closing_is_closing() {
        // Regression: built-in `done` / `wontfix` keep classifying as
        // closing even when the schema declares no `status_classes:`.
        let s = default_schema();
        assert_eq!(status_class(&s, "done"), StatusClass::Closing);
        assert_eq!(status_class(&s, "wontfix"), StatusClass::Closing);
        assert_eq!(status_class(&s, "open"), StatusClass::Active);
        assert_eq!(status_class(&s, "in-progress"), StatusClass::Active);
    }

    #[test]
    fn status_class_custom_closing_status_routes_through_schema() {
        // A project that declares `archived` as closing in
        // `status_classes:` gets the closing-side classification.
        let yaml = "version: 1\nstatus_classes:\n  archived: closing\n  verified: active\n";
        let schema: Schema = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(status_class(&schema, "archived"), StatusClass::Closing);
        assert!(is_closing(&schema, "archived"));
        assert_eq!(status_class(&schema, "verified"), StatusClass::Active);
        assert!(!is_closing(&schema, "verified"));
        // Built-ins still classify correctly through the same schema.
        assert!(is_closing(&schema, "done"));
        assert!(!is_closing(&schema, "open"));
    }

    #[test]
    fn status_class_unknown_status_defaults_to_active() {
        // Schema present but doesn't declare the status's class, AND
        // it isn't in the built-in fallback. Lenient default: active —
        // chosen so a stray status doesn't get auto-stamped with
        // `closed:` or banished to the closed directory bucket.
        let s = default_schema();
        assert_eq!(status_class(&s, "ufo"), StatusClass::Active);
    }

    #[test]
    fn load_merges_user_status_classes_with_defaults() {
        // The user can extend the lifecycle taxonomy with custom
        // statuses without restating the built-in `status` enum or
        // re-declaring the built-in classifications.
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nstatus_classes:\n  archived: closing\n",
        )
        .unwrap();
        let schema = load(tmp.path()).unwrap();
        assert!(is_closing(&schema, "archived"));
        // Built-in `done` still classifies through the fallback.
        assert!(is_closing(&schema, "done"));
        // Built-in `open` stays active.
        assert!(!is_closing(&schema, "open"));
    }

    #[test]
    fn validate_flags_required_when_closing_status_missing_closed() {
        // Built-in default declares `closed: required_when status_class
        // closing`. A `done` issue without `closed:` must be flagged.
        let schema = default_schema();
        let fm: Mapping =
            serde_yaml::from_str("type: bug\nstatus: done\npriority: normal\n").unwrap();
        let v = validate(&schema, &fm);
        assert!(
            v.iter().any(
                |x| matches!(x, ViolationKind::RequiredWhen { field, .. } if field == "closed")
            ),
            "expected RequiredWhen for closed on a closing status, got {v:?}"
        );
    }

    #[test]
    fn validate_required_when_satisfied_by_present_closed() {
        let schema = default_schema();
        let fm: Mapping =
            serde_yaml::from_str("type: bug\nstatus: done\npriority: normal\nclosed: 2026-05-06\n")
                .unwrap();
        let v = validate(&schema, &fm);
        assert!(
            !v.iter()
                .any(|x| matches!(x, ViolationKind::RequiredWhen { .. })),
            "closed present must satisfy required_when, got {v:?}"
        );
    }

    #[test]
    fn validate_required_when_inactive_for_active_status() {
        let schema = default_schema();
        let fm: Mapping =
            serde_yaml::from_str("type: bug\nstatus: open\npriority: normal\n").unwrap();
        let v = validate(&schema, &fm);
        assert!(
            !v.iter()
                .any(|x| matches!(x, ViolationKind::RequiredWhen { .. })),
            "active status must not trigger closed required_when, got {v:?}"
        );
    }

    #[test]
    fn required_when_honours_schema_status_class_override() {
        // A repo that reclassifies built-in `done` as active must NOT
        // require `closed:` for a `done` issue — required_when resolves
        // the class through the same layered lookup.
        let yaml = "version: 1\nstatus_classes:\n  done: active\n";
        let schema: Schema = serde_yaml::from_str(yaml).unwrap();
        // The user schema above has no `fields`, so merge with defaults.
        let mut merged = default_schema();
        merged
            .status_classes
            .insert("done".into(), StatusClass::Active);
        let fm: Mapping =
            serde_yaml::from_str("type: bug\nstatus: done\npriority: normal\n").unwrap();
        assert!(
            !validate(&merged, &fm)
                .iter()
                .any(|x| matches!(x, ViolationKind::RequiredWhen { .. })),
            "done reclassified active must not require closed:"
        );
        // Sanity: the standalone parse keeps the override too.
        assert_eq!(status_class(&schema, "done"), StatusClass::Active);
    }

    #[test]
    fn field_required_for_status_tracks_schema_declaration() {
        let schema = default_schema();
        assert!(field_required_for_status(&schema, "closed", "done"));
        assert!(!field_required_for_status(&schema, "closed", "open"));
        // A field with no required_when is never conditionally required.
        assert!(!field_required_for_status(&schema, "type", "done"));
    }

    #[test]
    fn default_schema_carries_built_in_aliases() {
        let s = default_schema();
        assert_eq!(status_alias_target(&s, "closed"), Some("done"));
        assert_eq!(status_alias_target(&s, "resolved"), Some("fixed"));
        assert_eq!(status_alias_target(&s, "in_progress"), Some("in-progress"));
        assert_eq!(type_alias_target(&s, "enhancement"), Some("improvement"));
        assert_eq!(type_alias_target(&s, "refactor"), Some("chore"));
        // A canonical value is not an alias key.
        assert_eq!(status_alias_target(&s, "done"), None);
    }

    #[test]
    fn load_merges_user_aliases_over_built_ins() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nstatus_aliases:\n  closed: fixed\n  wip: in-progress\n",
        )
        .unwrap();
        let schema = load(tmp.path()).unwrap();
        // User override replaces the built-in target.
        assert_eq!(status_alias_target(&schema, "closed"), Some("fixed"));
        // User-added key is present.
        assert_eq!(status_alias_target(&schema, "wip"), Some("in-progress"));
        // Untouched built-in survives the merge.
        assert_eq!(status_alias_target(&schema, "resolved"), Some("fixed"));
        // type_aliases built-ins survive when only status_aliases edited.
        assert_eq!(type_alias_target(&schema, "refactor"), Some("chore"));
    }

    #[test]
    fn default_schema_aliases_pass_validation() {
        // The built-in alias tables must satisfy the loadability checks:
        // every target is in its field enum and none is itself a key.
        validate_loadability(&default_schema()).unwrap();
    }

    #[test]
    fn load_rejects_status_alias_target_outside_enum() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        // `nope` is not a member of the built-in `status` enum.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nstatus_aliases:\n  legacy: nope\n",
        )
        .unwrap();
        let err = load(tmp.path()).unwrap_err();
        let chain = format!("{err:#}");
        assert!(
            chain.contains("status_aliases") && chain.contains("not in the `status` enum"),
            "expected enum-membership rejection, got {chain}"
        );
    }

    #[test]
    fn load_rejects_type_alias_target_outside_enum() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\ntype_aliases:\n  legacy: nonsense\n",
        )
        .unwrap();
        let err = load(tmp.path()).unwrap_err();
        let chain = format!("{err:#}");
        assert!(
            chain.contains("type_aliases") && chain.contains("not in the `type` enum"),
            "expected enum-membership rejection, got {chain}"
        );
    }

    #[test]
    fn load_rejects_status_alias_chain() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        // `a → b` where `b` is itself an alias key (`b → done`).
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nstatus_aliases:\n  a: b\n  b: done\n",
        )
        .unwrap();
        let err = load(tmp.path()).unwrap_err();
        let chain = format!("{err:#}");
        assert!(
            chain.contains("status_aliases") && chain.contains("alias chains are not resolved"),
            "expected alias-chain rejection, got {chain}"
        );
    }

    #[test]
    fn load_rejects_self_alias() {
        // A degenerate self-alias `a → a` is a one-element chain and must
        // be rejected by the same check.
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nstatus_aliases:\n  done: done\n",
        )
        .unwrap();
        let err = load(tmp.path()).unwrap_err();
        let chain = format!("{err:#}");
        assert!(
            chain.contains("alias chains are not resolved"),
            "expected self-alias rejection, got {chain}"
        );
    }

    #[test]
    fn alias_validation_skips_enum_check_when_field_has_no_enum() {
        // If `status` is redeclared without an `enum`, any target is
        // acceptable (nothing to fail validation against) — only the
        // chain rule still applies.
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\nstatus_aliases:\n  legacy: anything-goes\n",
        )
        .unwrap();
        load(tmp.path()).unwrap();
    }

    #[test]
    fn narrowing_enum_prunes_stale_builtin_alias_instead_of_bricking() {
        // Narrowing `status` past a built-in alias target must NOT brick
        // loading (the merge can't delete the inherited alias). The stale
        // built-in is pruned so `doctor --fix` can't coerce to an
        // out-of-enum value; a built-in whose target survives is kept.
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\n    enum: [open, in-progress, archived]\n",
        )
        .unwrap();
        let schema = load(tmp.path()).unwrap();
        // `closed → done` is stale (done not in narrowed enum) → pruned.
        assert_eq!(status_alias_target(&schema, "closed"), None);
        assert_eq!(would_coerce(&schema, "status", "closed"), None);
        // `in_progress → in-progress` survives (target still in enum).
        assert_eq!(
            status_alias_target(&schema, "in_progress"),
            Some("in-progress")
        );
    }

    #[test]
    fn alias_chain_rejected_even_without_enum() {
        // The chain rule is independent of enum presence.
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\nstatus_aliases:\n  a: b\n  b: c\n",
        )
        .unwrap();
        let err = load(tmp.path()).unwrap_err();
        assert!(format!("{err:#}").contains("alias chains are not resolved"));
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
        assert!(
            load(tmp.path()).is_err(),
            "typo'd `requried` must be rejected"
        );
    }
}
