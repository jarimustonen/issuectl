//! User-defined boards. See `docs/design/custom-boards.md`.
//!
//! A board YAML file at `.issuectl/boards/<name>.yaml` declares a
//! group_by axis (built-in scalar field or custom scalar from
//! `.schema.yaml`) plus an explicit ordered column list. The server
//! reads boards on every request — no caching beyond the schema cache
//! the loader already participates in.
//!
//! Validation philosophy follows `AGENTS-AI-FIRST-CLI.md`: the loader
//! rejects malformed input strictly so the AI caller can fix its YAML
//! and retry. Whitespace, unknown enum values, list-typed group_by,
//! and empty columns on required fields are all hard errors. The only
//! soft (read-only banner) failure mode is when the board file is
//! itself well-formed but references something *outside* the board
//! YAML that's now broken — typically a `.schema.yaml` field that
//! disappeared. That's a different file's problem; the banner points
//! the user there rather than 404'ing the URL.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::query;

/// Built-in scalar fields a board may group on. Multi-valued fields
/// (`labels`, `related`) are intentionally absent — see the design
/// note for the v1 scope decision.
const BUILTIN_SCALAR_GROUP_BY: &[&str] = &[
    "epic", "assignee", "owner", "priority", "type", "reporter", "status",
];

/// Built-in list-typed fields that v1 boards explicitly reject. Listed
/// independently of `.schema.yaml` so a user-edited schema that omits
/// them still gets the right error message ("list-typed not
/// supported") instead of falling through to the generic "not
/// declared" path.
const BUILTIN_LIST_FIELDS: &[&str] = &["labels", "related"];

/// Built-in scalars that cannot be cleared (`null`). Boards grouping
/// on these reject the empty `value: ""` column at load time — the
/// server's PATCH would 422 every drag attempt anyway.
const BUILTIN_NON_NULLABLE: &[&str] = &["priority", "type", "status"];

/// Filter-bar field keys the JS recognizes. A board YAML's
/// `filters: [...]` may include any subset of these. Empty (or
/// omitted) means "hide all", matching the v0 behavior of the custom
/// board.
const FILTER_KEYS: &[&str] = &["search", "type", "assignee", "epic", "label"];

#[derive(Debug, Clone)]
pub struct Board {
    pub name: String,
    pub group_by: String,
    pub columns: Vec<BoardColumn>,
    pub filter_src: Option<String>,
    /// Pre-parsed `filter:` query, populated at load time so the route
    /// never re-parses or guesses what to skip. `None` when no filter
    /// was declared.
    pub parsed_filter: Option<query::Query>,
    /// Filter-bar fields the client should render (subset of
    /// `FILTER_KEYS`). Empty vec = hide the filter bar entirely.
    pub filters: Vec<String>,
    /// Typed soft errors. Empty vec = healthy board. Non-empty = render
    /// read-only with all listed reasons surfaced; drag is disabled.
    pub soft_errors: Vec<SoftError>,
}

#[derive(Debug, Clone)]
pub struct BoardColumn {
    pub value: String,
    pub label: String,
}

/// Soft errors are recoverable misconfigurations whose root cause
/// lives *outside* the board YAML — typically `.schema.yaml`. The
/// route renders the board read-only with a banner; the caller fixes
/// the schema (or filter expression) and reloads.
#[derive(Debug, Clone)]
pub enum SoftError {
    /// `group_by` references a field that's neither a built-in nor
    /// declared in `.schema.yaml`. Carries the offending field name.
    UnknownGroupBy(String),
}

impl SoftError {
    pub fn message(&self) -> String {
        match self {
            SoftError::UnknownGroupBy(field) => format!(
                "group_by field {field:?} is not declared in .schema.yaml; \
                 add it (or fix the typo) and reload"
            ),
        }
    }
}

#[derive(Debug)]
pub enum BoardError {
    NotFound,
    /// Hard validation failure — the YAML itself is wrong. Mapped to
    /// 404 by the route. Carries a public-safe message (no filesystem
    /// paths).
    Validation(String),
    Io(anyhow::Error),
}

