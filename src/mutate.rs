//! Mutation contract shared by the CLI and the web server.
//!
//! Implements the M1 protocol from `docs/design/web-edit-sync.md` §3:
//! repo-wide `flock` → locate → read → optimistic-version check →
//! mutate in memory → atomic write → recompute canonical hash →
//! publish before releasing the lock. The lock guard is RAII so panic
//! / cancellation during the sequence drops the file and releases the
//! advisory lock.
//!
//! Post-flat-layout (issue `awfully-faint-sound`): status changes are
//! pure frontmatter PATCHes — there is no `open/` ↔ `closed/`
//! directory rename. If the slug is found at a legacy path, the
//! mutation moves it to flat layout in-line under the same flock.
//!
//! The CLI (`do_update`, `do_close`, `do_new`) and the axum PATCH/POST
//! handlers both call into this module so a) every writer obtains the
//! same `flock`, b) every writer emits the same canonical version
//! token, and c) the web server never has to fork a second process to
//! mutate state.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Deserializer};

use crate::canonical::canonical_hash;
use crate::models::Issue;
use crate::repo::{self, folder_for_status, IssueSummary};
use crate::server::events::{EventHub, EventPayload};
use crate::write::{self, ItemFile};

/// Three-state field patch from the design's §3.5.
///
/// `Unspecified` means the caller did not mention the field at all
/// (leave it alone). `Clear` means the caller asked to remove the
/// field's value. `Set(T)` means the caller asked to write a new
/// value. Maps directly onto JSON's `field-absent` / `null` /
/// `concrete-value` and onto clap's `omitted` / `--no-X` / `--X v`.
#[derive(Debug, Clone)]
pub enum Patch<T> {
    Unspecified,
    Clear,
    Set(T),
}

impl<T> Default for Patch<T> {
    fn default() -> Self {
        Patch::Unspecified
    }
}

// `#[serde(default)]` on each field plus this Deserialize impl gives
// us the three-state mapping: absent → Unspecified, null → Clear,
// concrete value → Set(T). Empty-string Set is rejected as a
// validation error in `validate()` rather than here so the surface
// for the rejection is closer to the user.
impl<'de, T> Deserialize<'de> for Patch<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        match v {
            serde_json::Value::Null => Ok(Patch::Clear),
            other => {
                let inner = T::deserialize(other).map_err(serde::de::Error::custom)?;
                Ok(Patch::Set(inner))
            }
        }
    }
}

/// Per-field PATCH request. Mirrors `clap`'s `Update` flags 1:1 so
/// `cmd_update` can produce the same shape from the command line.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateIssueRequest {
    #[serde(default)]
    pub expected_version: Option<String>,
    #[serde(default)]
    pub status: Patch<String>,
    #[serde(default)]
    pub priority: Patch<String>,
    #[serde(default)]
    pub assignee: Patch<String>,
    #[serde(default)]
    pub owner: Patch<String>,
    #[serde(default)]
    pub epic: Patch<String>,
    #[serde(default)]
    pub add_labels: Vec<String>,
    #[serde(default)]
    pub remove_labels: Vec<String>,
    #[serde(default)]
    pub add_related: Vec<String>,
    #[serde(default)]
    pub remove_related: Vec<String>,
    #[serde(default)]
    pub add_commits: Vec<CommitSpec>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct CommitSpec {
    pub hash: String,
    pub summary: String,
}

impl UpdateIssueRequest {
    /// True when no field would actually change on disk — every patch
    /// slot is `Unspecified` and every list/commit collection is empty.
    /// `expected_version` is *not* a mutation; an empty body with only a
    /// version token is still a no-op (M13).
    pub fn is_noop(&self) -> bool {
        matches!(self.status, Patch::Unspecified)
            && matches!(self.priority, Patch::Unspecified)
            && matches!(self.assignee, Patch::Unspecified)
            && matches!(self.owner, Patch::Unspecified)
            && matches!(self.epic, Patch::Unspecified)
            && self.add_labels.is_empty()
            && self.remove_labels.is_empty()
            && self.add_related.is_empty()
            && self.remove_related.is_empty()
            && self.add_commits.is_empty()
    }

