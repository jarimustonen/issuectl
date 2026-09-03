//! Import / export of issues to and from portable formats.
//!
//! Two halves, both pure (no filesystem, no process spawning — the CLI
//! owns reading files and shelling out to `gh`):
//!
//! * **export** — render a slice of [`Issue`] to JSON, Markdown, or CSV
//!   on demand. JSON serializes the full [`Issue`] (every field); CSV and
//!   Markdown are lossy, human-oriented projections (metadata + title for
//!   CSV, metadata + body for Markdown — neither carries the issue body
//!   in CSV nor is re-importable).
//! * **import** — parse a foreign payload into [`ImportRecord`]s, the
//!   lenient intake shape. JSON intake reads both issuectl's own JSON
//!   export and hand-written arrays; [`parse_github`] reads `gh issue
//!   list --json …` output. Each record converts to a [`NewArgs`] via
//!   [`ImportRecord::into_new_args`] so the CLI funnels every imported
//!   issue through the same `do_new` validation path as `issuectl new`.
//!
//! **Import is content-level, not a byte-faithful round-trip.** Every
//! imported issue is *created fresh*: it gets a new slug and `open`
//! status, with today's `created`/`updated` dates. Source fields that
//! `do_new` does not accept are therefore dropped on import — `status`
//! (so closed issues, including GitHub `--state closed`, arrive open),
//! `closed`/`created`/`updated` timestamps, `commits`, `related`, and
//! custom (`extra`) frontmatter fields. Title, type, priority, labels,
//! assignee/reporter (or owner for epics), epic, and content survive. A foreign
//! `description` remains free text, while issuectl's exported `body` remains
//! structured Markdown. See [`ImportRecord`].

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::models::Issue;
use crate::mutate::new_issue::NewArgs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Markdown,
    Csv,
}

/// Render issues to the requested format. Caller has already applied any
/// query / folder filtering — `transfer` serializes exactly what it is
/// handed, in the order it is handed.
pub fn export(issues: &[Issue], format: ExportFormat) -> Result<String> {
    match format {
        ExportFormat::Json => export_json(issues),
        ExportFormat::Markdown => Ok(export_markdown(issues)),
        ExportFormat::Csv => Ok(export_csv(issues)),
    }
}

fn export_json(issues: &[Issue]) -> Result<String> {
    serde_json::to_string_pretty(issues).context("serializing issues to JSON")
}

fn export_markdown(issues: &[Issue]) -> String {
    let mut out = String::new();
    out.push_str("# Issues\n\n");
    if issues.is_empty() {
        out.push_str("_No issues._\n");
        return out;
    }
    for issue in issues {
        out.push_str(&format!("## {} ({})\n\n", issue.title, issue.slug));
        let mut meta: Vec<(&str, String)> = vec![
            ("type", issue.issue_type.clone()),
            ("status", issue.status.clone()),
            ("priority", issue.priority.clone()),
        ];
        if let Some(a) = &issue.assignee {
            meta.push(("assignee", a.clone()));
        }
        if let Some(o) = &issue.owner {
            meta.push(("owner", o.clone()));
        }
        if let Some(r) = &issue.reporter {
            meta.push(("reporter", r.clone()));
        }
        if let Some(e) = &issue.epic {
            meta.push(("epic", e.clone()));
        }
        if let Some(labels) = &issue.labels {
            if !labels.is_empty() {
                meta.push(("labels", labels.join(", ")));
            }
        }
        if let Some(c) = &issue.created {
            meta.push(("created", c.clone()));
        }
        if let Some(c) = &issue.closed {
            meta.push(("closed", c.clone()));
        }
        for (k, v) in meta {
            out.push_str(&format!("- **{k}**: {v}\n"));
        }
        out.push('\n');
        let body = issue.body.trim();
        if !body.is_empty() {
            out.push_str(body);
            out.push('\n');
        }
        out.push_str("\n---\n\n");
    }
    out
}

const CSV_HEADER: &[&str] = &[
    "slug", "type", "status", "priority", "assignee", "owner", "reporter", "epic", "labels",
    "title", "created", "updated", "closed",
];

