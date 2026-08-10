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
    /// `closed:` — the parser lifts the string value out of the raw
    /// `closed_by:` frontmatter key into this slot on read (see
    /// `parser`; a non-string legacy value stays in `extra`). It is
    /// folded into `canonical_hash` under the `closed_by` key, and
    /// reserved from `set`/`update --field` so the only writer is the
    /// validated close-lifecycle slot.
    pub closed_by: Option<String>,
    pub commits: Option<Vec<Commit>>,

    // Scheduling DAG (see `crate::dag`). Both optional and
    // absent-by-default so an issue that sets neither hashes identically
    // to the pre-scheduling-fields shape. Lifted from the raw
    // frontmatter mapping by the parser exactly like `closed_by`: a
    // string `lane:` and a list-of-strings `collision:` are promoted
    // into these typed slots (and removed from `extra`); a malformed
    // shape stays in `extra`, readable and hashed as-is. Reserved from
    // `set`/`update --field`; the only writers are the dedicated
    // `update --lane` / `--add-collision` slots.
    /// Scheduling group ("hot-file family"). At most one lane per issue;
    /// a lane is a spawn-time mutual-exclusion group (one runs at a time).
    pub lane: Option<String>,
    /// Extra hot-file tokens beyond the lane that force spawn-time
    /// exclusion — two issues sharing a collision token cannot run
    /// concurrently even across lanes.
    pub collision: Option<Vec<String>>,
    /// Coarse intra-lane precedence key (see `crate::dag`). Consulted
    /// after the `blocked_by` topological order but before the slug
    /// lexical tie-break, so a human can pin "do this lane member before
    /// that one" without fabricating a dependency edge. Absent → today's
    /// behaviour (priority, then created, then slug). Lifted from the raw
    /// mapping by the parser exactly like `lane` — an *integer* `lane_seq:`
    /// is promoted into this typed slot; any other shape stays in `extra`.
    /// Projected into `canonical_hash` only when `Some` (so an issue that
    /// never sets it hashes identically). Reserved from `set`/`update
    /// --field`; the only writer is `update --lane-seq`.
    pub lane_seq: Option<i64>,

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