    /// Reject empty-string Sets, type-set vs enum mismatches, and
    /// add_X/remove_X intent collisions. Runs once after both serde
    /// and clap have produced the request.
    pub fn validate(&self) -> Result<(), MutateError> {
        check_set_nonempty("status", &self.status)?;
        check_set_nonempty("priority", &self.priority)?;
        check_set_nonempty("assignee", &self.assignee)?;
        check_set_nonempty("owner", &self.owner)?;
        check_set_nonempty("epic", &self.epic)?;

        if let Patch::Set(s) = &self.status {
            if !crate::all_statuses().iter().any(|v| v == s) {
                return Err(MutateError::Validation(format!(
                    "status {s:?} is not one of the known statuses"
                )));
            }
        }
        if let Patch::Set(p) = &self.priority {
            if !crate::PRIORITIES.iter().any(|v| v == p) {
                return Err(MutateError::Validation(format!(
                    "priority {p:?} is not one of the known priorities"
                )));
            }
        }

        for (name, list) in [
            ("add_labels", &self.add_labels),
            ("remove_labels", &self.remove_labels),
            ("add_related", &self.add_related),
            ("remove_related", &self.remove_related),
        ] {
            if list.iter().any(|s| s.is_empty()) {
                return Err(MutateError::Validation(format!(
                    "{name} contains an empty string element"
                )));
            }
        }
        if let Some(dup) = first_duplicate(&self.add_labels) {
            return Err(MutateError::Validation(format!(
                "add_labels contains duplicate {dup:?}"
            )));
        }
        if let Some(dup) = first_duplicate(&self.remove_labels) {
            return Err(MutateError::Validation(format!(
                "remove_labels contains duplicate {dup:?}"
            )));
        }
        if let Some(overlap) = first_overlap(&self.add_labels, &self.remove_labels) {
            return Err(MutateError::ConflictingIntent(format!(
                "label {overlap:?} appears in both add_labels and remove_labels"
            )));
        }
        if let Some(overlap) = first_overlap(&self.add_related, &self.remove_related) {
            return Err(MutateError::ConflictingIntent(format!(
                "related ref {overlap:?} appears in both add_related and remove_related"
            )));
        }
        Ok(())
    }
}

fn check_set_nonempty(field: &str, p: &Patch<String>) -> Result<(), MutateError> {
    if let Patch::Set(v) = p {
        if v.is_empty() {
            return Err(MutateError::Validation(format!(
                "{field}: empty-string Set is not allowed (use null to clear)"
            )));
        }
    }
    Ok(())
}

fn first_duplicate(xs: &[String]) -> Option<&String> {
    let mut seen = std::collections::HashSet::new();
    for x in xs {
        if !seen.insert(x.as_str()) {
            return Some(x);
        }
    }
    None
}

fn first_overlap<'a>(a: &'a [String], b: &[String]) -> Option<&'a String> {
    let bs: std::collections::HashSet<&str> = b.iter().map(String::as_str).collect();
    a.iter().find(|x| bs.contains(x.as_str()))
}

/// Successful mutation result. Used both by the CLI (for `--json`
/// output) and by the server (to build the PATCH 200 response).
#[derive(Debug)]
pub struct UpdateOutcome {
    pub issue: Issue,
    pub version: String,
    /// Directory containing the issue's `item.md` after the mutation.
    /// Stable across status transitions in the flat layout.
    pub issue_dir: PathBuf,
    /// True if this mutation transitioned the status from active to
    /// closing. No directory move happens — the booleans are kept for
    /// CLI/web messaging parity.
    pub moved_to_closed: bool,
    pub moved_to_open: bool,
}

/// Errors that the mutate layer surfaces to its callers. The CLI maps
/// these to anyhow + exit codes; the server maps them to HTTP statuses
/// in `api.rs`.
#[derive(Debug)]
pub enum MutateError {
    NotFound,
    /// Slug present at multiple paths simultaneously (flat + legacy, or
    /// both legacy folders). Carries the offending paths so the API
    /// response and CLI message can tell the user where to look — the
    /// blanket pre-flat-layout "open/ and closed/" message no longer
    /// covers the new ambiguity classes.
    AmbiguousSlug {
        paths: Vec<PathBuf>,
    },
    /// `expected_version` did not match the current canonical hash.
    /// Carries the current full issue plus its version so the response
    /// can include them per §4.3.
    VersionMismatch {
        current: Issue,
        version: String,
    },
    /// On-disk content has parser warnings (malformed YAML, missing
    /// fields, merge markers). We refuse to mutate a corrupt file
    /// rather than silently overwriting recovered defaults — the user
    /// must fix the source before mutating it (§8.6).
    Corrupt {
        warnings: Vec<String>,
    },
    Validation(String),
    ConflictingIntent(String),
    Io(anyhow::Error),
}