fn export_csv(issues: &[Issue]) -> String {
    let mut out = String::new();
    out.push_str(&CSV_HEADER.join(","));
    out.push('\n');
    for issue in issues {
        let labels = issue
            .labels
            .as_ref()
            .map(|l| l.join(";"))
            .unwrap_or_default();
        let row = [
            issue.slug.as_str(),
            issue.issue_type.as_str(),
            issue.status.as_str(),
            issue.priority.as_str(),
            issue.assignee.as_deref().unwrap_or(""),
            issue.owner.as_deref().unwrap_or(""),
            issue.reporter.as_deref().unwrap_or(""),
            issue.epic.as_deref().unwrap_or(""),
            labels.as_str(),
            issue.title.as_str(),
            issue.created.as_deref().unwrap_or(""),
            issue.updated.as_deref().unwrap_or(""),
            issue.closed.as_deref().unwrap_or(""),
        ];
        let escaped: Vec<String> = row.iter().map(|f| csv_field(f)).collect();
        out.push_str(&escaped.join(","));
        out.push('\n');
    }
    out
}

/// RFC 4180 field quoting: wrap in double quotes and double any interior
/// quote when the value contains a comma, quote, CR, or LF.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Lenient intake record. Every field beyond `title` is optional so the
/// same shape parses issuectl's own JSON export, a hand-authored array,
/// and (after a field rename in [`parse_github`]) GitHub's `gh` output.
/// Unknown keys are ignored — this is deliberate so that the rich
/// [`Issue`] JSON export (which carries `slug`, `status`, `created`,
/// `commits`, … keys this shape does not model) parses without error.
/// The flip side is that a typo'd key (`asignee`) is silently dropped.
///
/// Only the fields modeled here cross into the created issue; see the
/// module-level note for the full list of source fields that are dropped
/// on import (status, dates, commits, related, slug, custom fields).
#[derive(Debug, Clone, Deserialize)]
pub struct ImportRecord {
    pub title: String,
    #[serde(rename = "type", default)]
    pub issue_type: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub reporter: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub epic: Option<String>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    #[serde(default)]
    pub source: Option<String>,
    /// Foreign free-text description. Import wraps this content in the
    /// generated `## Description` section.
    #[serde(default)]
    pub description: Option<String>,
    /// Structured Markdown from issuectl's own JSON export. Import recognizes
    /// the canonical matching document H1, removes it because the fresh issue
    /// renderer creates it again, and preserves the remaining section order and
    /// content subject to normal creation-time whitespace normalization and
    /// destination schema requirements.
    ///
    /// For compatibility with the former serde alias, an unversioned `body`
    /// without a leading H1 is treated as free text. `body` and `description`
    /// are mutually exclusive.
    #[serde(default)]
    pub body: Option<String>,
}

impl ImportRecord {
    /// Convert into the `do_new` argument shape. `default_type` supplies
    /// the issue type when the record omits one. Epics carry `owner` and
    /// never `reporter`/`assignee` (which `do_new` rejects for epics);
    /// non-epics carry `reporter`/`assignee` and never `owner`.
    pub fn into_new_args(self, default_type: &str) -> Result<NewArgs> {
        let issue_type = self
            .issue_type
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| default_type.to_string());
        let is_epic = issue_type == "epic";
        let content = ImportContent::decode(self.body, self.description, &self.title)?;
        let (description, structured_body) = match content {
            ImportContent::None => (None, false),
            ImportContent::FreeText(text) => (Some(text), false),
            ImportContent::Structured(body) => (Some(body), true),
        };
        Ok(NewArgs {
            issue_type,
            title: self.title,
            slug: None,
            // Imported records carry real, distinct titles: derive a
            // readable slug (with random fallback) just like `issuectl new`.
            slug_random: false,
            reporter: if is_epic { None } else { self.reporter },
            assignee: if is_epic { None } else { self.assignee },
            owner: if is_epic { self.owner } else { None },
            priority: self
                .priority
                .filter(|p| !p.trim().is_empty())
                .unwrap_or_else(|| "normal".to_string()),
            epic: self.epic,
            labels: self.labels.unwrap_or_default(),
            related: vec![],
            source: self.source,
            description,
            structured_body,
            custom_fields: vec![],
            lane: None,
            lane_seq: None,
            collision: vec![],
            status: None,
            inbox: false,
        })
    }
}

