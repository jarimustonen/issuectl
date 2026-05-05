use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub hash: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    /// Canonical identifier — derived from directory name.
    pub slug: String,
    pub folder: String,

    // Core frontmatter
    pub created: Option<String>,
    pub status: String,

    // Optional frontmatter
    pub updated: Option<String>,
    pub priority: String,
    #[serde(rename = "type")]
    pub issue_type: String,

    // People
    pub reporter: Option<String>,
    pub assignee: Option<String>,
    pub owner: Option<String>,

    // Relationships — references are slugs.
    pub epic: Option<String>,
    pub related: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,

    // Lifecycle
    pub closed: Option<String>,
    pub commits: Option<Vec<Commit>>,

    // Derived from markdown body
    pub title: String,
    pub body: String,
}

impl Issue {
    pub fn effective_assignee(&self) -> &str {
        self.assignee
            .as_deref()
            .or(self.owner.as_deref())
            .unwrap_or("")
    }
}