impl std::fmt::Display for MutateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MutateError::NotFound => write!(f, "issue not found"),
            MutateError::AmbiguousSlug { paths } => {
                write!(
                    f,
                    "slug present at multiple paths — resolve manually: {}",
                    paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            MutateError::VersionMismatch { version, .. } => {
                write!(f, "version mismatch (current: {version})")
            }
            MutateError::Corrupt { warnings } => {
                write!(f, "corrupt issue: {}", warnings.join("; "))
            }
            MutateError::Validation(s) => write!(f, "validation: {s}"),
            MutateError::ConflictingIntent(s) => write!(f, "conflicting intent: {s}"),
            MutateError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for MutateError {}

impl From<anyhow::Error> for MutateError {
    fn from(e: anyhow::Error) -> Self {
        MutateError::Io(e)
    }
}

/// RAII guard for the repo-wide write lock. Created with
/// `acquire(root)` and released on `Drop` (panic-safe).
pub struct WriteLock {
    _file: File,
}

impl WriteLock {
    pub fn acquire(root: &Path) -> Result<Self> {
        let dir = root.join(".issuectl");
        fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
        let path = dir.join("write.lock");
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create(true).truncate(false);
        // Explicit 0o600: `create(true)` alone honours umask, which can
        // make the lock world-readable on permissive umasks.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let f = opts
            .open(&path)
            .with_context(|| format!("cannot open lock file {}", path.display()))?;
        // `OpenOptions::mode(0o600)` only fires on creation. If the
        // file already exists with looser permissions (older issuectl,
        // permissive umask, fork), reopen does not retighten — so we
        // unconditionally `set_permissions` after open.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .with_context(|| format!("cannot chmod {}", path.display()))?;
        }
        FileExt::lock_exclusive(&f)
            .with_context(|| format!("cannot acquire flock on {}", path.display()))?;
        Ok(WriteLock { _file: f })
    }
}

// fs2 releases the advisory lock when the file handle is closed (Drop).

// ── Public entry points ─────────────────────────────────────────────────

/// PATCH-style update of an existing issue. The slug comes from the
/// caller (URL path or CLI arg); the request body carries the rest.
/// `hub` is `Some` for server callers (so the SSE clients see a
/// synthetic `IssueUpserted` published before flock release) and
/// `None` for CLI callers (the watcher will pick up the change).
pub fn update_issue(
    root: &Path,
    slug: &str,
    req: UpdateIssueRequest,
    hub: Option<&Arc<EventHub>>,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }

    // Normalize related-ref shapes BEFORE validate() so a typo'd ref
    // like `add_related: ["123"]` + `remove_related: ["#123"]`
    // (which both normalize to `#123`) is caught by the overlap check.
    let normalized_add_related = crate::normalize_related_refs_pub(&req.add_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let normalized_remove_related = crate::normalize_related_refs_pub(&req.remove_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let mut req_normalized = req;
    req_normalized.add_related = normalized_add_related.clone();
    req_normalized.remove_related = normalized_remove_related.clone();
    let req = req_normalized;

    req.validate()?;

    // M13: an empty PATCH (all `Unspecified`, no list/commit changes)
    // is a no-op — return the current state without touching the file.
    // Without this short-circuit, an "empty" call would still bump
    // `updated:` and (surprisingly) trigger an in-line legacy→flat
    // migration. Read-only locate + parse, no write, no publish.
    if req.is_noop() {
        let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
        let located = repo::locate_issue_full(root, slug).map_err(|_| MutateError::NotFound)?;
        let parsed = crate::parser::parse_item_md_with_warnings(&located.item_path, slug, "open");
        if !parsed.warnings.is_empty() {
            return Err(MutateError::Corrupt {
                warnings: parsed.warnings,
            });
        }
        let mut issue = parsed.issue;
        issue.folder = folder_for_status(&issue.status).to_string();
        let version = canonical_hash(&issue);
        if let Some(ref expected) = req.expected_version {
            if expected != &version {
                return Err(MutateError::VersionMismatch {
                    current: issue,
                    version,
                });
            }
        }
        return Ok(UpdateOutcome {
            issue_dir: located
                .item_path
                .parent()
                .expect("item.md has parent")
                .to_path_buf(),
            issue,
            version,
            moved_to_closed: false,
            moved_to_open: false,
        });
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;

    // 1) locate, then in-line migrate any legacy path under the flock
    //    so writes always land at the canonical flat path.
    let item_path = locate_and_migrate(root, slug)?;
    update_issue_under_lock(slug, item_path, req, hub)
}

/// Body of `update_issue` that runs with the flock already held. Used
/// by `close_issue` to read+decide+mutate atomically without
/// double-acquiring the lock (which deadlocks on Linux because fs2's
/// advisory lock is per-fd).
fn update_issue_under_lock(
    slug: &str,
    item_path: PathBuf,
    req: UpdateIssueRequest,
    hub: Option<&Arc<EventHub>>,
) -> Result<UpdateOutcome, MutateError> {
    let folder = "open"; // placeholder; folder is derived from status post-write

    // 2) read + parse + hash. Refuse to mutate a corrupt file —
    // overwriting parser fallback defaults would silently destroy the
    // user's real (but malformed) on-disk content (§8.6 / M7).
    let parsed = crate::parser::parse_item_md_with_warnings(&item_path, slug, folder);
    if !parsed.warnings.is_empty() {
        return Err(MutateError::Corrupt {
            warnings: parsed.warnings,
        });
    }
    let mut current_issue = parsed.issue;
    current_issue.folder = folder_for_status(&current_issue.status).to_string();
    let current_version = canonical_hash(&current_issue);
    let prev_status = current_issue.status.clone();

    // 3) optimistic concurrency
    if let Some(ref expected) = req.expected_version {
        if expected != &current_version {
            return Err(MutateError::VersionMismatch {
                current: current_issue,
                version: current_version,
            });
        }
    }

    // 4) load the YAML mapping for in-place edits
    let mut item = write::read_item(&item_path).map_err(MutateError::Io)?;
    let mut moved_to_closed = false;
    let mut moved_to_open = false;

    // Status change is a pure frontmatter PATCH; no directory rename
    // (post-flat-layout). The `moved_to_*` booleans now track the
    // active↔closing transition for messaging parity with the old API.
    if let Patch::Set(s) = &req.status {
        write::set_string(&mut item.frontmatter, "status", s);
        let prev_closing = crate::is_closing_status(&prev_status);
        let new_closing = crate::is_closing_status(s);
        if new_closing {
            // Only set `closed:` on the active→closing edge, OR backfill
            // if the field is missing on a closing→closing transition
            // against an issue that pre-dates the auto-stamping.
            // Closing→closing (e.g. fixed→wontfix) MUST preserve the
            // historical close date — overwriting it would silently
            // destroy provenance.
            let has_closed = item
                .frontmatter
                .contains_key(serde_yaml::Value::String("closed".into()));
            if !prev_closing || !has_closed {
                write::set_string(&mut item.frontmatter, "closed", &write::today());
            }
            if !prev_closing {
                moved_to_closed = true;
            }
        } else {
            write::remove_key(&mut item.frontmatter, "closed");
            if prev_closing {
                moved_to_open = true;
            }
        }
    } else if let Patch::Clear = &req.status {
        return Err(MutateError::Validation(
            "status cannot be cleared (issues always have a status)".into(),
        ));
    }

    apply_string_patch(&mut item, "priority", &req.priority);
    apply_string_patch(&mut item, "assignee", &req.assignee);
    apply_string_patch(&mut item, "owner", &req.owner);
    apply_string_patch(&mut item, "epic", &req.epic);

    for label in &req.add_labels {
        write::add_to_string_list(&mut item.frontmatter, "labels", label)
            .map_err(MutateError::Io)?;
    }
    for label in &req.remove_labels {
        write::remove_from_string_list(&mut item.frontmatter, "labels", label)
            .map_err(MutateError::Io)?;
    }

    // related refs were normalized before validate(), so use them as-is.
    for r in &req.add_related {
        write::add_to_string_list(&mut item.frontmatter, "related", r).map_err(MutateError::Io)?;
    }
    for r in &req.remove_related {
        write::remove_from_string_list(&mut item.frontmatter, "related", r)
            .map_err(MutateError::Io)?;
    }

    for spec in &req.add_commits {
        if spec.hash.is_empty() || spec.summary.is_empty() {
            return Err(MutateError::Validation(
                "commit hash and summary must be non-empty".into(),
            ));
        }
        write::add_commit(&mut item.frontmatter, &spec.hash, &spec.summary)
            .map_err(MutateError::Io)?;
    }

    write::set_string(&mut item.frontmatter, "updated", &write::today());

    // 5) atomic write. No directory rename — flat layout means
    //    `item_path` is the canonical location regardless of status.
    write_item_atomic(&item_path, &item).map_err(MutateError::Io)?;
    let final_path = item_path.clone();

    // 6) recompute canonical hash from final on-disk content
    let after = crate::parser::parse_item_md_with_warnings(&final_path, slug, "open");
    let mut new_issue = after.issue;
    new_issue.folder = folder_for_status(&new_issue.status).to_string();
    let new_version = canonical_hash(&new_issue);

    // 7) publish while still inside the lock so seq order matches
    //    disk order.
    if let Some(hub) = hub {
        hub.publish(EventPayload::IssueUpserted {
            slug: slug.to_string(),
            version: new_version.clone(),
            issue: Box::new(IssueSummary::from(new_issue.clone())),
        });
    }

    Ok(UpdateOutcome {
        issue: new_issue,
        version: new_version,
        issue_dir: final_path
            .parent()
            .expect("written file has a parent")
            .to_path_buf(),
        moved_to_closed,
        moved_to_open,
    })
}

/// `issuectl close` semantics: read current type/status, reject if
/// already closing, default the status from the issue type, then apply
/// a status PATCH — all under a single flock so the type read cannot
/// race a concurrent mutation that flips it (M4).
///
/// `status_override` mirrors `--status`. When `None`, the default is
/// `fixed` for `type: bug`, `done` otherwise.
pub fn close_issue(
    root: &Path,
    slug: &str,
    status_override: Option<String>,
    commits: Vec<CommitSpec>,
    expected_version: Option<String>,
    hub: Option<&Arc<EventHub>>,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;

    let item_path = locate_and_migrate(root, slug)?;
    let item = write::read_item(&item_path).map_err(MutateError::Io)?;
    let current_status = item
        .frontmatter
        .get(serde_yaml::Value::String("status".into()))
        .and_then(|v| v.as_str())
        .unwrap_or("open")
        .to_string();
    if crate::is_closing_status(&current_status) {
        return Err(MutateError::Validation(format!(
            "issue {slug} already has a closing status ({current_status}); use `update` to change status"
        )));
    }
    let issue_type = item
        .frontmatter
        .get(serde_yaml::Value::String("type".into()))
        .and_then(|v| v.as_str())
        .unwrap_or("bug")
        .to_string();
    let resolved_status = status_override.unwrap_or_else(|| {
        if issue_type == "bug" {
            "fixed".to_string()
        } else {
            "done".to_string()
        }
    });

    let req = UpdateIssueRequest {
        expected_version,
        status: Patch::Set(resolved_status),
        add_commits: commits,
        ..Default::default()
    };
    // _lock drops at end-of-scope after the locked update path returns.
    // We call the under-lock helper directly so we don't double-acquire
    // (fs2 advisory flock is per-fd; nested `WriteLock::acquire` would
    // deadlock on Linux).
    let mut req_normalized = req;
    let normalized_add_related = crate::normalize_related_refs_pub(&req_normalized.add_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let normalized_remove_related =
        crate::normalize_related_refs_pub(&req_normalized.remove_related)
            .map_err(|e| MutateError::Validation(e.to_string()))?;
    req_normalized.add_related = normalized_add_related;
    req_normalized.remove_related = normalized_remove_related;
    req_normalized.validate()?;
    update_issue_under_lock(slug, item_path, req_normalized, hub)
}

/// PUT-style replacement of an issue's body markdown. Same lock and
/// optimistic-concurrency contract as `update_issue`, but only the body
/// (and `updated:`) change. Status/folder are untouched, so this never
/// causes a directory rename. `hub` follows the same `Some` server /
/// `None` CLI convention as `update_issue`.
pub fn update_body(
    root: &Path,
    slug: &str,
    expected_version: Option<String>,
    body: String,
    hub: Option<&Arc<EventHub>>,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;

    let item_path = locate_and_migrate(root, slug)?;

    let parsed = crate::parser::parse_item_md_with_warnings(&item_path, slug, "open");
    if !parsed.warnings.is_empty() {
        return Err(MutateError::Corrupt {
            warnings: parsed.warnings,
        });
    }
    let mut prev_issue = parsed.issue;
    prev_issue.folder = folder_for_status(&prev_issue.status).to_string();
    let current_version = canonical_hash(&prev_issue);

    if let Some(ref expected) = expected_version {
        if expected != &current_version {
            return Err(MutateError::VersionMismatch {
                current: prev_issue,
                version: current_version,
            });
        }
    }

    let mut item = write::read_item(&item_path).map_err(MutateError::Io)?;
    // Clients send a plain markdown body. Preserve the read_item
    // convention of one leading newline so the on-disk layout stays
    // `---\n<fm>\n---\n\n<body>` rather than `---<body>` — without
    // this, every web save would collapse the blank separator line
    // and parse_item still works but readers see a slightly different
    // file each round-trip.
    item.body = if body.starts_with('\n') {
        body
    } else {
        format!("\n{body}")
    };
    write::set_string(&mut item.frontmatter, "updated", &write::today());

    write_item_atomic(&item_path, &item).map_err(MutateError::Io)?;

    let after = crate::parser::parse_item_md_with_warnings(&item_path, slug, "open");
    let mut new_issue = after.issue;
    new_issue.folder = folder_for_status(&new_issue.status).to_string();
    let new_version = canonical_hash(&new_issue);

    if let Some(hub) = hub {
        hub.publish(EventPayload::IssueUpserted {
            slug: slug.to_string(),
            version: new_version.clone(),
            issue: Box::new(IssueSummary::from(new_issue.clone())),
        });
    }

    Ok(UpdateOutcome {
        issue: new_issue,
        version: new_version,
        issue_dir: item_path
            .parent()
            .expect("item.md has a parent")
            .to_path_buf(),
        moved_to_closed: false,
        moved_to_open: false,
    })
}

/// Apply a `Patch<String>` onto a frontmatter mapping. `Unspecified`
/// is a no-op; `Clear` removes the key; `Set(v)` sets the key.
fn apply_string_patch(item: &mut ItemFile, key: &str, p: &Patch<String>) {
    match p {
        Patch::Unspecified => {}
        Patch::Clear => write::remove_key(&mut item.frontmatter, key),
        Patch::Set(v) => write::set_string(&mut item.frontmatter, key, v),
    }
}

/// Locate the issue and, if it lives at a legacy path, move it to the
/// canonical flat path under the held flock. Returns the final flat
/// `item.md` path.
///
/// Delegates classification to `repo::resolve_layout` so the mutate
/// layer's view of the filesystem matches the loader/watcher/migrate
/// commands — no per-call-site classification logic. After a legacy →
/// flat migration, re-resolves to validate the new flat path picked up
/// by the resolver's symlink/escape hardening (M4 fix).
fn locate_and_migrate(root: &Path, slug: &str) -> Result<PathBuf, MutateError> {
    use repo::LayoutState;
    match repo::resolve_layout(root, slug) {
        LayoutState::Flat { item_path } => Ok(item_path),
        LayoutState::Legacy { .. } => {
            // Migrate, then re-resolve to surface any post-rename
            // anomaly (e.g. a symlink swapped in concurrently).
            repo::migrate_to_flat_inplace(root, slug).map_err(MutateError::Io)?;
            match repo::resolve_layout(root, slug) {
                LayoutState::Flat { item_path } => Ok(item_path),
                LayoutState::Absent => Err(MutateError::NotFound),
                LayoutState::Ambiguous { paths } => Err(MutateError::AmbiguousSlug { paths }),
                LayoutState::Invalid { reason, .. } => Err(MutateError::Io(anyhow!("{reason}"))),
                LayoutState::Legacy { .. } => Err(MutateError::Io(anyhow!(
                    "post-migration state still classifies as legacy"
                ))),
            }
        }
        LayoutState::Ambiguous { paths } => Err(MutateError::AmbiguousSlug { paths }),
        LayoutState::Absent => Err(MutateError::NotFound),
        LayoutState::Invalid { reason, .. } => Err(MutateError::Io(anyhow!("{reason}"))),
    }
}

/// Atomic write: stage as `.issuectl-tmp-…`, fsync, persist into
/// place. On Unix, best-effort fsync the parent directory after
/// rename. The tempfile prefix is the signal the watcher uses to
/// filter our own writes (§5.1).
pub fn write_item_atomic(target: &Path, item: &ItemFile) -> Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| anyhow!("target has no parent: {}", target.display()))?;
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let serialized = write::serialize_item(item)?;
    let mut tf = tempfile::Builder::new()
        .prefix(".issuectl-tmp-")
        .tempfile_in(dir)
        .with_context(|| format!("cannot create tempfile in {}", dir.display()))?;
    use std::io::Write;
    tf.as_file_mut()
        .write_all(serialized.as_bytes())
        .with_context(|| format!("cannot write {}", target.display()))?;
    tf.as_file()
        .sync_all()
        .with_context(|| format!("cannot fsync {}", target.display()))?;
    tf.persist(target)
        .map_err(|e| anyhow!("cannot persist tempfile: {e}"))?;
    #[cfg(unix)]
    {
        if let Err(err) = fsync_dir(dir) {
            eprintln!(
                "issuectl[mutate]: fsync_dir({}) failed: {err}",
                dir.display()
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    let f = File::open(dir)?;
    f.sync_all()
}

// ── Create / close ──────────────────────────────────────────────────────

/// Request body for `POST /api/issues`. Mirrors `cmd_new`'s flag set.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NewIssueRequest {
    #[serde(rename = "type")]
    pub issue_type: String,
    pub title: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub reporter: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub epic: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub related: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_priority() -> String {
    "normal".to_string()
}

#[derive(Debug)]
pub struct NewOutcome {
    pub issue: Issue,
    pub version: String,
    pub issue_dir: PathBuf,
}

pub fn new_issue(
    root: &Path,
    req: NewIssueRequest,
    hub: Option<&Arc<EventHub>>,
) -> Result<NewOutcome, MutateError> {
    if req.title.trim().is_empty() {
        return Err(MutateError::Validation("title cannot be empty".into()));
    }
    if !crate::ISSUE_TYPES.iter().any(|t| t == &req.issue_type) {
        return Err(MutateError::Validation(format!(
            "type {:?} is not one of the known types",
            req.issue_type
        )));
    }
    if !crate::PRIORITIES.iter().any(|p| p == &req.priority) {
        return Err(MutateError::Validation(format!(
            "priority {:?} is not one of the known priorities",
            req.priority
        )));
    }

    // C3: hold the flock through write + parse + publish so seq order
    // matches disk order. The previous implementation called `do_new`,
    // which acquired/released the lock internally — the synthetic
    // `IssueUpserted` then published OUTSIDE the lock, inverting seq
    // against concurrent writers.
    let lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    let outcome = crate::do_new_locked(
        &lock,
        root,
        crate::NewArgs {
            issue_type: req.issue_type,
            title: req.title,
            slug: req.slug,
            reporter: req.reporter,
            assignee: req.assignee,
            owner: req.owner,
            priority: req.priority,
            epic: req.epic,
            labels: req.labels,
            related: req.related,
            source: req.source,
            description: req.description,
        },
    )
    .map_err(|e| {
        let s = e.to_string();
        if s.contains("already") || s.contains("exists") {
            MutateError::ConflictingIntent(s)
        } else {
            MutateError::Validation(s)
        }
    })?;

    // Re-read for canonical hash + Issue. Still holding the lock.
    let parsed =
        crate::parser::parse_item_md_with_warnings(&outcome.item_path, &outcome.slug, "open");
    let mut issue = parsed.issue;
    issue.folder = folder_for_status(&issue.status).to_string();
    let version = canonical_hash(&issue);

    if let Some(hub) = hub {
        hub.publish(EventPayload::IssueUpserted {
            slug: outcome.slug.clone(),
            version: version.clone(),
            issue: Box::new(IssueSummary::from(issue.clone())),
        });
    }

    let result = NewOutcome {
        issue_dir: outcome.item_path.parent().unwrap().to_path_buf(),
        issue,
        version,
    };
    drop(lock);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fresh_repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        tmp
    }

    fn seed_issue(root: &Path, _folder: &str, slug: &str, status: &str) -> String {
        // Flat layout: `_folder` retained for test-call-site compatibility
        // but no longer affects on-disk placement.
        let dir = root.join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            format!(
                "---\ntype: bug\ncreated: 2026-05-06\nstatus: {status}\npriority: normal\n---\n\n# Title\n",
            ),
        )
        .unwrap();
        let parsed = crate::parser::parse_item_md_with_warnings(&dir.join("item.md"), slug, "open");
        let mut issue = parsed.issue;
        issue.folder = crate::repo::folder_for_status(&issue.status).to_string();
        canonical_hash(&issue)
    }

    #[test]
    fn update_with_fresh_version_succeeds() {
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "test-slug-one", "open");
        let req = UpdateIssueRequest {
            expected_version: Some(v0),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "test-slug-one", req, None).unwrap();
        assert!(out.version.starts_with("sha256:"));
        let after = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(after.contains("priority: high"));
    }

    #[test]
    fn update_with_stale_version_returns_409() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "test-slug-two", "open");
        let req = UpdateIssueRequest {
            expected_version: Some("sha256:deadbeef".into()),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "test-slug-two", req, None).unwrap_err();
        match err {
            MutateError::VersionMismatch { current, version } => {
                assert_eq!(current.slug, "test-slug-two");
                assert!(version.starts_with("sha256:"));
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn status_only_patch_leaves_other_fields_untouched() {
        // Drag-and-drop kanban moves PATCH only `status`. Other fields
        // (priority, assignee, epic, …) must round-trip unchanged via
        // `Patch::Unspecified` — without this the web UI would silently
        // clobber metadata on every column move.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/dnd-status-only");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: high\nassignee: alice\nepic: roadmap\n---\n\n# Title\n",
        )
        .unwrap();

        let req = UpdateIssueRequest {
            status: Patch::Set("in-progress".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "dnd-status-only", req, None).unwrap();
        assert_eq!(out.issue.status, "in-progress");
        assert_eq!(out.issue.priority, "high");
        assert_eq!(out.issue.assignee.as_deref(), Some("alice"));
        assert_eq!(out.issue.epic.as_deref(), Some("roadmap"));
        let on_disk = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(on_disk.contains("status: in-progress"));
        assert!(on_disk.contains("priority: high"));
        assert!(on_disk.contains("assignee: alice"));
        assert!(on_disk.contains("epic: roadmap"));
    }

    #[test]
    fn reopening_a_closed_issue_clears_closed_date() {
        // Drag-and-drop allows moving a card out of the closed column
        // back to an active status. The frontmatter `closed:` date must
        // be removed in the same write so the issue isn't left in a
        // contradictory "status: open, closed: 2026-01-01" state.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/reopen-me");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-01\nstatus: fixed\n\
             priority: normal\nclosed: 2026-05-05\n---\n\n# Title\n",
        )
        .unwrap();

        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "reopen-me", req, None).unwrap();
        assert!(out.moved_to_open);
        assert_eq!(out.issue.status, "open");
        assert!(
            out.issue.closed.is_none(),
            "closed: must be cleared on reopen"
        );
        let on_disk = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(on_disk.contains("status: open"));
        assert!(
            !on_disk.contains("closed:"),
            "frontmatter must not retain closed: after reopen, got:\n{on_disk}"
        );
    }

    #[test]
    fn update_status_to_closing_does_not_move_directory() {
        // M14: use inode comparison rather than `created()` (which is
        // Err on most Linux ext4 setups, silently making the assertion
        // a no-op).
        use std::os::unix::fs::MetadataExt;
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "close-me-now", "open");
        let flat_dir = tmp.path().join("issues/close-me-now");
        let before_inode = fs::metadata(&flat_dir).unwrap().ino();
        let req = UpdateIssueRequest {
            status: Patch::Set("fixed".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "close-me-now", req, None).unwrap();
        // Status transition flag still flips, but the dir does not move.
        assert!(out.moved_to_closed);
        assert_eq!(out.issue_dir, flat_dir);
        assert!(flat_dir.is_dir(), "flat dir must still exist");
        assert!(!tmp.path().join("issues/closed/close-me-now").exists());
        assert!(!tmp.path().join("issues/open/close-me-now").exists());
        let after_inode = fs::metadata(&flat_dir).unwrap().ino();
        assert_eq!(
            before_inode, after_inode,
            "directory must not have been recreated"
        );
        let on_disk = fs::read_to_string(flat_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("status: fixed"));
        assert!(on_disk.contains("closed:"));
    }

    #[test]
    fn empty_patch_is_noop_no_legacy_migration() {
        // M13: an empty PATCH against a legacy-path issue must NOT
        // migrate the directory or bump `updated:`. The version returned
        // matches what `show --json` would have read.
        let tmp = fresh_repo();
        let legacy = tmp.path().join("issues/open/empty-patch-legacy");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\nupdated: 2026-01-01\n---\n\n# T\n",
        )
        .unwrap();
        let before = fs::read_to_string(legacy.join("item.md")).unwrap();

        let req = UpdateIssueRequest::default();
        let out = update_issue(tmp.path(), "empty-patch-legacy", req, None).unwrap();

        // Legacy directory is preserved (no migration on a no-op).
        assert!(legacy.is_dir(), "legacy dir must remain untouched");
        let after = fs::read_to_string(legacy.join("item.md")).unwrap();
        assert_eq!(before, after, "no-op must not touch item.md");
        assert!(out.version.starts_with("sha256:"));
    }

    #[test]
    fn closing_to_closing_preserves_closed_date() {
        // C2: fixed → wontfix must preserve the original `closed:` date.
        // Overwriting it silently destroys historical close provenance.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/preserve-closed-date");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: fixed\nclosed: 2026-01-15\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            status: Patch::Set("wontfix".into()),
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "preserve-closed-date", req, None).unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after.contains("closed: 2026-01-15"), "got:\n{after}");
        assert!(after.contains("status: wontfix"));
    }

    #[test]
    fn closing_backfills_closed_date_when_missing() {
        // Closing→closing on an issue that pre-dates auto-stamping should
        // backfill rather than leave the field empty.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/backfill-closed");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: fixed\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            status: Patch::Set("wontfix".into()),
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "backfill-closed", req, None).unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(
            after.contains("closed:"),
            "expected backfilled closed date in:\n{after}"
        );
    }

    #[test]
    fn legacy_path_is_migrated_in_place_on_write() {
        let tmp = fresh_repo();
        let legacy = tmp.path().join("issues/open/legacy-one-here");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let v0 = {
            let parsed = crate::parser::parse_item_md_with_warnings(
                &legacy.join("item.md"),
                "legacy-one-here",
                "open",
            );
            canonical_hash(&parsed.issue)
        };
        let req = UpdateIssueRequest {
            expected_version: Some(v0),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "legacy-one-here", req, None).unwrap();
        assert!(out
            .issue_dir
            .to_string_lossy()
            .ends_with("issues/legacy-one-here"));
        assert!(!legacy.exists(), "legacy dir must be gone after write");
    }

    #[test]
    fn ambiguous_layout_is_rejected() {
        let tmp = fresh_repo();
        // Both flat and legacy versions of the same slug exist.
        let flat = tmp.path().join("issues/dual-path-here");
        fs::create_dir_all(&flat).unwrap();
        fs::write(
            flat.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let legacy = tmp.path().join("issues/open/dual-path-here");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "dual-path-here", req, None).unwrap_err();
        assert!(matches!(err, MutateError::AmbiguousSlug { .. }));
    }

    #[test]
    fn patch_clear_removes_field() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/has-epic-here");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\nepic: foo-bar\n---\n\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            epic: Patch::Clear,
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "has-epic-here", req, None).unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(!after.contains("epic:"));
    }

    #[test]
    fn patch_unspecified_does_not_touch_field() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/keep-epic-as-is");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\nepic: stay-here\n---\n\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "keep-epic-as-is", req, None).unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after.contains("epic: stay-here"));
        assert!(after.contains("priority: high"));
    }

    #[test]
    fn add_and_remove_label_overlap_rejected() {
        let req = UpdateIssueRequest {
            add_labels: vec!["x".into()],
            remove_labels: vec!["x".into()],
            ..Default::default()
        };
        let err = req.validate().unwrap_err();
        assert!(matches!(err, MutateError::ConflictingIntent(_)));
    }

    #[test]
    fn deserialize_unspecified_clear_set() {
        // Field absent → Unspecified, default-derived
        let r: UpdateIssueRequest = serde_json::from_str("{}").unwrap();
        assert!(matches!(r.epic, Patch::Unspecified));

        // null → Clear
        let r: UpdateIssueRequest = serde_json::from_str(r#"{"epic": null}"#).unwrap();
        assert!(matches!(r.epic, Patch::Clear));

        // string → Set
        let r: UpdateIssueRequest = serde_json::from_str(r#"{"epic": "foo"}"#).unwrap();
        assert!(matches!(r.epic, Patch::Set(ref s) if s == "foo"));
    }

    #[test]
    fn deserialize_rejects_unknown_field() {
        let result: Result<UpdateIssueRequest, _> = serde_json::from_str(r#"{"priorty": "high"}"#);
        assert!(result.is_err(), "typo'd field must be rejected");
    }

    #[test]
    fn empty_string_set_rejected() {
        let req = UpdateIssueRequest {
            epic: Patch::Set("".into()),
            ..Default::default()
        };
        assert!(matches!(req.validate(), Err(MutateError::Validation(_))));
    }

    #[test]
    fn update_body_roundtrip_advances_version() {
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "body-roundtrip-x", "open");
        let out = update_body(
            tmp.path(),
            "body-roundtrip-x",
            Some(v0.clone()),
            "# rewrite\n\nnew body".into(),
            None,
        )
        .unwrap();
        assert!(out.version.starts_with("sha256:"));
        assert_ne!(out.version, v0);
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("new body"));
    }

    #[test]
    fn update_body_stale_version_returns_409() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "body-stale-here", "open");
        let err = update_body(
            tmp.path(),
            "body-stale-here",
            Some("sha256:deadbeef".into()),
            "x".into(),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::VersionMismatch { .. }));
    }

    #[test]
    fn write_lock_file_has_strict_permissions() {
        // The flock itself is exercised by every other test in this
        // module (each `update_issue` / `new_issue` call acquires it).
        // Cross-process exclusion would need `std::process::Command`
        // and is out of scope here; this test just asserts the
        // on-disk lock file gets `0o600` on Unix even when it
        // pre-exists with looser permissions.
        let tmp = fresh_repo();
        // Pre-create with permissive mode to verify the unconditional
        // chmod path (M1 reviewer flag C3).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = tmp.path().join(".issuectl");
            fs::create_dir_all(&dir).unwrap();
            let lock = dir.join("write.lock");
            fs::write(&lock, b"").unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
        }
        let _l1 = WriteLock::acquire(tmp.path()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = fs::metadata(tmp.path().join(".issuectl/write.lock")).unwrap();
            assert_eq!(
                m.permissions().mode() & 0o777,
                0o600,
                "lock file should be 0o600 even if it pre-existed at 0o644"
            );
        }
    }
}
