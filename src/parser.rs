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
        None => Frontmatter::default(),
    };

    // Surface legacy numeric epic refs as a warning instead of an
    // unconditional stderr print — the doctor --fix pass migrates these
    // and the web UI flags them inline.
    if let Some(ref e) = fm.epic {
        if !e.is_empty() && e.chars().all(|c| c.is_ascii_digit()) {
            warnings.push(format!(
                "{}: epic: {} is a legacy numeric ref — run `issuectl doctor --fix`",
                path.display(),
                e
            ));
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
            return strip_legacy_title_number(rest).trim().to_string();
        }
    }
    String::new()
}

fn strip_legacy_title_number(title: &str) -> &str {
    let title = title.strip_prefix('E').unwrap_or(title);
    let Some((number, rest)) = title.split_once(". ") else {
        return title;
    };
    if number.chars().all(|ch| ch.is_ascii_digit()) {
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
