//! Agent context bundle — gathers an issue plus its surroundings (parent
//! epic, related/blocking issues, body sections, schema rules, commits)
//! into a deterministic structure for downstream LLM prompts. Read-side
//! only: never mutates anything under `issues/`.
//!
//! Determinism notes:
//! - Map-shaped fields use `BTreeMap` so JSON serialization is stable.
//! - Lists derived from frontmatter (`related`, `blocked_by`) are
//!   deduplicated and sorted before resolution.
//! - The markdown renderer emits sections in a fixed order.
//!
//! The same `Bundle` powers both the markdown surface (`issuectl
//! context`) and template substitution (`issuectl prompt`).
//
// Substitution is intentionally minimal: `{{key}}` (whitespace tolerated
// inside braces) and a flat key namespace. We considered Tera/Handlebars
// but a stronger templating engine would invite logic in templates,
// which is exactly what we don't want — the prompt is meant to be a
// frozen artefact of the bundle.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use serde::Serialize;

use crate::body_sections;
use crate::models::{Commit, Issue};
use crate::repo;
use crate::schema;

/// Section names we lift out of an issue body for the bundle. Listed in
/// the order they appear in the rendered markdown.
const ACCEPTANCE_SECTIONS: &[&str] = &["Acceptance Criteria", "Quick Test"];