/// Explicit internal content representation after decoding the two distinct
/// JSON fields. The exact-H1 check is only the compatibility decoder for old,
/// unversioned issuectl exports; it is not inferred from a serde alias.
enum ImportContent {
    None,
    FreeText(String),
    Structured(String),
}

impl ImportContent {
    fn decode(body: Option<String>, description: Option<String>, title: &str) -> Result<Self> {
        match (body, description) {
            (Some(_), Some(_)) => anyhow::bail!(
                "import record {title:?} supplies both `body` and `description`; supply exactly one"
            ),
            (None, Some(description)) => Ok(Self::FreeText(description)),
            (None, None) => Ok(Self::None),
            (Some(body), None) => {
                if let Some(stripped) = strip_exported_title(&body, title) {
                    return Ok(Self::Structured(stripped.to_string()));
                }
                if let Some(heading) = leading_h1(&body) {
                    anyhow::bail!(
                        "import record {title:?} has body title H1 {heading:?}; it must match the record title"
                    );
                }
                // Before `body` and `description` were separate fields, `body`
                // was a serde alias for the free-text description. Preserve
                // that behavior for unversioned, non-document-shaped input.
                Ok(Self::FreeText(body))
            }
        }
    }
}

fn leading_h1(body: &str) -> Option<&str> {
    let after_marker = body.strip_prefix("# ")?;
    Some(
        after_marker
            .split_once('\n')
            .map_or(after_marker, |(heading, _)| heading)
            .trim_end_matches('\r'),
    )
}

/// Remove the matching title H1 emitted by issuectl's item renderer.
fn strip_exported_title<'a>(body: &'a str, title: &str) -> Option<&'a str> {
    let after_marker = body.strip_prefix("# ")?;
    let (heading, rest) = match after_marker.split_once('\n') {
        Some(parts) => parts,
        None if after_marker.trim_end_matches('\r') == title => return Some(""),
        None => return None,
    };
    if heading.trim_end_matches('\r') != title {
        return None;
    }
    Some(
        rest.strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\n'))
            .unwrap_or(rest),
    )
}

/// Parse a JSON import payload: either a top-level array of records or a
/// single record object. Dispatches on the parsed JSON shape (not a
/// fragile leading-character sniff) and tolerates a leading UTF-8 BOM,
/// which `serde_json` otherwise rejects.
pub fn parse_json(input: &str) -> Result<Vec<ImportRecord>> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let value: serde_json::Value =
        serde_json::from_str(input).context("parsing JSON import (invalid JSON)")?;
    match value {
        serde_json::Value::Array(_) => serde_json::from_value(value)
            .context("parsing JSON import (expected an array of issue objects)"),
        serde_json::Value::Object(_) => {
            let single: ImportRecord = serde_json::from_value(value)
                .context("parsing JSON import (expected an issue object)")?;
            Ok(vec![single])
        }
        _ => anyhow::bail!("JSON import must be an object or an array of objects"),
    }
}

// ── GitHub intake ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GithubIssue {
    title: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    labels: Vec<GithubLabel>,
    #[serde(default)]
    assignees: Vec<GithubUser>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GithubUser {
    login: String,
}

