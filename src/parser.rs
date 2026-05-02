use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
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
    pub epic: Option<u32>,
    pub related: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,
    pub closed: Option<String>,
    pub commits: Option<Vec<super::models::Commit>>,
}

pub fn parse_issue_dir(dirname: &str) -> Option<(u32, String)> {
    let hyphen = dirname.find('-')?;
    let num_part = &dirname[..hyphen];
    let number: u32 = num_part.parse().ok()?;
    let slug = dirname[hyphen + 1..].to_string();
    Some((number, slug))
}

pub fn parse_item_md(path: &Path, number: u32, slug: &str, folder: &str) -> crate::models::Issue {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Warning: cannot read {}: {}", path.display(), e);
            return crate::models::Issue {
                number,
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
            };
        }
    };

    let (frontmatter, body) = split_frontmatter(&text);
    let fm: Frontmatter = frontmatter
        .and_then(|yaml_text| serde_yaml::from_str(yaml_text).ok())
        .unwrap_or_default();

    let title = extract_title(body);

    crate::models::Issue {
        number,
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
    }
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

impl Default for Frontmatter {
    fn default() -> Self {
        Self {
            created: None,
            updated: None,
            issue_type: None,
            reporter: None,
            assignee: None,
            owner: None,
            status: None,
            priority: None,
            epic: None,
            related: None,
            labels: None,
            closed: None,
            commits: None,
        }
    }
}
