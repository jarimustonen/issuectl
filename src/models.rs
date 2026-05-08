use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

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

    /// Frontmatter keys outside the schema above. Stored as
    /// `serde_json::Value` so the type is JSON-safe by construction
    /// — the parser converts YAML → JSON at the boundary, emitting a
    /// `LoadWarning` for shapes JSON cannot represent (non-string
    /// mapping keys, YAML tags). That keeps `canonical_hash` and the
    /// API serializer panic-free regardless of what users put in
    /// frontmatter. Round-trip preservation rides on the raw
    /// `serde_yaml::Mapping` in `write::ItemFile`, so unknowns are
    /// preserved on disk byte-for-byte; this field exists only to
    /// project them into the version hash and surface them on the
    /// wire. Hidden from JSON when empty so issues without unknowns
    /// keep the pre-PR API shape.
    ///
    /// **Known limitation:** any conversion failure (a single
    /// non-string nested key, a YAML tag) trips
    /// `MutateError::Corrupt` and refuses *all* writes to the
    /// issue, not just writes that touch the offending key. The
    /// user has to hand-edit the file to repair it. This is the
    /// conservative default — a partial `extra` would let the hash
    /// lie about file contents — but it does mean the web/API has
    /// no path to fix a typo'd custom key. Tracked for revisit if
    /// users hit it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, JsonValue>,
}

impl Issue {
    pub fn effective_assignee(&self) -> &str {
        self.assignee
            .as_deref()
            .or(self.owner.as_deref())
            .unwrap_or("")
    }
}