impl std::fmt::Display for BoardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoardError::NotFound => write!(f, "board not found"),
            BoardError::Validation(s) => write!(f, "validation: {s}"),
            BoardError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for BoardError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardFile {
    name: String,
    group_by: String,
    columns: Vec<BoardColumnFile>,
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    filters: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardColumnFile {
    #[serde(default)]
    value: String,
    label: String,
}

/// Boards directory under the repo root.
pub fn boards_dir(root: &Path) -> PathBuf {
    root.join(".issuectl").join("boards")
}

fn board_file_path(root: &Path, name: &str) -> PathBuf {
    boards_dir(root).join(format!("{name}.yaml"))
}

/// Slug-shaped predicate: lowercase alphanumeric, hyphens, underscores.
/// Public so route handlers reuse the same predicate as the loader —
/// keeping two copies in sync was the original maintenance hazard.
pub fn is_valid_board_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// List board names available under `.issuectl/boards/`. Returns an
/// empty vec if the directory does not exist; sorted ascending so the
/// API response is stable across runs.
pub fn list(root: &Path) -> Vec<String> {
    let dir = boards_dir(root);
    let entries = match std::fs::read_dir(&dir) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.extension().map(|x| x == "yaml").unwrap_or(false) {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .filter(|s| is_valid_board_name(s))
                    .map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();
    names.sort();
    names
}

pub fn load(
    root: &Path,
    name: &str,
    config: &dyn crate::repo_config::ConfigSource,
) -> Result<Board, BoardError> {
    if !is_valid_board_name(name) {
        return Err(BoardError::NotFound);
    }
    let path = board_file_path(root, name);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(BoardError::NotFound),
        Err(e) => return Err(BoardError::Io(e.into())),
    };
    // Public-safe parse error: no filesystem path. Operators read the
    // detailed path from server logs; clients see a localized message.
    let file: BoardFile = serde_yaml::from_str(&text)
        .map_err(|e| BoardError::Validation(format!("YAML parse error: {e}")))?;

    if file.name != name {
        return Err(BoardError::Validation(format!(
            "board name {:?} disagrees with filename {:?}",
            file.name, name
        )));
    }
    if file.group_by.trim() != file.group_by {
        return Err(BoardError::Validation(format!(
            "group_by {:?} must not have leading/trailing whitespace",
            file.group_by
        )));
    }
    if file.group_by.is_empty() {
        return Err(BoardError::Validation(
            "group_by must not be empty".to_string(),
        ));
    }
    if file.columns.is_empty() {
        return Err(BoardError::Validation(
            "columns must not be empty".to_string(),
        ));
    }
    let mut seen = BTreeSet::new();
    for c in &file.columns {
        if c.value.trim() != c.value {
            return Err(BoardError::Validation(format!(
                "column value {:?} must not have leading/trailing whitespace",
                c.value
            )));
        }
        if c.label.trim().is_empty() {
            return Err(BoardError::Validation(format!(
                "column with value {:?}: label must not be empty",
                c.value
            )));
        }
        if !seen.insert(c.value.clone()) {
            return Err(BoardError::Validation(format!(
                "duplicate column value {:?}",
                c.value
            )));
        }
    }

    // Built-in list fields are rejected explicitly so the diagnostic
    // is correct even if the user's `.schema.yaml` doesn't declare
    // them. Schema-declared list fields fall through to the same
    // error, just via the schema lookup.
    if BUILTIN_LIST_FIELDS.contains(&file.group_by.as_str()) {
        return Err(BoardError::Validation(format!(
            "group_by {:?} is a list-typed field; v1 boards only support scalar fields",
            file.group_by
        )));
    }
    let schema_arc = config
        .schema(root)
        .map_err(|e| BoardError::Io(anyhow::anyhow!("load schema: {e}")))?;
    if schema_arc
        .fields
        .get(&file.group_by)
        .map(|s| s.list)
        .unwrap_or(false)
    {
        return Err(BoardError::Validation(format!(
            "group_by {:?} is a list-typed field; v1 boards only support scalar fields",
            file.group_by
        )));
    }

    // Required built-ins reject the empty/unassigned column.
    if BUILTIN_NON_NULLABLE.contains(&file.group_by.as_str())
        && file.columns.iter().any(|c| c.value.is_empty())
    {
        return Err(BoardError::Validation(format!(
            "group_by {:?} is a required field; the empty/unassigned column is not allowed \
             (a drop here would clear a required value and 422 every time)",
            file.group_by
        )));
    }

    // Enum-constrained fields: column values must be in the schema's
    // `enum` list. Empty (clear) is allowed only for nullable fields,
    // which is enforced above.
    if let Some(spec) = schema_arc.fields.get(&file.group_by) {
        if let Some(allowed) = &spec.allowed {
            for c in &file.columns {
                if c.value.is_empty() {
                    continue; // clear semantics; nullability already checked
                }
                if !allowed.iter().any(|v| v == &c.value) {
                    return Err(BoardError::Validation(format!(
                        "column value {:?} is not allowed by .schema.yaml's enum for {:?}: {:?}",
                        c.value, file.group_by, allowed
                    )));
                }
            }
        }
    }

    // Filter parses to a real `Query` at load time, hard-rejected on
    // failure. The route never re-parses; it consumes
    // `Board::parsed_filter` directly. A typo in `filter:` is the
    // caller's bug — fail loudly so it can be fixed, rather than
    // silently rendering all issues with a banner.
    let (filter_src, parsed_filter) = match &file.filter {
        Some(f) if !f.trim().is_empty() => {
            let trimmed = f.trim();
            match query::parse(trimmed) {
                Ok(q) => (Some(trimmed.to_string()), Some(q)),
                Err(e) => {
                    return Err(BoardError::Validation(format!(
                        "filter does not parse: {e}"
                    )));
                }
            }
        }
        _ => (None, None),
    };

    // Validate the filter-bar config: each entry must be one of
    // `FILTER_KEYS`, no duplicates. Default (omitted) is hide-all
    // (empty Vec).
    let filters = match file.filters {
        Some(list) => {
            let mut seen = BTreeSet::new();
            let mut out = Vec::new();
            for f in list {
                if !FILTER_KEYS.contains(&f.as_str()) {
                    return Err(BoardError::Validation(format!(
                        "filters entry {f:?} is not a known filter key; allowed: {FILTER_KEYS:?}"
                    )));
                }
                if !seen.insert(f.clone()) {
                    return Err(BoardError::Validation(format!(
                        "filters entry {f:?} listed twice"
                    )));
                }
                out.push(f);
            }
            out
        }
        None => Vec::new(),
    };

    // Soft-error pass: things that may have changed under the board's
    // feet without it being a YAML defect. Today: the schema field
    // disappeared. (filter was hard-rejected above; list-typed was
    // hard-rejected above.)
    let mut soft_errors = Vec::new();
    let group_by_known = BUILTIN_SCALAR_GROUP_BY.contains(&file.group_by.as_str())
        || schema_arc.fields.contains_key(&file.group_by);
    if !group_by_known {
        soft_errors.push(SoftError::UnknownGroupBy(file.group_by.clone()));
    }

    Ok(Board {
        name: file.name,
        group_by: file.group_by,
        columns: file
            .columns
            .into_iter()
            .map(|c| BoardColumn {
                value: c.value,
                label: c.label,
            })
            .collect(),
        filter_src,
        parsed_filter,
        filters,
        soft_errors,
    })
}

/// Resolve the group_by value of an issue for a given field name.
/// Returns the empty string when the field is missing/null — that's
/// the empty-bucket key. Non-string values from `extra` (numbers,
/// bools) round-trip via `to_string`; arrays/objects are rejected at
/// load time via the list-typed check, so this is the sane scalar
/// fallback.
pub fn group_value_for(issue: &crate::models::Issue, group_by: &str) -> String {
    match group_by {
        "epic" => issue.epic.clone().unwrap_or_default(),
        "assignee" => issue.assignee.clone().unwrap_or_default(),
        "owner" => issue.owner.clone().unwrap_or_default(),
        "reporter" => issue.reporter.clone().unwrap_or_default(),
        "priority" => issue.priority.clone(),
        "type" => issue.issue_type.clone(),
        "status" => issue.status.clone(),
        other => match issue.extra.get(other) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            Some(serde_json::Value::Bool(b)) => b.to_string(),
            Some(serde_json::Value::Null) | None => String::new(),
            // Arrays/objects in a "scalar" custom field are
            // misdeclared schema. Treat as the unassigned bucket
            // rather than stringifying to a JSON literal that no
            // user-declared column will ever match.
            Some(_) => String::new(),
        },
    }
}