#[derive(Debug, Clone, Serialize)]
pub struct Bundle {
    pub issue: BundleIssue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epic: Option<BundleEpic>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub sections: BTreeMap<String, String>,
    pub related_issues: Vec<RelatedRef>,
    pub blocking_issues: Vec<RelatedRef>,
    pub schema: SchemaSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct BundleIssue {
    pub slug: String,
    pub title: String,
    #[serde(rename = "type")]
    pub issue_type: String,
    pub status: String,
    pub priority: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reporter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epic: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<Commit>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BundleEpic {
    pub slug: String,
    pub title: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelatedRef {
    pub slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub issue_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchemaSummary {
    pub version: u32,
    pub fields: Vec<SchemaFieldSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchemaFieldSummary {
    pub name: String,
    pub required: bool,
    pub list: bool,
    #[serde(skip_serializing_if = "Option::is_none", rename = "enum")]
    pub allowed: Option<Vec<String>>,
}

/// Build the bundle for `slug`. Loads all issues to resolve references
/// rather than performing N file reads — the load is already cached
/// inside `repo::load_issues` for this run.
pub fn build(root: &Path, slug: &str) -> Result<Bundle> {
    let all = repo::load_issues(root);
    let issue = all
        .iter()
        .find(|i| i.slug == slug)
        .with_context(|| format!("issue {slug} not found"))?
        .clone();

    let blocked_by = read_blocked_by(root, slug)?;
    let related_slugs = related_slugs(&issue);
    let related_issues = resolve_refs(&related_slugs, &all);
    let blocking_issues = resolve_refs(&blocked_by, &all);

    let mut sections = BTreeMap::new();
    for name in ACCEPTANCE_SECTIONS {
        if let Some(text) = body_sections::extract_section_text(&issue.body, name) {
            if !text.trim().is_empty() {
                sections.insert((*name).to_string(), text);
            }
        }
    }

    let epic = issue
        .epic
        .as_deref()
        .and_then(|epic_slug| all.iter().find(|i| i.slug == epic_slug))
        .map(|e| BundleEpic {
            slug: e.slug.clone(),
            title: e.title.clone(),
            status: e.status.clone(),
            goal: body_sections::extract_section_text(&e.body, "Goal")
                .filter(|s| !s.trim().is_empty()),
            scope: body_sections::extract_section_text(&e.body, "Scope")
                .filter(|s| !s.trim().is_empty()),
        });

    let schema = load_schema_summary(root);

    let bundle_issue = BundleIssue {
        slug: issue.slug.clone(),
        title: issue.title.clone(),
        issue_type: issue.issue_type.clone(),
        status: issue.status.clone(),
        priority: issue.priority.clone(),
        created: issue.created.clone(),
        updated: issue.updated.clone(),
        closed: issue.closed.clone(),
        reporter: issue.reporter.clone(),
        assignee: issue.assignee.clone(),
        owner: issue.owner.clone(),
        epic: issue.epic.clone(),
        labels: issue.labels.clone().unwrap_or_default(),
        related: related_slugs,
        blocked_by,
        commits: issue.commits.clone().unwrap_or_default(),
        body: issue.body.clone(),
    };

    Ok(Bundle {
        issue: bundle_issue,
        epic,
        sections,
        related_issues,
        blocking_issues,
        schema,
    })
}

/// Strip the leading `@` sigil that frontmatter uses for cross-references
/// and drop legacy `#NN` numeric refs (those don't resolve to a slug).
fn normalise_ref(raw: &str) -> Option<String> {
    let t = raw.trim();
    if let Some(slug) = t.strip_prefix('@') {
        if !slug.is_empty() {
            return Some(slug.to_string());
        }
    }
    if t.starts_with('#') {
        return None;
    }
    if !t.is_empty() {
        Some(t.to_string())
    } else {
        None
    }
}

fn related_slugs(issue: &Issue) -> Vec<String> {
    let raw = issue.related.clone().unwrap_or_default();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for r in raw {
        if let Some(s) = normalise_ref(&r) {
            if seen.insert(s.clone()) {
                out.push(s);
            }
        }
    }
    out.sort();
    out
}

/// `blocked_by` is not part of the typed `Frontmatter` struct (it's a
/// schema-extensible custom field), so we re-read the YAML mapping to
/// pull it out. Accepts a list of strings or a single string.
fn read_blocked_by(root: &Path, slug: &str) -> Result<Vec<String>> {
    let located = repo::locate_issue_full(root, slug)?;
    let text = fs::read_to_string(&located.item_path)
        .with_context(|| format!("cannot read {}", located.item_path.display()))?;
    let (fm, _body) = crate::parser::split_frontmatter(&text);
    let Some(yaml_text) = fm else {
        return Ok(Vec::new());
    };
    let Ok(map) = serde_yaml::from_str::<serde_yaml::Mapping>(yaml_text) else {
        return Ok(Vec::new());
    };
    let Some(v) = map.get(serde_yaml::Value::String("blocked_by".into())) else {
        return Ok(Vec::new());
    };
    let raw: Vec<String> = match v {
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect(),
        serde_yaml::Value::String(s) => vec![s.clone()],
        _ => Vec::new(),
    };
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for r in raw {
        if let Some(s) = normalise_ref(&r) {
            if seen.insert(s.clone()) {
                out.push(s);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn resolve_refs(slugs: &[String], all: &[Issue]) -> Vec<RelatedRef> {
    slugs
        .iter()
        .map(|s| match all.iter().find(|i| i.slug == *s) {
            Some(i) => RelatedRef {
                slug: i.slug.clone(),
                title: Some(i.title.clone()),
                status: Some(i.status.clone()),
                issue_type: Some(i.issue_type.clone()),
            },
            None => RelatedRef {
                slug: s.clone(),
                title: None,
                status: None,
                issue_type: None,
            },
        })
        .collect()
}

fn load_schema_summary(root: &Path) -> SchemaSummary {
    let s = schema::load(root).unwrap_or_else(|_| schema::default_schema());
    let fields: Vec<_> = s
        .fields
        .iter()
        .map(|(name, spec)| SchemaFieldSummary {
            name: name.clone(),
            required: spec.required,
            list: spec.list,
            allowed: spec.allowed.clone(),
        })
        .collect();
    SchemaSummary {
        version: s.version,
        fields,
    }
}

// ── Rendering ─────────────────────────────────────────────────────────

pub fn render_markdown(b: &Bundle) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Context: {}\n\n", b.issue.slug));
    out.push_str(&format!("## Issue\n\n- title: {}\n", b.issue.title));
    out.push_str(&format!("- type: {}\n", b.issue.issue_type));
    out.push_str(&format!("- status: {}\n", b.issue.status));
    out.push_str(&format!("- priority: {}\n", b.issue.priority));
    if let Some(v) = &b.issue.created {
        out.push_str(&format!("- created: {v}\n"));
    }
    if let Some(v) = &b.issue.updated {
        out.push_str(&format!("- updated: {v}\n"));
    }
    if let Some(v) = &b.issue.closed {
        out.push_str(&format!("- closed: {v}\n"));
    }
    if let Some(v) = &b.issue.assignee {
        out.push_str(&format!("- assignee: {v}\n"));
    }
    if let Some(v) = &b.issue.owner {
        out.push_str(&format!("- owner: {v}\n"));
    }
    if let Some(v) = &b.issue.reporter {
        out.push_str(&format!("- reporter: {v}\n"));
    }
    if !b.issue.labels.is_empty() {
        out.push_str(&format!("- labels: {}\n", b.issue.labels.join(", ")));
    }
    out.push('\n');

    if let Some(epic) = &b.epic {
        out.push_str("## Parent Epic\n\n");
        out.push_str(&format!("- slug: @{}\n", epic.slug));
        out.push_str(&format!("- title: {}\n", epic.title));
        out.push_str(&format!("- status: {}\n", epic.status));
        if let Some(goal) = &epic.goal {
            out.push_str("\n### Goal\n\n");
            out.push_str(goal);
            out.push('\n');
        }
        if let Some(scope) = &epic.scope {
            out.push_str("\n### Scope\n\n");
            out.push_str(scope);
            out.push('\n');
        }
        out.push('\n');
    } else if let Some(s) = &b.issue.epic {
        out.push_str(&format!(
            "## Parent Epic\n\n- slug: @{s} (not found)\n\n"
        ));
    }

    if !b.related_issues.is_empty() {
        out.push_str("## Related\n\n");
        for r in &b.related_issues {
            out.push_str(&format_related_line(r));
        }
        out.push('\n');
    }

    if !b.blocking_issues.is_empty() {
        out.push_str("## Blocked By\n\n");
        for r in &b.blocking_issues {
            out.push_str(&format_related_line(r));
        }
        out.push('\n');
    }

    for (name, text) in &b.sections {
        out.push_str(&format!("## {name}\n\n{text}\n\n"));
    }

    if !b.issue.commits.is_empty() {
        out.push_str("## Commits\n\n");
        for c in &b.issue.commits {
            out.push_str(&format!("- {}: {}\n", c.hash, c.summary));
        }
        out.push('\n');
    }

    out.push_str("## Schema\n\n");
    out.push_str(&format!("- version: {}\n", b.schema.version));
    for f in &b.schema.fields {
        let mut line = format!("- {}", f.name);
        let mut bits = Vec::new();
        if f.required {
            bits.push("required".to_string());
        }
        if f.list {
            bits.push("list".to_string());
        }
        if let Some(values) = &f.allowed {
            bits.push(format!("enum=[{}]", values.join(", ")));
        }
        if !bits.is_empty() {
            line.push_str(&format!(" ({})", bits.join(", ")));
        }
        line.push('\n');
        out.push_str(&line);
    }
    out.push('\n');

    out.push_str("## Body\n\n");
    out.push_str(b.issue.body.trim_end());
    out.push('\n');
    out
}

fn format_related_line(r: &RelatedRef) -> String {
    match (&r.title, &r.status, &r.issue_type) {
        (Some(t), Some(st), Some(ty)) => {
            format!("- @{} — {t} ({ty}, {st})\n", r.slug)
        }
        _ => format!("- @{} (not found)\n", r.slug),
    }
}

pub fn render_json(b: &Bundle) -> Result<String> {
    Ok(serde_json::to_string_pretty(b)? + "\n")
}

// ── Cache writes ──────────────────────────────────────────────────────

const CACHE_RELATIVE: &str = ".issuectl/cache/agent";

pub fn cache_path(root: &Path, slug: &str, file: &str) -> PathBuf {
    root.join(CACHE_RELATIVE).join(slug).join(file)
}

pub fn write_artifact(root: &Path, slug: &str, file: &str, content: &str) -> Result<PathBuf> {
    let path = cache_path(root, slug, file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    fs::write(&path, content)
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}

// ── Prompt template substitution ──────────────────────────────────────

const PROMPT_DIR: &str = ".issuectl/prompts";

pub fn prompt_template_path(root: &Path, name: &str) -> PathBuf {
    let filename = if name.ends_with(".md") {
        name.to_string()
    } else {
        format!("{name}.md")
    };
    root.join(PROMPT_DIR).join(filename)
}

/// Render a template by substituting `{{key}}` placeholders against the
/// bundle. Unknown keys are left intact (so typos surface in the
/// rendered output). Whitespace inside the braces is tolerated:
/// `{{ key }}` and `{{key}}` are equivalent.
pub fn render_prompt(template: &str, bundle: &Bundle) -> String {
    let vars = template_vars(bundle);
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        if let Some(close) = after.find("}}") {
            let key = after[..close].trim();
            if let Some(val) = vars.get(key) {
                out.push_str(val);
                rest = &after[close + 2..];
                continue;
            }
        }
        // Unknown placeholder or unterminated braces — emit literal `{{`
        // and resume scanning right after.
        out.push_str("{{");
        rest = after;
    }
    out.push_str(rest);
    out
}

fn template_vars(b: &Bundle) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("slug".into(), b.issue.slug.clone());
    m.insert("title".into(), b.issue.title.clone());
    m.insert("type".into(), b.issue.issue_type.clone());
    m.insert("status".into(), b.issue.status.clone());
    m.insert("priority".into(), b.issue.priority.clone());
    m.insert("body".into(), b.issue.body.clone());
    m.insert("labels".into(), b.issue.labels.join(", "));
    m.insert(
        "related".into(),
        b.issue
            .related
            .iter()
            .map(|s| format!("@{s}"))
            .collect::<Vec<_>>()
            .join(", "),
    );
    m.insert(
        "blocked_by".into(),
        b.issue
            .blocked_by
            .iter()
            .map(|s| format!("@{s}"))
            .collect::<Vec<_>>()
            .join(", "),
    );
    m.insert(
        "commits".into(),
        b.issue
            .commits
            .iter()
            .map(|c| format!("{}: {}", c.hash, c.summary))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    m.insert(
        "epic_slug".into(),
        b.epic
            .as_ref()
            .map(|e| e.slug.clone())
            .unwrap_or_default(),
    );
    m.insert(
        "epic_title".into(),
        b.epic
            .as_ref()
            .map(|e| e.title.clone())
            .unwrap_or_default(),
    );
    m.insert(
        "epic_goal".into(),
        b.epic
            .as_ref()
            .and_then(|e| e.goal.clone())
            .unwrap_or_default(),
    );
    m.insert(
        "epic_scope".into(),
        b.epic
            .as_ref()
            .and_then(|e| e.scope.clone())
            .unwrap_or_default(),
    );
    for name in ACCEPTANCE_SECTIONS {
        let key = section_var_key(name);
        m.insert(
            key,
            b.sections.get(*name).cloned().unwrap_or_default(),
        );
    }
    // `context` exposes the full markdown bundle so a template can simply
    // reference {{context}} and append its own framing prose.
    m.insert("context".into(), render_markdown(b));
    m
}

fn section_var_key(name: &str) -> String {
    name.to_ascii_lowercase().replace(' ', "_")
}

pub fn load_template(root: &Path, name: &str) -> Result<String> {
    let path = prompt_template_path(root, name);
    if !path.is_file() {
        bail!(
            "prompt template {} not found (looked at {})",
            name,
            path.display()
        );
    }
    fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fresh_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        tmp
    }

    fn write_issue(root: &Path, slug: &str, fm: &str, body: &str) {
        let dir = root.join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), format!("---\n{fm}---\n{body}")).unwrap();
    }

    #[test]
    fn build_includes_issue_and_schema() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\npriority: high\n",
            "\n# Login deadlock\n\n## Acceptance Criteria\n\n- it works\n- no deadlock\n\n## Body text below.\n",
        );
        let b = build(tmp.path(), "amber-loud-fox").unwrap();
        assert_eq!(b.issue.slug, "amber-loud-fox");
        assert_eq!(b.issue.title, "Login deadlock");
        assert_eq!(b.issue.status, "open");
        assert!(b.epic.is_none());
        assert!(b.sections.contains_key("Acceptance Criteria"));
        assert!(b.schema.fields.iter().any(|f| f.name == "type"));
    }

    #[test]
    fn build_resolves_parent_epic_and_related() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "broad-shiny-epic",
            "type: epic\nstatus: in-progress\nowner: cara\n",
            "\n# Refactor auth\n\n## Goal\n\nReplace cookie session.\n\n## Scope\n\n- auth middleware\n",
        );
        write_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\nepic: broad-shiny-epic\nrelated: [\"@calm-bright-newt\"]\n",
            "\n# Login deadlock\n",
        );
        write_issue(
            tmp.path(),
            "calm-bright-newt",
            "type: feature\nstatus: open\n",
            "\n# Other thing\n",
        );
        let b = build(tmp.path(), "amber-loud-fox").unwrap();
        let epic = b.epic.expect("parent epic resolved");
        assert_eq!(epic.slug, "broad-shiny-epic");
        assert_eq!(epic.goal.as_deref(), Some("Replace cookie session."));
        assert!(epic.scope.is_some());
        assert_eq!(b.related_issues.len(), 1);
        assert_eq!(b.related_issues[0].slug, "calm-bright-newt");
        assert_eq!(b.related_issues[0].title.as_deref(), Some("Other thing"));
    }

    #[test]
    fn build_handles_missing_epic_gracefully() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\nepic: gone-missing-here\n",
            "\n# x\n",
        );
        let b = build(tmp.path(), "amber-loud-fox").unwrap();
        assert!(b.epic.is_none());
        // Render must mention the dangling reference rather than crashing.
        let md = render_markdown(&b);
        assert!(md.contains("gone-missing-here"));
    }

    #[test]
    fn build_picks_up_blocked_by_from_raw_frontmatter() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "blocker-quiet-newt",
            "type: task\nstatus: open\n",
            "\n# Blocker\n",
        );
        write_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\nblocked_by: [\"@blocker-quiet-newt\"]\n",
            "\n# x\n",
        );
        let b = build(tmp.path(), "amber-loud-fox").unwrap();
        assert_eq!(b.issue.blocked_by, vec!["blocker-quiet-newt".to_string()]);
        assert_eq!(b.blocking_issues.len(), 1);
        assert_eq!(b.blocking_issues[0].title.as_deref(), Some("Blocker"));
    }

    #[test]
    fn render_markdown_is_deterministic() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\nlabels: [b, a]\n",
            "\n# X\n\n## Acceptance Criteria\n\n- one\n",
        );
        let a = render_markdown(&build(tmp.path(), "amber-loud-fox").unwrap());
        let b = render_markdown(&build(tmp.path(), "amber-loud-fox").unwrap());
        assert_eq!(a, b, "repeated builds must produce identical markdown");
    }

    #[test]
    fn render_json_is_deterministic_and_stable_shape() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\n",
            "\n# X\n",
        );
        let a = render_json(&build(tmp.path(), "amber-loud-fox").unwrap()).unwrap();
        let b = render_json(&build(tmp.path(), "amber-loud-fox").unwrap()).unwrap();
        assert_eq!(a, b);
        let v: serde_json::Value = serde_json::from_str(&a).unwrap();
        assert!(v.get("issue").is_some());
        assert!(v.get("schema").is_some());
        assert!(v.get("related_issues").is_some());
    }

    #[test]
    fn render_prompt_substitutes_known_keys_and_keeps_unknown() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\n",
            "\n# Title here\n",
        );
        let bundle = build(tmp.path(), "amber-loud-fox").unwrap();
        let tpl = "Slug: {{slug}}; Title: {{ title }}; Unknown: {{nope}}";
        let out = render_prompt(tpl, &bundle);
        assert!(out.contains("Slug: amber-loud-fox"));
        assert!(out.contains("Title: Title here"));
        assert!(out.contains("{{nope}}"));
    }

    #[test]
    fn write_artifact_lands_under_cache_path() {
        let tmp = fresh_repo();
        write_issue(
            tmp.path(),
            "amber-loud-fox",
            "type: bug\nstatus: open\n",
            "\n# X\n",
        );
        let bundle = build(tmp.path(), "amber-loud-fox").unwrap();
        let p = write_artifact(
            tmp.path(),
            "amber-loud-fox",
            "context.md",
            &render_markdown(&bundle),
        )
        .unwrap();
        assert!(p.starts_with(tmp.path().join(".issuectl/cache/agent/amber-loud-fox")));
        assert!(p.is_file());
    }

    #[test]
    fn build_errors_on_missing_slug() {
        let tmp = fresh_repo();
        let err = build(tmp.path(), "no-such-slug").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }
}