/// Parse the JSON emitted by
/// `gh issue list --json number,title,body,labels,state,assignees,url`.
/// Pull requests are not filtered here — the caller's `gh issue list`
/// already excludes them. Labels become issuectl labels, the first
/// assignee becomes the assignee, and the issue URL becomes the source.
pub fn parse_github(input: &str) -> Result<Vec<ImportRecord>> {
    let issues: Vec<GithubIssue> =
        serde_json::from_str(input).context("parsing `gh issue list --json …` output")?;
    Ok(issues
        .into_iter()
        .map(|g| ImportRecord {
            title: g.title,
            issue_type: None,
            priority: None,
            assignee: g.assignees.into_iter().next().map(|u| u.login),
            reporter: None,
            owner: None,
            epic: None,
            labels: Some(g.labels.into_iter().map(|l| l.name).collect()),
            source: g.url,
            description: g.body.filter(|b| !b.trim().is_empty()),
            body: None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn issue(slug: &str, title: &str) -> Issue {
        Issue {
            slug: slug.to_string(),
            folder: "open".to_string(),
            created: Some("2026-01-01".to_string()),
            status: "open".to_string(),
            updated: Some("2026-01-02".to_string()),
            priority: "normal".to_string(),
            issue_type: "bug".to_string(),
            reporter: Some("alice".to_string()),
            assignee: Some("bob".to_string()),
            owner: None,
            epic: None,
            related: None,
            labels: Some(vec!["interop".to_string()]),
            closed: None,
            closed_by: None,
            lane: None,
            collision: None,
            lane_seq: None,
            commits: None,
            title: title.to_string(),
            body: format!("# {title}\n\n## Description\n\nSomething broke."),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn export_json_round_trips_through_import() {
        let issues = vec![issue("amber-loud-fox", "Login loops")];
        let json = export(&issues, ExportFormat::Json).unwrap();
        let records = parse_json(&json).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].title, "Login loops");
        assert_eq!(records[0].issue_type.as_deref(), Some("bug"));
        assert_eq!(records[0].assignee.as_deref(), Some("bob"));
        assert!(records[0]
            .body
            .as_deref()
            .unwrap()
            .contains("Something broke"));
        assert!(records[0].description.is_none());
        let args = records
            .into_iter()
            .next()
            .unwrap()
            .into_new_args("task")
            .unwrap();
        assert!(args.structured_body);
        assert_eq!(
            args.description.as_deref(),
            Some("## Description\n\nSomething broke.")
        );
    }

    #[test]
    fn export_csv_has_header_and_escapes() {
        let mut i = issue("amber-loud-fox", "Title, with comma");
        i.body = String::new();
        let csv = export(&[i], ExportFormat::Csv).unwrap();
        let mut lines = csv.lines();
        assert_eq!(lines.next().unwrap(), CSV_HEADER.join(","));
        let row = lines.next().unwrap();
        assert!(row.contains("\"Title, with comma\""), "row was {row}");
        assert!(row.starts_with("amber-loud-fox,bug,open,normal,bob,,alice,,interop,"));
    }

    #[test]
    fn csv_field_escapes_quotes_and_newlines() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("line1\nline2"), "\"line1\nline2\"");
    }

    #[test]
    fn export_markdown_lists_metadata_and_body() {
        let md = export(
            &[issue("amber-loud-fox", "Login loops")],
            ExportFormat::Markdown,
        )
        .unwrap();
        assert!(md.contains("## Login loops (amber-loud-fox)"));
        assert!(md.contains("- **type**: bug"));
        assert!(md.contains("- **labels**: interop"));
        assert!(md.contains("Something broke."));
    }

    #[test]
    fn export_markdown_handles_empty() {
        let md = export(&[], ExportFormat::Markdown).unwrap();
        assert!(md.contains("_No issues._"));
    }

    #[test]
    fn parse_json_accepts_single_object() {
        let records = parse_json(r#"{"title":"Solo"}"#).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].title, "Solo");
        assert!(records[0].issue_type.is_none());
    }

    #[test]
    fn parse_json_accepts_array() {
        let records = parse_json(r#"[{"title":"A"},{"title":"B","type":"task"}]"#).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].issue_type.as_deref(), Some("task"));
    }

    #[test]
    fn parse_json_rejects_missing_title() {
        assert!(parse_json(r#"[{"type":"bug"}]"#).is_err());
    }

    #[test]
    fn into_new_args_defaults_type_and_priority() {
        let rec = parse_json(r#"{"title":"X"}"#).unwrap().pop().unwrap();
        let args = rec.into_new_args("task").unwrap();
        assert_eq!(args.issue_type, "task");
        assert_eq!(args.priority, "normal");
        assert!(args.slug.is_none());
    }

    #[test]
    fn into_new_args_drops_people_for_epic_but_keeps_owner() {
        let rec = parse_json(
            r#"{"title":"E","type":"epic","assignee":"bob","reporter":"al","owner":"cara"}"#,
        )
        .unwrap()
        .pop()
        .unwrap();
        let args = rec.into_new_args("task").unwrap();
        assert_eq!(args.issue_type, "epic");
        assert!(args.assignee.is_none());
        assert!(args.reporter.is_none());
        assert_eq!(args.owner.as_deref(), Some("cara"));
    }

    #[test]
    fn into_new_args_drops_owner_for_non_epic() {
        let rec = parse_json(r#"{"title":"B","type":"bug","owner":"cara"}"#)
            .unwrap()
            .pop()
            .unwrap();
        let args = rec.into_new_args("task").unwrap();
        assert!(args.owner.is_none());
    }

    #[test]
    fn parse_json_tolerates_leading_bom() {
        let records = parse_json("\u{feff}[{\"title\":\"A\"}]").unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].title, "A");
    }

    #[test]
    fn parse_json_rejects_non_object_array() {
        assert!(parse_json("42").is_err());
        assert!(parse_json("\"hi\"").is_err());
    }

    #[test]
    fn parse_github_maps_fields() {
        let payload = r#"[
            {"number":1,"title":"First","body":"Body text","state":"open",
             "labels":[{"name":"bug"},{"name":"p1"}],
             "assignees":[{"login":"octocat"}],
             "url":"https://github.com/o/r/issues/1"},
            {"number":2,"title":"Second","body":"","state":"closed",
             "labels":[],"assignees":[],"url":"https://github.com/o/r/issues/2"}
        ]"#;
        let records = parse_github(payload).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].title, "First");
        assert_eq!(records[0].labels.as_ref().unwrap(), &["bug", "p1"]);
        assert_eq!(records[0].assignee.as_deref(), Some("octocat"));
        assert_eq!(
            records[0].source.as_deref(),
            Some("https://github.com/o/r/issues/1")
        );
        // empty body becomes None
        assert!(records[1].description.is_none());
        assert!(records[1].body.is_none());
        assert!(records[1].assignee.is_none());

        let args = records[0].clone().into_new_args("task").unwrap();
        assert!(!args.structured_body);
        assert_eq!(args.description.as_deref(), Some("Body text"));
    }

    #[test]
    fn foreign_description_stays_free_text() {
        let rec = parse_json(r#"{"title":"Foreign","description":"Plain text"}"#)
            .unwrap()
            .pop()
            .unwrap();
        let args = rec.into_new_args("bug").unwrap();
        assert!(!args.structured_body);
        assert_eq!(args.description.as_deref(), Some("Plain text"));
    }

    #[test]
    fn body_and_description_are_rejected() {
        let rec = parse_json(
            r##"{"title":"Both","description":"Foreign","body":"# Both\n\n## Expected\n\nStructured"}"##,
        )
        .unwrap()
        .pop()
        .unwrap();
        let err = match rec.into_new_args("bug") {
            Err(err) => err,
            Ok(_) => panic!("both content fields must be rejected"),
        };
        assert!(
            err.to_string()
                .contains("supplies both `body` and `description`"),
            "{err:#}"
        );
    }

    #[test]
    fn legacy_plain_and_empty_body_stay_free_text() {
        for body in ["Plain text", ""] {
            let json = serde_json::json!({"title": "Legacy", "body": body}).to_string();
            let rec = parse_json(&json).unwrap().pop().unwrap();
            let args = rec.into_new_args("bug").unwrap();
            assert!(!args.structured_body);
            assert_eq!(args.description.as_deref(), Some(body));
        }
    }

    #[test]
    fn title_only_body_is_recognized_as_structured() {
        let rec = parse_json(r##"{"title":"Title","body":"# Title"}"##)
            .unwrap()
            .pop()
            .unwrap();
        let args = rec.into_new_args("bug").unwrap();
        assert!(args.structured_body);
        assert_eq!(args.description.as_deref(), Some(""));
    }

    #[test]
    fn structured_body_rejects_a_nonmatching_h1() {
        let rec = parse_json(r##"{"title":"Title","body":"# Other\n\nContent"}"##)
            .unwrap()
            .pop()
            .unwrap();
        let err = match rec.into_new_args("bug") {
            Err(err) => err,
            Ok(_) => panic!("a mismatched document H1 must be rejected"),
        };
        assert!(err.to_string().contains("must match the record title"));
    }

    #[test]
    fn exported_title_stripping_handles_crlf() {
        assert_eq!(
            strip_exported_title("# Title\r\n\r\n## Description\r\n", "Title"),
            Some("## Description\r\n")
        );
    }
}