/// True if `group_by` maps to a dedicated `UpdateIssueRequest` slot
/// (so the JS PATCHes `{<field>: ...}` rather than `custom_fields`).
pub fn is_builtin_group_by(group_by: &str) -> bool {
    BUILTIN_SCALAR_GROUP_BY.contains(&group_by)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write_board(root: &Path, name: &str, body: &str) {
        let dir = boards_dir(root);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{name}.yaml")), body).unwrap();
    }

    #[test]
    fn load_minimal_epic_board() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns:\n  - value: \"\"\n    label: Unscoped\n  - value: foo\n    label: Foo\n",
        );
        let b = load(d.path(), "triage", &crate::repo_config::UncachedConfig).unwrap();
        assert_eq!(b.group_by, "epic");
        assert_eq!(b.columns.len(), 2);
        assert!(b.soft_errors.is_empty());
        assert!(b.filters.is_empty());
    }

    #[test]
    fn list_returns_sorted_names() {
        let d = tmp();
        write_board(
            d.path(),
            "z",
            "name: z\ngroup_by: epic\ncolumns: [{value: '', label: U}]\n",
        );
        write_board(
            d.path(),
            "a",
            "name: a\ngroup_by: epic\ncolumns: [{value: '', label: U}]\n",
        );
        let names = list(d.path());
        assert_eq!(names, vec!["a".to_string(), "z".to_string()]);
    }

    #[test]
    fn load_missing_board_is_not_found() {
        let d = tmp();
        match load(d.path(), "nope", &crate::repo_config::UncachedConfig) {
            Err(BoardError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_invalid_name() {
        let d = tmp();
        match load(d.path(), "../etc", &crate::repo_config::UncachedConfig) {
            Err(BoardError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_filename_mismatch() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: other\ngroup_by: epic\ncolumns: [{value: '', label: U}]\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(_)) => {}
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_empty_columns() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns: []\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(_)) => {}
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_duplicate_column_values() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns:\n  - {value: a, label: A1}\n  - {value: a, label: A2}\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(s)) => assert!(s.contains("duplicate")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_list_typed_group_by() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: labels\ncolumns: [{value: '', label: U}]\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(s)) => assert!(s.contains("list")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// Even if the user's `.schema.yaml` happens not to declare
    /// `labels`, the loader still rejects it as a list-typed built-in
    /// — the `BUILTIN_LIST_FIELDS` constant is the source of truth.
    #[test]
    fn load_rejects_labels_even_without_schema_declaration() {
        let d = tmp();
        // Schema overrides labels as scalar; we still reject because
        // labels is built-in list-typed.
        let schema_path = d.path().join("issues").join(".schema.yaml");
        fs::create_dir_all(schema_path.parent().unwrap()).unwrap();
        fs::write(
            &schema_path,
            "version: 1\nfields:\n  labels:\n    required: false\n",
        )
        .unwrap();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: labels\ncolumns: [{value: '', label: U}]\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(s)) => assert!(s.contains("list-typed")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn load_unknown_group_by_renders_read_only() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: nonexistent_field\ncolumns: [{value: '', label: U}]\n",
        );
        let b = load(d.path(), "triage", &crate::repo_config::UncachedConfig).unwrap();
        assert_eq!(b.soft_errors.len(), 1);
        match &b.soft_errors[0] {
            SoftError::UnknownGroupBy(f) => assert_eq!(f, "nonexistent_field"),
        }
    }

    #[test]
    fn load_bad_filter_is_hard_rejected() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns: [{value: '', label: U}]\nfilter: \"bogus:value\"\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(s)) => assert!(s.contains("filter")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_empty_column_on_required_builtin() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: priority\ncolumns:\n  - {value: '', label: Unscoped}\n  - {value: high, label: High}\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(s)) => assert!(s.contains("required")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_column_value_outside_schema_enum() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: priority\ncolumns:\n  - {value: medium, label: Medium}\n  - {value: high, label: High}\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(s)) => {
                assert!(s.contains("medium"), "msg: {s}");
                assert!(s.contains("enum"), "msg: {s}");
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn load_accepts_in_enum_column_values() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: priority\ncolumns:\n  - {value: normal, label: Normal}\n  - {value: high, label: High}\n",
        );
        let b = load(d.path(), "triage", &crate::repo_config::UncachedConfig).unwrap();
        assert_eq!(b.columns.len(), 2);
    }

    #[test]
    fn load_rejects_whitespace_in_group_by() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: \"epic \"\ncolumns: [{value: '', label: U}]\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(s)) => assert!(s.contains("whitespace")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_whitespace_in_column_value() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns: [{value: 'foo ', label: F}]\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(s)) => assert!(s.contains("whitespace")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_unknown_top_level_keys() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns: [{value: '', label: U}]\nbogus: 1\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(_)) => {}
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn load_accepts_filters_subset() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns: [{value: '', label: U}]\nfilters: [search, type]\n",
        );
        let b = load(d.path(), "triage", &crate::repo_config::UncachedConfig).unwrap();
        assert_eq!(b.filters, vec!["search".to_string(), "type".to_string()]);
    }

    #[test]
    fn load_rejects_unknown_filter_key() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns: [{value: '', label: U}]\nfilters: [bogus]\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(s)) => assert!(s.contains("bogus")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_duplicate_filter_key() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns: [{value: '', label: U}]\nfilters: [search, search]\n",
        );
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(s)) => assert!(s.contains("twice")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn load_parses_filter_into_query() {
        let d = tmp();
        write_board(
            d.path(),
            "bugs",
            "name: bugs\ngroup_by: epic\ncolumns: [{value: '', label: U}]\nfilter: \"type:bug\"\n",
        );
        let b = load(d.path(), "bugs", &crate::repo_config::UncachedConfig).unwrap();
        assert!(b.parsed_filter.is_some());
        assert_eq!(b.filter_src.as_deref(), Some("type:bug"));
    }

    #[test]
    fn validation_error_does_not_leak_filesystem_path() {
        let d = tmp();
        write_board(d.path(), "triage", "this is not yaml: : :\n: : :");
        match load(d.path(), "triage", &crate::repo_config::UncachedConfig) {
            Err(BoardError::Validation(s)) => {
                assert!(!s.contains(d.path().to_str().unwrap()), "leaked path: {s}");
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }
}
