use std::path::Path;

use serde::Deserialize;

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
    pub epic: Option<String>,
    pub related: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,
    pub closed: Option<String>,
    pub commits: Option<Vec<super::models::Commit>>,
    /// Slug stored in frontmatter; mirrors the directory name. Authoritative
    /// identifier is still the directory name; this is just informational.
    #[allow(dead_code)]
    pub slug: Option<String>,
}

/// Issue together with any non-fatal parse warnings (unreadable file,
/// malformed YAML). Callers that want strict behavior — e.g., the web API
/// surfacing per-issue parse errors — should consult `warnings`; the CLI
/// continues to print to stderr and use the lossy `parse_item_md` wrapper.
pub struct ParsedItem {
    pub issue: crate::models::Issue,
    pub warnings: Vec<String>,
}

pub fn parse_item_md_with_warnings(path: &Path, slug: &str, folder: &str) -> ParsedItem {
    let mut warnings = Vec::new();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            warnings.push(format!("cannot read {}: {}", path.display(), e));
            return ParsedItem {
                issue: default_issue(slug, folder),
                warnings,
            };
        }
    };

    let (frontmatter, body) = split_frontmatter(&text);
    let fm = match frontmatter {
        Some(yaml_text) => match serde_yaml::from_str::<Frontmatter>(yaml_text) {
            Ok(fm) => fm,
            Err(e) => {
                warnings.push(format!("invalid YAML frontmatter in {}: {}", path.display(), e));
                Frontmatter::default()
            }
        },
        None => {
            warnings.push(format!("missing YAML frontmatter in {}", path.display()));
            Frontmatter::default()
        }
    };

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
        title,
        body: body.unwrap_or_default().trim().to_string(),
    };
    ParsedItem { issue, warnings }
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

fn split_frontmatter(text: &str) -> (Option<&str>, Option<&str>) {
    let text = text.trim_start();
    if !text.starts_with("---") {
        return (None, Some(text));
    }
    let rest = &text[3..];
    if let Some(end) = rest.find("\n---") {
        let yaml_text = &rest[..end];
        let body = &rest[end + 4..];
        (Some(yaml_text), Some(body))
    } else {
        (None, Some(text))
    }
}

fn extract_title(body: Option<&str>) -> String {
    let body = match body {
        Some(b) => b,
        None => return String::new(),
    };
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return rest.trim().to_string();
        }
    }
    String::new()
}
