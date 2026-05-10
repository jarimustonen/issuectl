//! User-defined boards. See `docs/design/custom-boards.md`.
//!
//! A board YAML file at `.issuectl/boards/<name>.yaml` declares a
//! group_by axis (built-in scalar field or custom scalar from
//! `.schema.yaml`) plus an explicit ordered column list. The server
//! reads boards on every request — no caching beyond the schema cache
//! the loader already participates in.
//!
//! The two error tiers exist so the route can decide between "hide the
//! board" (404) and "show the board read-only" (200 + banner). Hard
//! errors are author-fixable in the YAML; soft errors are author-
//! fixable in `.schema.yaml` or the filter expression — different
//! files, different banner copy.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::schema;

/// Built-in scalar fields a board may group on. Multi-valued fields
/// (`labels`, `related`) are intentionally absent — see the design
/// note for the v1 scope decision.
const BUILTIN_SCALAR_GROUP_BY: &[&str] = &[
    "epic", "assignee", "owner", "priority", "type", "reporter", "status",
];

/// Schema fields that exist but are list-typed. Listed for a clearer
/// error message — the loader rejects them up front instead of
/// letting the user discover at runtime that drag does nothing
/// sensible.
fn is_schema_list_field(schema: &schema::Schema, name: &str) -> bool {
    schema.fields.get(name).map(|s| s.list).unwrap_or(false)
}

#[derive(Debug, Clone)]
pub struct Board {
    pub name: String,
    pub group_by: String,
    pub columns: Vec<BoardColumn>,
    pub filter: Option<String>,
    /// Set when the board file itself is well-formed but its
    /// `group_by` field is missing from the schema or its `filter`
    /// fails to parse. The route returns 200 with `read_only=true`;
    /// the JS surfaces a banner and disables drag.
    pub read_only_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BoardColumn {
    pub value: String,
    pub label: String,
}

#[derive(Debug)]
pub enum BoardError {
    NotFound,
    /// Hard validation failure in the YAML itself (parse error,
    /// duplicate column values, list-typed group_by, ...). Mapped to
    /// 404 by the route — a board this broken has no useful read-only
    /// fallback.
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

/// Slug-shaped predicate; matches the issue slug rules so the URL
/// `/board/<name>` survives a round-trip without escaping. Loose
/// enough to allow underscores too because YAML basenames typically
/// permit them.
fn is_valid_board_name(name: &str) -> bool {
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

pub fn load(root: &Path, name: &str) -> Result<Board, BoardError> {
    if !is_valid_board_name(name) {
        return Err(BoardError::NotFound);
    }
    let path = board_file_path(root, name);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(BoardError::NotFound),
        Err(e) => return Err(BoardError::Io(e.into())),
    };
    let file: BoardFile = serde_yaml::from_str(&text)
        .map_err(|e| BoardError::Validation(format!("parse {}: {e}", path.display())))?;

    if file.name != name {
        return Err(BoardError::Validation(format!(
            "board name {:?} disagrees with filename {:?}",
            file.name, name
        )));
    }
    if file.columns.is_empty() {
        return Err(BoardError::Validation(
            "columns must not be empty".to_string(),
        ));
    }
    let mut seen = BTreeSet::new();
    for c in &file.columns {
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

    // Pre-flight schema lookup to classify hard-vs-soft errors.
    let schema_arc =
        schema::load(root).map_err(|e| BoardError::Io(anyhow::anyhow!("load schema: {e}")))?;
    if is_schema_list_field(&schema_arc, &file.group_by) {
        return Err(BoardError::Validation(format!(
            "group_by {:?} is a list-typed field; v1 boards only support scalar fields",
            file.group_by
        )));
    }

    let mut read_only_reason: Option<String> = None;
    let group_by_known = BUILTIN_SCALAR_GROUP_BY.contains(&file.group_by.as_str())
        || schema_arc.fields.contains_key(&file.group_by);
    if !group_by_known {
        read_only_reason = Some(format!(
            "group_by field {:?} is not declared in .schema.yaml; board is read-only",
            file.group_by
        ));
    }

    if let Some(filter) = &file.filter {
        let trimmed = filter.trim();
        if !trimmed.is_empty() {
            if let Err(e) = crate::query::parse(trimmed) {
                read_only_reason = Some(format!("filter does not parse ({e}); board is read-only"));
            }
        }
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
        filter: file.filter.filter(|s| !s.trim().is_empty()),
        read_only_reason,
    })
}

/// Resolve the group_by value of an issue for a given field name.
/// Returns the empty string when the field is missing/null — that's
/// the empty-bucket key.
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
            Some(serde_json::Value::Null) | None => String::new(),
            // Numeric/bool/etc. round-trip through to_string for a
            // reasonable display; v1 docs say boards target scalar
            // fields, so this is the rare fallback.
            Some(other) => other.to_string(),
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
        let b = load(d.path(), "triage").unwrap();
        assert_eq!(b.group_by, "epic");
        assert_eq!(b.columns.len(), 2);
        assert!(b.read_only_reason.is_none());
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
        match load(d.path(), "nope") {
            Err(BoardError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_invalid_name() {
        let d = tmp();
        match load(d.path(), "../etc") {
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
        match load(d.path(), "triage") {
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
        match load(d.path(), "triage") {
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
        match load(d.path(), "triage") {
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
        match load(d.path(), "triage") {
            Err(BoardError::Validation(s)) => assert!(s.contains("list")),
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
        let b = load(d.path(), "triage").unwrap();
        assert!(b
            .read_only_reason
            .as_ref()
            .unwrap()
            .contains("not declared"));
    }

    #[test]
    fn load_bad_filter_renders_read_only() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns: [{value: '', label: U}]\nfilter: \"bogus:value\"\n",
        );
        let b = load(d.path(), "triage").unwrap();
        assert!(b.read_only_reason.as_ref().unwrap().contains("filter"));
    }

    #[test]
    fn load_rejects_unknown_top_level_keys() {
        let d = tmp();
        write_board(
            d.path(),
            "triage",
            "name: triage\ngroup_by: epic\ncolumns: [{value: '', label: U}]\nbogus: 1\n",
        );
        match load(d.path(), "triage") {
            Err(BoardError::Validation(_)) => {}
            other => panic!("expected Validation, got {other:?}"),
        }
    }
}
