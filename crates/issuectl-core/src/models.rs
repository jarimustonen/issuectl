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
    /// Closer attribution, managed in lockstep with `closed:` (set on
    /// close, scrubbed on reopen). A first-class typed field mirroring
    /// `closed:` — the parser migrates a legacy `extra["closed_by"]`
    /// into it on read (see `parser`), it is folded into `canonical_hash`
    /// under the `closed_by` key, and reserved from `set`/`update --field`
    /// so the only writer is the validated close-lifecycle slot.
    pub closed_by: Option<String>,
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

    /// Canonical, deduplicated, sorted slug list for this issue's
    /// `blocked_by:` frontmatter array. Reads from `extra` because
    /// `blocked_by` is intentionally NOT a typed `Frontmatter` field
    /// (see `parser::Frontmatter::unknown` doc) — promoting it would
    /// let serde consume the key before `extra` is built and silently
    /// drop it from query/context bundles. Accepts a list of strings
    /// or a single string for hand-edited tolerance; any other shape
    /// yields an empty list. The reverse `blocks` relationship is
    /// derived at runtime by scanning every issue's `blocked_by`
    /// across the repo (see `crate::refs::blocked_by_graph`).
    pub fn blocked_by(&self) -> Vec<String> {
        use serde_json::Value;
        let raw: Vec<String> = match self.extra.get("blocked_by") {
            Some(Value::Array(seq)) => seq
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect(),
            Some(Value::String(s)) => vec![s.clone()],
            _ => Vec::new(),
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for r in raw {
            let t = r.trim();
            if t.is_empty() {
                continue;
            }
            let candidate = t.strip_prefix('@').unwrap_or(t);
            if crate::slug::is_valid(candidate) && seen.insert(candidate.to_string()) {
                out.push(candidate.to_string());
            }
        }
        out.sort();
        out
    }
}
