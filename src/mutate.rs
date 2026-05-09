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
    /// Per-key custom-frontmatter PATCH. Mirrors the top-level `Patch`
    /// ternary: omitted (no entry) leaves the key alone; `null` removes
    /// the key; a string sets it. Built-in keys (`status`, `priority`,
    /// dates, etc.) are reserved here — use the dedicated request slots.
    #[serde(default)]
    pub custom_fields: std::collections::BTreeMap<String, Patch<String>>,
    /// CLI-only: compute the post-mutation bytes and return them via
    /// `UpdateOutcome::pending_serialized` instead of writing or
    /// publishing. The flock is still acquired so the read+plan is
    /// consistent with concurrent writers. Skipped by serde so JSON
    /// clients can't drive a dry-run via the wire format.
    #[serde(skip)]
    pub dry_run: bool,
}

/// Frontmatter keys that have dedicated CLI flags or request-shape
/// slots, or are auto-managed by the mutation layer. Forbidden inside
/// `custom_fields` (both new and update paths) — the second column is a
/// user-facing hint pointing at the right slot.
///
/// Single source of truth: `parse_custom_field` /
/// `parse_custom_field_key` in `main.rs` and
/// `UpdateIssueRequest::validate` all consume this constant. Adding a
/// new built-in field must update this list once.
pub const RESERVED_CUSTOM_FIELD_KEYS: &[(&str, &str)] = &[
    ("type", "--type"),
    ("title", "--title"),
    ("slug", "--slug"),
    ("reporter", "--reporter"),
    ("assignee", "--assignee"),
    ("owner", "--owner"),
    ("priority", "--priority"),
    ("epic", "--epic"),
    ("labels", "--label (repeatable)"),
    ("related", "--related (repeatable)"),
    ("status", "set automatically by `new` (always `open`)"),
    ("created", "set automatically by `new` (today)"),
    ("updated", "set automatically by `new`/`update` (today)"),
    ("closed", "set automatically when status moves to a closing value"),
    ("commits", "use `update --add-commit` after creation"),
];

/// Returns the user-facing hint for a reserved key, or `None` if the
/// key is free for custom use.
pub fn reserved_custom_field_hint(key: &str) -> Option<&'static str> {
    RESERVED_CUSTOM_FIELD_KEYS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, hint)| *hint)
}

pub fn is_valid_custom_field_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
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
            && self.custom_fields.is_empty()
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

        for (key, patch) in &self.custom_fields {
            if !is_valid_custom_field_key(key) {
                return Err(MutateError::Validation(format!(
                    "custom field key {key:?} must be alphanumeric / underscore / hyphen"
                )));
            }
            if let Some(hint) = reserved_custom_field_hint(key) {
                return Err(MutateError::Validation(format!(
                    "custom field {key:?} is built-in: {hint}"
                )));
            }
            if let Patch::Set(v) = patch {
                // Reject blank/whitespace-only Sets so the API and CLI
                // agree (the CLI parser strips and rejects empty;
                // without `trim()` here a JSON client could still slip
                // a `"   "` through to disk).
                if v.trim().is_empty() {
                    return Err(MutateError::Validation(format!(
                        "custom field {key:?}: empty-string Set is not allowed (use null to clear)"
                    )));
                }
                if v.trim() != v.as_str() {
                    return Err(MutateError::Validation(format!(
                        "custom field {key:?}: leading or trailing whitespace is not allowed"
                    )));
                }
            }
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
    /// Set when the request had `dry_run = true` (or the body-mutation
    /// callers' `dry_run` flag). Carries the bytes that *would* have
    /// been written so the CLI can render a unified diff. `None` for
    /// real writes — the file on disk is the authoritative version.
    pub pending_serialized: Option<String>,
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
    /// Post-mutation frontmatter violates the repo schema. Mapped to
    /// 422 — the client can fix it by adjusting the request (e.g.
    /// supplying `--field team=...`).
    SchemaViolation(String),
    /// `.schema.yaml` is malformed or rejected at load time (bad
    /// version, deny_unknown_fields, unsatisfiable required field).
    /// Mapped to 5xx — the client cannot fix it from the request; an
    /// operator must edit the schema file.
    SchemaConfig(String),
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
            MutateError::SchemaViolation(s) => write!(f, "schema: {s}"),
            MutateError::SchemaConfig(s) => write!(f, "schema config: {s}"),
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
    // Dry-run noop also short-circuits — the diff would be empty.
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
            pending_serialized: None,
        });
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    crate::schema::ensure_default_written(root).map_err(MutateError::Io)?;

    // 1) locate, then in-line migrate any legacy path under the flock
    //    so writes always land at the canonical flat path.
    let item_path = locate_and_migrate(root, slug)?;
    let schema = crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    update_issue_under_lock(slug, item_path, req, hub, &schema)
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
    schema: &crate::schema::Schema,
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

    // Custom-field patches. Reserved-key / shape checks already ran in
    // `validate()`; here we just translate the ternary onto the YAML
    // mapping. `Unspecified` shouldn't appear (BTreeMap entries imply
    // the caller mentioned the key) but is handled defensively.
    for (key, patch) in &req.custom_fields {
        match patch {
            Patch::Unspecified => {}
            Patch::Clear => write::remove_key(&mut item.frontmatter, key),
            Patch::Set(v) => write::set_string(&mut item.frontmatter, key, v),
        }
    }

    write::set_string(&mut item.frontmatter, "updated", &write::today());

    // Reopen flow: when transitioning closing → active, append a
    // `## Reopen Notes — <date>` section so the rationale isn't
    // implicit. One section per transition (multiple reopens stack).
    if moved_to_open {
        let trimmed_body = item.body.trim_start_matches('\n');
        let with_section =
            crate::body_sections::append_reopen_notes(trimmed_body, &write::today());
        item.body = crate::body_sections::canonicalise_body_leading(&with_section);
    }

    // 4b) schema validation against the post-mutation frontmatter. The
    //     built-in clap parsers already guard known enums; this layer
    //     enforces user-declared required fields and custom enums
    //     (e.g. a constrained `labels` enum). Schema is loaded once by
    //     the caller and threaded in so we don't re-read the file on
    //     each mutation.
    let violations = crate::schema::validate(schema, &item.frontmatter);
    if !violations.is_empty() {
        let msg = violations
            .iter()
            .map(|v| v.message())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(MutateError::SchemaViolation(msg));
    }

    // 5) Either dry-run (compute serialized bytes, skip write/publish)
    //    or atomic write. No directory rename — flat layout means
    //    `item_path` is the canonical location regardless of status.
    if req.dry_run {
        let pending = write::serialize_item(&item).map_err(MutateError::Io)?;
        let new_issue = parse_in_memory(&item, slug);
        let new_version = canonical_hash(&new_issue);
        return Ok(UpdateOutcome {
            issue: new_issue,
            version: new_version,
            issue_dir: item_path
                .parent()
                .expect("item.md has a parent")
                .to_path_buf(),
            moved_to_closed,
            moved_to_open,
            pending_serialized: Some(pending),
        });
    }
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
        pending_serialized: None,
    })
}

/// Parse an in-memory `ItemFile` into a domain `Issue` by serializing
/// to a string and re-running the parser. Used by dry-run paths so we
/// don't have to rebuild the parser logic for in-memory mutations.
fn parse_in_memory(item: &ItemFile, slug: &str) -> Issue {
    let serialized =
        write::serialize_item(item).expect("serialize succeeds for valid in-memory item");
    let parsed = crate::parser::parse_item_md_text_with_warnings(
        &serialized,
        slug,
        "open",
        Path::new("<dry-run>"),
    );
    let mut issue = parsed.issue;
    issue.folder = folder_for_status(&issue.status).to_string();
    issue
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
    crate::schema::ensure_default_written(root).map_err(MutateError::Io)?;

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
    let schema = crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    update_issue_under_lock(slug, item_path, req_normalized, hub, &schema)
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
    dry_run: bool,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    crate::schema::ensure_default_written(root).map_err(MutateError::Io)?;

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

    // Schema validation: body-set doesn't change frontmatter shape but
    // the schema may have tightened since the last write. Refusing here
    // matches the `update_issue` contract.
    let schema = crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let violations = crate::schema::validate(&schema, &item.frontmatter);
    if !violations.is_empty() {
        let msg = violations
            .iter()
            .map(|v| v.message())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(MutateError::SchemaViolation(msg));
    }

    if dry_run {
        let pending = write::serialize_item(&item).map_err(MutateError::Io)?;
        let new_issue = parse_in_memory(&item, slug);
        let new_version = canonical_hash(&new_issue);
        return Ok(UpdateOutcome {
            issue: new_issue,
            version: new_version,
            issue_dir: item_path
                .parent()
                .expect("item.md has a parent")
                .to_path_buf(),
            moved_to_closed: false,
            moved_to_open: false,
            pending_serialized: Some(pending),
        });
    }

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
        pending_serialized: None,
    })
}

/// Append a timestamped block to the issue's `## Comments` section
/// (creating it if missing). Same flock + optimistic-version contract
/// as `update_issue`. Body-only mutation: `status`, `closed`, etc.
/// are untouched, so this never causes a status transition or
/// directory rename.
pub fn note_issue(
    root: &Path,
    slug: &str,
    author: &str,
    message: &str,
    section: &str,
    expected_version: Option<String>,
    hub: Option<&Arc<EventHub>>,
    dry_run: bool,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }
    crate::body_sections::validate_author(author)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    crate::body_sections::validate_message(message)
        .map_err(|e| MutateError::Validation(e.to_string()))?;

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
    let block = crate::body_sections::render_note_block(
        &crate::body_sections::now_iso(),
        author,
        message,
    )
    .map_err(|e| MutateError::Validation(e.to_string()))?;
    let trimmed_body = item.body.trim_start_matches('\n');
    let appended = crate::body_sections::append_block(trimmed_body, section, &block);
    // Canonicalise leading-newline shape so `serialize_item` always
    // produces `---\n\n<body>` rather than leaving a legacy
    // no-blank-line file in a state `fmt` would still want to change.
    item.body = crate::body_sections::canonicalise_body_leading(&appended);
    write::set_string(&mut item.frontmatter, "updated", &write::today());

    if dry_run {
        let pending = write::serialize_item(&item).map_err(MutateError::Io)?;
        let new_issue = parse_in_memory(&item, slug);
        let new_version = canonical_hash(&new_issue);
        return Ok(UpdateOutcome {
            issue: new_issue,
            version: new_version,
            issue_dir: item_path
                .parent()
                .expect("item.md has a parent")
                .to_path_buf(),
            moved_to_closed: false,
            moved_to_open: false,
            pending_serialized: Some(pending),
        });
    }

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
        pending_serialized: None,
    })
}

/// Toggle a markdown checklist item in the issue body. Matches the
/// first body line containing `substring` whose stripped text starts
/// with `- [ ]` or `- [x]` (case-insensitive on the cross marker), and
/// flips its checkbox in place. Errors when zero or multiple lines
/// match. Same flock + optimistic-version contract as `update_body`.
pub fn toggle_checkbox(
    root: &Path,
    slug: &str,
    substring: &str,
    expected_version: Option<String>,
    hub: Option<&Arc<EventHub>>,
    dry_run: bool,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }
    if substring.trim().is_empty() {
        return Err(MutateError::Validation(
            "task substring cannot be empty".into(),
        ));
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    crate::schema::ensure_default_written(root).map_err(MutateError::Io)?;

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
    let new_body = toggle_checkbox_in_body(&item.body, substring)?;
    item.body = new_body;
    write::set_string(&mut item.frontmatter, "updated", &write::today());

    if dry_run {
        let pending = write::serialize_item(&item).map_err(MutateError::Io)?;
        let new_issue = parse_in_memory(&item, slug);
        let new_version = canonical_hash(&new_issue);
        return Ok(UpdateOutcome {
            issue: new_issue,
            version: new_version,
            issue_dir: item_path
                .parent()
                .expect("item.md has a parent")
                .to_path_buf(),
            moved_to_closed: false,
            moved_to_open: false,
            pending_serialized: Some(pending),
        });
    }

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
        pending_serialized: None,
    })
}

/// Find a unique checkbox line containing `substring` and return the
/// body with that one line's `[ ]` / `[x]` toggled. The checkbox shape
/// matched is `^\s*[-*+]\s+\[[ xX]\]\s` so common GFM variants work,
/// while still rejecting `- [n]` or other non-checkbox brackets.
fn toggle_checkbox_in_body(body: &str, substring: &str) -> Result<String, MutateError> {
    let mut matches: Vec<usize> = Vec::new();
    let lines: Vec<&str> = body.split('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        if checkbox_state(line).is_some() && line.contains(substring) {
            matches.push(i);
        }
    }
    match matches.len() {
        0 => Err(MutateError::Validation(format!(
            "no checkbox line matched {substring:?}"
        ))),
        1 => {
            let idx = matches[0];
            let toggled = toggle_line_checkbox(lines[idx]);
            let mut out = Vec::with_capacity(lines.len());
            for (i, l) in lines.iter().enumerate() {
                if i == idx {
                    out.push(toggled.clone());
                } else {
                    out.push((*l).to_string());
                }
            }
            Ok(out.join("\n"))
        }
        n => Err(MutateError::Validation(format!(
            "{n} checkbox lines matched {substring:?}; refine to a unique substring"
        ))),
    }
}

/// `Some(true)` for `- [x]`, `Some(false)` for `- [ ]`, `None` otherwise.
fn checkbox_state(line: &str) -> Option<bool> {
    let trimmed = line.trim_start();
    let bullet = trimmed.chars().next()?;
    if !matches!(bullet, '-' | '*' | '+') {
        return None;
    }
    let rest = &trimmed[bullet.len_utf8()..];
    if !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let rest = rest.trim_start();
    if !rest.starts_with('[') || rest.len() < 4 || &rest[2..3] != "]" {
        return None;
    }
    let mark = rest.as_bytes()[1];
    let after = rest.as_bytes().get(3).copied();
    if !matches!(after, Some(b' ') | Some(b'\t')) {
        return None;
    }
    match mark {
        b' ' => Some(false),
        b'x' | b'X' => Some(true),
        _ => None,
    }
}

fn toggle_line_checkbox(line: &str) -> String {
    let lead_len = line.len() - line.trim_start().len();
    let (lead, rest) = line.split_at(lead_len);
    // rest looks like `- [ ] foo` (or `* [x] foo` / `+ [X] foo`).
    let bullet = &rest[..1];
    let after_bullet = &rest[1..];
    let bullet_pad_len = after_bullet.len() - after_bullet.trim_start().len();
    let (bullet_pad, body) = after_bullet.split_at(bullet_pad_len);
    // body == `[X] ...`
    let mark = body.as_bytes()[1];
    let new_mark = if mark == b' ' { 'x' } else { ' ' };
    format!(
        "{lead}{bullet}{bullet_pad}[{new_mark}]{}",
        &body[3..]
    )
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
    /// Custom frontmatter fields keyed by field name, mirroring CLI
    /// `--field key=value`. Required for repos whose schema declares
    /// custom required fields — without this, API creation cannot
    /// satisfy the schema and falls into the same bricking failure
    /// mode the CLI `--field` flag was added to fix.
    #[serde(default)]
    pub custom_fields: std::collections::BTreeMap<String, String>,
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
            custom_fields: req.custom_fields.into_iter().collect(),
        },
    )
    .map_err(|e| match e {
        crate::DoNewError::SchemaViolation(s) => MutateError::SchemaViolation(s),
        crate::DoNewError::SchemaConfig(s) => MutateError::SchemaConfig(s),
        crate::DoNewError::Conflict(s) => MutateError::ConflictingIntent(s),
        crate::DoNewError::Validation(s) => MutateError::Validation(s),
        crate::DoNewError::Io(e) => MutateError::Io(e),
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
    fn simple_unknown_scalar_fields_survive_read_write_round_trip() {
        // A user-added `triage:` key on a fresh issue with a simple
        // scalar value survives a no-op read→write cycle without
        // textual change, AND lands in `Issue::extra` so
        // canonical_hash sees it. Byte identity is *not* a general
        // contract — `serde_yaml` reformats comments, scalar styles,
        // anchors, list flow style — but for the simple
        // `key: scalar` case it's stable, and that's the case this
        // test pins down.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/keep-triage");
        fs::create_dir_all(&dir).unwrap();
        let original = "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
                        priority: normal\ntriage: alice\nreviewer: bob\n\
                        ---\n\n# Title\n";
        fs::write(dir.join("item.md"), original).unwrap();
        let item = crate::write::read_item(&dir.join("item.md")).unwrap();
        crate::write::write_item(&dir.join("item.md"), &item).unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert_eq!(original, after);

        // The parsed Issue must carry the unknowns into `extra` so
        // canonical_hash sees them.
        let parsed =
            crate::parser::parse_item_md_with_warnings(&dir.join("item.md"), "keep-triage", "open");
        assert_eq!(
            parsed.issue.extra.get("triage"),
            Some(&serde_json::Value::String("alice".into()))
        );
        assert_eq!(
            parsed.issue.extra.get("reviewer"),
            Some(&serde_json::Value::String("bob".into()))
        );
    }

    #[test]
    fn unknown_field_edits_with_refreshed_version_do_not_block_later_updates() {
        // Two external writes land in sequence on different custom
        // keys (`triage:` then `reviewer:`); the third writer takes
        // the post-edit version and PATCHes a known field. No 409
        // because the third writer didn't carry a stale view. This
        // does NOT prove field-level merge — whole-document
        // optimistic concurrency means a writer that *was* stale on
        // either custom key would still 409 (covered separately by
        // `external_edit_to_unknown_field_makes_stale_version_409`).
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/concurrent-distinct");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\n---\n\n# Title\n",
        )
        .unwrap();

        // Edit #1: external writer adds `triage: alice`. Then a
        // mutate.rs PATCH with the *post-external-edit* version
        // succeeds — no 409 because we picked up the new hash first.
        let v0 = crate::canonical::canonical_hash(
            &crate::parser::parse_item_md_with_warnings(
                &dir.join("item.md"),
                "concurrent-distinct",
                "open",
            )
            .issue,
        );
        let mut item = crate::write::read_item(&dir.join("item.md")).unwrap();
        item.frontmatter.insert(
            serde_yaml::Value::String("triage".into()),
            serde_yaml::Value::String("alice".into()),
        );
        crate::write::write_item(&dir.join("item.md"), &item).unwrap();
        let v1 = crate::canonical::canonical_hash(
            &crate::parser::parse_item_md_with_warnings(
                &dir.join("item.md"),
                "concurrent-distinct",
                "open",
            )
            .issue,
        );
        assert_ne!(v0, v1, "adding unknown key must change the hash");

        // Edit #2: another external writer adds `reviewer: bob` while
        // *holding the fresh* v1, then `issuectl update --priority high
        // --expected-version v2` lands cleanly.
        let mut item = crate::write::read_item(&dir.join("item.md")).unwrap();
        item.frontmatter.insert(
            serde_yaml::Value::String("reviewer".into()),
            serde_yaml::Value::String("bob".into()),
        );
        crate::write::write_item(&dir.join("item.md"), &item).unwrap();
        let v2 = crate::canonical::canonical_hash(
            &crate::parser::parse_item_md_with_warnings(
                &dir.join("item.md"),
                "concurrent-distinct",
                "open",
            )
            .issue,
        );
        assert_ne!(v1, v2);

        let req = UpdateIssueRequest {
            expected_version: Some(v2),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "concurrent-distinct", req, None).unwrap();
        assert_eq!(out.issue.priority, "high");
        // Both unknown keys must survive the mutation round-trip.
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("triage: alice"));
        assert!(on_disk.contains("reviewer: bob"));
    }

    #[test]
    fn external_edit_to_unknown_field_makes_stale_version_409() {
        // The contract: an unknown field changing under a writer's
        // feet must trip optimistic concurrency the same way a known
        // field would. Without unknown-key projection in
        // `canonical_hash`, this PATCH would silently succeed and
        // could clobber a custom field the writer never read.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/concurrent-same-key");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\ntriage: alice\n---\n\n# Title\n",
        )
        .unwrap();
        let v0 = crate::canonical::canonical_hash(
            &crate::parser::parse_item_md_with_warnings(
                &dir.join("item.md"),
                "concurrent-same-key",
                "open",
            )
            .issue,
        );

        // External writer overwrites the same unknown key.
        let mut item = crate::write::read_item(&dir.join("item.md")).unwrap();
        item.frontmatter.insert(
            serde_yaml::Value::String("triage".into()),
            serde_yaml::Value::String("bob".into()),
        );
        crate::write::write_item(&dir.join("item.md"), &item).unwrap();

        // Original writer comes back with v0 — must 409.
        let req = UpdateIssueRequest {
            expected_version: Some(v0),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "concurrent-same-key", req, None).unwrap_err();
        match err {
            MutateError::VersionMismatch { current, .. } => {
                assert_eq!(current.slug, "concurrent-same-key");
                assert_eq!(
                    current.extra.get("triage"),
                    Some(&serde_json::Value::String("bob".into())),
                    "current state surfaced to the caller must reflect the new unknown value"
                );
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn external_delete_of_unknown_field_makes_stale_version_409() {
        // Symmetric to the same-key 409 test: a writer who saved
        // `triage: alice` and didn't notice an external `git pull`
        // wiped the key must still trip optimistic concurrency,
        // because removal changes the hash too.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/concurrent-delete-key");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\ntriage: alice\n---\n\n# Title\n",
        )
        .unwrap();
        let v0 = crate::canonical::canonical_hash(
            &crate::parser::parse_item_md_with_warnings(
                &dir.join("item.md"),
                "concurrent-delete-key",
                "open",
            )
            .issue,
        );

        // External writer removes the unknown key entirely.
        let mut item = crate::write::read_item(&dir.join("item.md")).unwrap();
        crate::write::remove_key(&mut item.frontmatter, "triage");
        crate::write::write_item(&dir.join("item.md"), &item).unwrap();

        let req = UpdateIssueRequest {
            expected_version: Some(v0),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "concurrent-delete-key", req, None).unwrap_err();
        assert!(
            matches!(err, MutateError::VersionMismatch { .. }),
            "expected VersionMismatch on stale view after unknown-key delete, got {err:?}"
        );
    }

    #[test]
    fn non_string_nested_key_in_unknown_value_warns_not_panics() {
        // YAML allows non-string mapping keys; JSON does not. The
        // parser must surface that as a `MutateError::Corrupt`
        // (carrying the warning) rather than letting the hash code
        // panic on a `serde_json::to_value` failure.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/bad-nested-key");
        fs::create_dir_all(&dir).unwrap();
        // `? [1, 2]` is YAML's explicit-key syntax for a sequence
        // key. Top-level keys are still strings (so the frontmatter
        // parses); the offending non-string key lives inside
        // `weird:`.
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\nweird:\n  ? [1, 2]\n  : foo\n---\n\n# Title\n",
        )
        .unwrap();

        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "bad-nested-key", req, None).unwrap_err();
        match err {
            MutateError::Corrupt { warnings } => {
                assert!(
                    warnings.iter().any(|w| w.contains("weird")
                        && (w.contains("string") || w.contains("mapping key"))),
                    "expected a warning naming the bad key, got: {warnings:?}"
                );
            }
            other => panic!("expected Corrupt, got {other:?}"),
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
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: fixed\npriority: normal\n---\n\n# T\n",
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
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: open\npriority: normal\n---\n\n# T\n",
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
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: open\npriority: normal\nepic: foo-bar\n---\n\n# T\n",
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
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: open\npriority: normal\nepic: stay-here\n---\n\n# T\n",
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
    fn update_sets_custom_field_via_patch() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "cf-set", "open");
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("triage".into(), Patch::Set("P1".into()));
        let out = update_issue(tmp.path(), "cf-set", req, None).unwrap();
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("triage: P1"), "got: {on_disk}");
        assert_eq!(
            out.issue.extra.get("triage"),
            Some(&serde_json::Value::String("P1".into()))
        );
    }

    #[test]
    fn update_clears_custom_field_via_null_patch() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/cf-clear");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\nowner_team: payments\n---\n\n# T\n",
        )
        .unwrap();
        let mut req = UpdateIssueRequest::default();
        req.custom_fields.insert("owner_team".into(), Patch::Clear);
        let out = update_issue(tmp.path(), "cf-clear", req, None).unwrap();
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(
            !on_disk.contains("owner_team:"),
            "owner_team should be removed; got: {on_disk}"
        );
    }

    #[test]
    fn update_custom_field_set_and_clear_atomic() {
        // JSON `{"custom_fields": {"triage": "P1", "owner_team": null}}`
        // sets one key and removes another in a single PATCH.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/cf-mixed");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\nowner_team: payments\n---\n\n# T\n",
        )
        .unwrap();
        let req: UpdateIssueRequest = serde_json::from_str(
            r#"{"custom_fields": {"triage": "P1", "owner_team": null}}"#,
        )
        .unwrap();
        let out = update_issue(tmp.path(), "cf-mixed", req, None).unwrap();
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("triage: P1"));
        assert!(!on_disk.contains("owner_team:"));
    }

    #[test]
    fn update_custom_field_rejects_reserved_key() {
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("status".into(), Patch::Set("done".into()));
        let err = req.validate().unwrap_err();
        match err {
            MutateError::Validation(msg) => assert!(msg.contains("built-in"), "got: {msg}"),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn update_custom_field_rejects_invalid_key_shape() {
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("bad key!".into(), Patch::Set("x".into()));
        assert!(matches!(req.validate(), Err(MutateError::Validation(_))));
    }

    #[test]
    fn update_custom_field_rejects_empty_string_set() {
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("triage".into(), Patch::Set("".into()));
        let err = req.validate().unwrap_err();
        assert!(
            matches!(err, MutateError::Validation(ref m) if m.contains("empty-string")),
            "got: {err:?}"
        );
    }

    #[test]
    fn update_custom_field_violating_schema_is_rejected() {
        // Schema declares `triage` required + enum; the PATCH supplies a
        // value outside the enum. Post-mutation schema validation must
        // 422 it (no on-disk change).
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  triage:\n    enum: [P0, P1, P2]\n",
        )
        .unwrap();
        let _v0 = seed_issue(tmp.path(), "open", "cf-schema", "open");
        let dir = tmp.path().join("issues/cf-schema");
        let before = fs::read_to_string(dir.join("item.md")).unwrap();
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("triage".into(), Patch::Set("P9".into()));
        let err = update_issue(tmp.path(), "cf-schema", req, None).unwrap_err();
        assert!(
            matches!(err, MutateError::SchemaViolation(_)),
            "got: {err:?}"
        );
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert_eq!(before, after, "schema-rejected PATCH must not write");
    }

    #[test]
    fn update_custom_field_bumps_canonical_version() {
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "cf-bump", "open");
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("triage".into(), Patch::Set("P1".into()));
        let out = update_issue(tmp.path(), "cf-bump", req, None).unwrap();
        assert_ne!(v0, out.version, "custom-field PATCH must change the hash");
    }

    #[test]
    fn update_custom_field_with_stale_version_returns_409() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "cf-stale", "open");
        let mut req = UpdateIssueRequest {
            expected_version: Some("sha256:deadbeef".into()),
            ..Default::default()
        };
        req.custom_fields
            .insert("triage".into(), Patch::Set("P1".into()));
        let err = update_issue(tmp.path(), "cf-stale", req, None).unwrap_err();
        assert!(matches!(err, MutateError::VersionMismatch { .. }));
    }

    #[test]
    fn update_custom_field_repairs_missing_required_schema_field() {
        // The motivating bug: a schema introduces a required custom
        // field, an existing issue lacks it, and every PATCH 422s on
        // SchemaViolation. The fix is exactly that the same PATCH can
        // SUPPLY the missing field via `custom_fields` and succeed.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n    enum: [payments, infra]\n",
        )
        .unwrap();
        let _ = seed_issue(tmp.path(), "open", "cf-required-repair", "open");

        // Sanity: a no-custom-field PATCH is rejected.
        let err = update_issue(
            tmp.path(),
            "cf-required-repair",
            UpdateIssueRequest {
                priority: Patch::Set("high".into()),
                ..Default::default()
            },
            None,
        )
        .unwrap_err();
        assert!(
            matches!(err, MutateError::SchemaViolation(_)),
            "expected SchemaViolation without team set, got {err:?}"
        );

        // The repair PATCH supplies the missing key.
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("team".into(), Patch::Set("payments".into()));
        let out = update_issue(tmp.path(), "cf-required-repair", req, None).unwrap();
        assert_eq!(
            out.issue.extra.get("team"),
            Some(&serde_json::Value::String("payments".into()))
        );
    }

    #[test]
    fn update_custom_field_rejects_whitespace_only_set() {
        // `--field key=" "` is rejected by the CLI parser; the API
        // path must reject it too so a JSON client cannot smuggle a
        // blank value past `validate()`.
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("triage".into(), Patch::Set("   ".into()));
        let err = req.validate().unwrap_err();
        assert!(
            matches!(err, MutateError::Validation(ref m) if m.contains("empty-string")),
            "got: {err:?}"
        );
    }

    #[test]
    fn update_custom_field_rejects_padded_set() {
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("triage".into(), Patch::Set(" P1".into()));
        assert!(matches!(req.validate(), Err(MutateError::Validation(_))));
    }

    #[test]
    fn deserialize_custom_fields_supports_set_clear() {
        let r: UpdateIssueRequest = serde_json::from_str(
            r#"{"custom_fields": {"a": "x", "b": null}}"#,
        )
        .unwrap();
        assert!(matches!(r.custom_fields.get("a"), Some(Patch::Set(s)) if s == "x"));
        assert!(matches!(r.custom_fields.get("b"), Some(Patch::Clear)));
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
            false,
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
            false,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::VersionMismatch { .. }));
    }

    #[test]
    fn reopen_appends_reopen_notes_section() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/reopen-section");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-01\nstatus: fixed\n\
             priority: normal\nclosed: 2026-05-05\n---\n\n# T\n\n## Description\n\nx\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "reopen-section", req, None).unwrap();
        let on_disk = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(
            on_disk.contains("## Reopen Notes —"),
            "expected Reopen Notes section, got:\n{on_disk}"
        );
    }

    #[test]
    fn note_appends_to_comments_section() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/notable-issue-x");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T\n\n## Description\n\nx\n",
        )
        .unwrap();
        // First note creates the section.
        let _ = note_issue(
            tmp.path(),
            "notable-issue-x",
            "alice",
            "first thought",
            crate::body_sections::COMMENTS,
            None,
            None,
            false,
        )
        .unwrap();
        let after1 = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after1.contains("## Comments"));
        assert!(after1.contains("first thought"));
        // Second note appends without duplicating the section.
        let _ = note_issue(
            tmp.path(),
            "notable-issue-x",
            "bob",
            "second thought",
            crate::body_sections::COMMENTS,
            None,
            None,
            false,
        )
        .unwrap();
        let after2 = fs::read_to_string(dir.join("item.md")).unwrap();
        assert_eq!(after2.matches("## Comments").count(), 1);
        assert!(after2.contains("first thought"));
        assert!(after2.contains("second thought"));
        let i_first = after2.find("first thought").unwrap();
        let i_second = after2.find("second thought").unwrap();
        assert!(i_first < i_second);
        // Description preserved.
        assert!(after2.contains("## Description"));
    }

    #[test]
    fn note_rejects_stale_version() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "stale-note-here", "open");
        let err = note_issue(
            tmp.path(),
            "stale-note-here",
            "alice",
            "hi",
            crate::body_sections::COMMENTS,
            Some("sha256:deadbeef".into()),
            None,
            false,
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

    #[test]
    fn update_writes_default_schema_on_first_use() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "bootstrap-target", "open");
        assert!(!tmp.path().join("issues/.schema.yaml").exists());
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "bootstrap-target", req, None).unwrap();
        assert!(
            tmp.path().join("issues/.schema.yaml").is_file(),
            "schema file should be auto-written on first mutation"
        );
    }

    #[test]
    fn update_rejects_label_outside_schema_enum() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "label-enum-target", "open");
        // Constrain labels to a fixed set.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n  labels:\n    list: true\n    enum: [infra, frontend]\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            add_labels: vec!["bogus".into()],
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "label-enum-target", req, None).unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => assert!(
                msg.contains("labels") && msg.contains("bogus"),
                "expected labels/bogus in error, got {msg:?}"
            ),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn body_set_validates_against_schema() {
        // A schema tightened after the issue was created should block
        // body-set, matching the contract update_issue follows.
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "body-schema-target", "open");
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let err = update_body(
            tmp.path(),
            "body-schema-target",
            Some(v0),
            "# new body\n".into(),
            None,
            false,
        )
        .unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => assert!(msg.contains("team"), "got {msg:?}"),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn new_issue_passes_custom_fields_through() {
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n    enum: [payments, infra]\n",
        )
        .unwrap();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "API new with custom field".into();
        req.priority = "normal".into();
        req.custom_fields.insert("team".into(), "payments".into());
        let outcome = new_issue(tmp.path(), req, None).unwrap();
        let on_disk = fs::read_to_string(outcome.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("team: payments"), "got {on_disk}");
    }

    #[test]
    fn new_issue_schema_violation_returns_typed_error() {
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Missing team".into();
        req.priority = "normal".into();
        let err = new_issue(tmp.path(), req, None).unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => assert!(
                msg.contains("team"),
                "expected `team` in error, got {msg:?}"
            ),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn new_issue_slug_conflict_returns_typed_error() {
        // do_new_locked rejects an explicit slug whose flat directory
        // already exists. Pre-typed-error refactor this surfaced via a
        // string match on `"already" / "exists"`; now it must come
        // through DoNewError::Conflict → MutateError::ConflictingIntent.
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/taken-slug")).unwrap();
        fs::write(
            tmp.path().join("issues/taken-slug/item.md"),
            "---\nstatus: open\n---\n",
        )
        .unwrap();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Conflict".into();
        req.priority = "normal".into();
        req.slug = Some("taken-slug".into());
        let err = new_issue(tmp.path(), req, None).unwrap_err();
        assert!(
            matches!(err, MutateError::ConflictingIntent(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn new_issue_legacy_slug_conflict_returns_typed_error() {
        // Companion to `new_issue_slug_conflict_returns_typed_error`:
        // covers the OTHER `DoNewError::Conflict` site where the slug
        // exists at a legacy `issues/open/<slug>/` path. Pre-flat-layout
        // installs hit this branch; the typed mapping must classify it
        // as ConflictingIntent (not Io / not SchemaViolation).
        let tmp = fresh_repo();
        let legacy_open = tmp.path().join("issues/open/legacy-slug");
        fs::create_dir_all(&legacy_open).unwrap();
        fs::write(legacy_open.join("item.md"), "---\nstatus: open\n---\n").unwrap();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Legacy conflict".into();
        req.priority = "normal".into();
        req.slug = Some("legacy-slug".into());
        let err = new_issue(tmp.path(), req, None).unwrap_err();
        assert!(
            matches!(err, MutateError::ConflictingIntent(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn new_issue_validation_returns_typed_error() {
        // Validation paths in do_new_locked (here: `--owner` on a
        // non-epic) used to be the catch-all string-match fallback, so
        // their classification was correct only by accident. Lock it.
        let tmp = fresh_repo();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Owner on non-epic".into();
        req.priority = "normal".into();
        req.owner = Some("alice".into());
        let err = new_issue(tmp.path(), req, None).unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "got {err:?}");
    }

    #[test]
    fn new_issue_schema_config_returns_typed_error() {
        // Malformed `.schema.yaml` is the bug that motivated the typed-
        // error refactor: pre-refactor the catch-all string match
        // misclassified it as MutateError::Validation (HTTP 400). It
        // must now route through DoNewError::SchemaConfig →
        // MutateError::SchemaConfig (HTTP 500).
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields: : :\n",
        )
        .unwrap();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Bad schema".into();
        req.priority = "normal".into();
        let err = new_issue(tmp.path(), req, None).unwrap_err();
        assert!(matches!(err, MutateError::SchemaConfig(_)), "got {err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn new_issue_io_failure_returns_typed_error() {
        // Force `fs::create_dir(<root>/issues/<slug>)` to fail with
        // EACCES by chmod'ing the issues parent to read-only. That
        // path used to be funnelled into the `Validation` fallback by
        // the string matcher; the typed enum routes it correctly to
        // MutateError::Io.
        //
        // RAII guard restores permissions on every exit (including
        // panic) so `tempdir`'s `Drop` cleanup never inherits a
        // 0o500 directory.
        use std::os::unix::fs::PermissionsExt;
        struct PermGuard {
            path: PathBuf,
            original: std::fs::Permissions,
        }
        impl Drop for PermGuard {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.path, self.original.clone());
            }
        }

        let tmp = fresh_repo();
        // Use the production helper rather than a hardcoded YAML literal
        // so the test does not break if the schema format evolves.
        crate::schema::ensure_default_written(tmp.path()).unwrap();
        let issues_dir = tmp.path().join("issues");
        let original = fs::metadata(&issues_dir).unwrap().permissions();
        let mut readonly = original.clone();
        readonly.set_mode(0o500);
        fs::set_permissions(&issues_dir, readonly).unwrap();
        let _guard = PermGuard {
            path: issues_dir.clone(),
            original: original.clone(),
        };

        // chmod 0o500 has no effect for uid 0; skip the assertion when
        // a probe write still succeeds (CI containers occasionally run
        // as root).
        let probe = issues_dir.join(".io-probe");
        let chmod_enforced = fs::write(&probe, b"x").is_err();
        let _ = fs::remove_file(&probe);
        if !chmod_enforced {
            return;
        }

        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Cannot write".into();
        req.priority = "normal".into();
        req.slug = Some("io-fail-slug".into());
        let err = new_issue(tmp.path(), req, None).unwrap_err();

        assert!(matches!(err, MutateError::Io(_)), "got {err:?}");
    }

    #[test]
    fn do_new_error_to_anyhow_text_matches_per_variant() {
        // Lock the byte-identical CLI text contract: the From<DoNewError>
        // for anyhow::Error impl is what `cmd_new` relies on to keep
        // human-readable error messages stable across the typed-error
        // refactor. If a future contributor edits the variants without
        // touching the conversion, this test fails before users do.
        use crate::DoNewError;

        let cases: &[(DoNewError, &str)] = &[
            (
                DoNewError::Validation("--owner is only valid with --type epic".into()),
                "--owner is only valid with --type epic",
            ),
            (
                DoNewError::Conflict("target directory already exists: /x".into()),
                "target directory already exists: /x",
            ),
            (
                DoNewError::SchemaViolation("missing required field \"team\"".into()),
                "schema: missing required field \"team\"",
            ),
            (
                DoNewError::SchemaConfig("cannot read .schema.yaml".into()),
                "cannot read .schema.yaml",
            ),
        ];
        for (err, expected) in cases {
            // Have to clone-by-construction since DoNewError is not Clone.
            let cloned = match err {
                DoNewError::Validation(s) => DoNewError::Validation(s.clone()),
                DoNewError::Conflict(s) => DoNewError::Conflict(s.clone()),
                DoNewError::SchemaViolation(s) => DoNewError::SchemaViolation(s.clone()),
                DoNewError::SchemaConfig(s) => DoNewError::SchemaConfig(s.clone()),
                DoNewError::Io(_) => unreachable!(),
            };
            let any: anyhow::Error = cloned.into();
            assert_eq!(format!("{any:#}"), *expected, "variant {err:?}");
        }

        // Io variant: the inner anyhow::Error is returned as-is, so its
        // context chain is preserved verbatim.
        let io = DoNewError::Io(
            anyhow::Error::msg(std::io::Error::new(std::io::ErrorKind::Other, "disk full"))
                .context("cannot write /tmp/x"),
        );
        let any: anyhow::Error = io.into();
        assert_eq!(format!("{any:#}"), "cannot write /tmp/x: disk full");
    }

    // ── publish-before-release helpers ──────────────────────────────────
    //
    // web-edit-sync §3.1 step 8: every server-mediated mutation must publish
    // its IssueUpserted event WHILE the repo flock is still held, so global
    // seq order matches on-disk write order. The pre-fix `new_issue`
    // delegated to `do_new` (which acquired and released the lock
    // internally) and then published OUTSIDE the lock — a fast-following
    // PATCH could land an event with a smaller seq, breaking dedupe-by-
    // version on the client. The same contract binds `update_issue`,
    // `update_body`, `note_issue`, and `close_issue`; the helpers below
    // exercise the invariant on each.
    //
    // The probe re-opens `.issuectl/write.lock` on a fresh fd and tries a
    // non-blocking `flock(LOCK_EX)`. POSIX `flock(2)` on Linux and macOS is
    // per open-file-description, so a separate `open()` from the same
    // process conflicts. We match `ErrorKind::WouldBlock` exactly so that
    // any *other* error (permission, ENOENT, or a hypothetical `fs2`
    // backend switch to per-process `fcntl` record locks) panics loudly
    // instead of silently passing.

    const PROBE_UNSEEN: u8 = 0;
    const PROBE_HELD: u8 = 1;
    const PROBE_RELEASED: u8 = 2;

    fn install_lock_probe(
        hub: &Arc<EventHub>,
        lock_path: std::path::PathBuf,
    ) -> Arc<std::sync::atomic::AtomicU8> {
        use fs2::FileExt;
        use std::sync::atomic::{AtomicU8, Ordering};

        let observed = Arc::new(AtomicU8::new(PROBE_UNSEEN));
        let probe_state = observed.clone();
        hub.set_on_publish_for_test(Arc::new(move |_evt| {
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock_path)
                .expect("lock file must exist by publish time");
            match f.try_lock_exclusive() {
                Ok(()) => {
                    let _ = FileExt::unlock(&f);
                    probe_state.store(PROBE_RELEASED, Ordering::SeqCst);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    probe_state.store(PROBE_HELD, Ordering::SeqCst);
                }
                Err(e) => panic!(
                    "unexpected error from try_lock_exclusive in publish probe: {e} \
                     (kind={:?}); the test cannot tell whether the flock was held — \
                     fix the probe or investigate fs2 lock semantics on this platform",
                    e.kind()
                ),
            }
        }));
        observed
    }

    fn assert_probe_saw_held(observed: &std::sync::atomic::AtomicU8, mutation: &str) {
        use std::sync::atomic::Ordering;
        match observed.load(Ordering::SeqCst) {
            PROBE_HELD => {}
            PROBE_RELEASED => panic!(
                "{mutation}: IssueUpserted was published AFTER the repo flock was released — \
                 violates web-edit-sync §3.1 step 8 (publish-before-release)"
            ),
            _ => panic!(
                "{mutation}: publish hook never fired — \
                 mutation did not publish IssueUpserted"
            ),
        }
    }

    #[test]
    fn new_issue_publishes_before_releasing_flock() {
        let tmp = fresh_repo();
        let hub = Arc::new(EventHub::new());
        let observed = install_lock_probe(&hub, tmp.path().join(".issuectl/write.lock"));

        let req = NewIssueRequest {
            issue_type: "bug".into(),
            title: "publish under flock".into(),
            priority: "normal".into(),
            ..Default::default()
        };
        new_issue(tmp.path(), req, Some(&hub)).unwrap();

        assert_probe_saw_held(&observed, "new_issue");
    }

    #[test]
    fn update_issue_publishes_before_releasing_flock() {
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "patch-publish-flock", "open");
        let hub = Arc::new(EventHub::new());
        let observed = install_lock_probe(&hub, tmp.path().join(".issuectl/write.lock"));

        let req = UpdateIssueRequest {
            expected_version: Some(v0),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "patch-publish-flock", req, Some(&hub)).unwrap();

        assert_probe_saw_held(&observed, "update_issue");
    }

    #[test]
    fn close_issue_publishes_before_releasing_flock() {
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "close-publish-flock", "open");
        let hub = Arc::new(EventHub::new());
        let observed = install_lock_probe(&hub, tmp.path().join(".issuectl/write.lock"));

        close_issue(
            tmp.path(),
            "close-publish-flock",
            None,
            Vec::new(),
            Some(v0),
            Some(&hub),
        )
        .unwrap();

        assert_probe_saw_held(&observed, "close_issue");
    }

    #[test]
    fn update_body_publishes_before_releasing_flock() {
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "body-publish-flock", "open");
        let hub = Arc::new(EventHub::new());
        let observed = install_lock_probe(&hub, tmp.path().join(".issuectl/write.lock"));

        update_body(
            tmp.path(),
            "body-publish-flock",
            Some(v0),
            "# Replaced body\n".into(),
            Some(&hub),
            false,
        )
        .unwrap();

        assert_probe_saw_held(&observed, "update_body");
    }

    #[test]
    fn note_issue_publishes_before_releasing_flock() {
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "note-publish-flock", "open");
        let hub = Arc::new(EventHub::new());
        let observed = install_lock_probe(&hub, tmp.path().join(".issuectl/write.lock"));

        note_issue(
            tmp.path(),
            "note-publish-flock",
            "alice",
            "hello from a probe",
            crate::body_sections::COMMENTS,
            Some(v0),
            Some(&hub),
            false,
        )
        .unwrap();

        assert_probe_saw_held(&observed, "note_issue");
    }

    #[test]
    fn new_issue_does_not_publish_on_error_path() {
        // Symmetric guarantee: when the mutation fails (here: a schema
        // violation rejected before any write), no IssueUpserted may be
        // emitted. Otherwise the API would announce a state change that
        // never landed on disk.
        use std::sync::atomic::Ordering;

        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let hub = Arc::new(EventHub::new());
        let observed = install_lock_probe(&hub, tmp.path().join(".issuectl/write.lock"));

        let req = NewIssueRequest {
            issue_type: "bug".into(),
            title: "missing required team".into(),
            priority: "normal".into(),
            ..Default::default()
        };
        let err = new_issue(tmp.path(), req, Some(&hub)).unwrap_err();
        assert!(matches!(err, MutateError::SchemaViolation(_)));
        assert_eq!(
            observed.load(Ordering::SeqCst),
            PROBE_UNSEEN,
            "new_issue published an IssueUpserted on an error path — \
             the API would announce a write that never happened"
        );
    }

    #[test]
    fn malformed_schema_surfaces_as_schema_error() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "broken-schema-target", "open");
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields: : :\n",
        )
        .unwrap();
        let err = update_issue(
            tmp.path(),
            "broken-schema-target",
            UpdateIssueRequest {
                priority: Patch::Set("high".into()),
                ..Default::default()
            },
            None,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::SchemaConfig(_)), "got {err:?}");
    }

    #[test]
    fn update_rejects_when_custom_required_field_missing() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "custom-required-target", "open");
        // Add a custom required field after the issue exists. Any
        // mutation should now fail until the user adds the field.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n  team:\n    required: true\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "custom-required-target", req, None).unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => assert!(
                msg.contains("team"),
                "expected `team` in error, got {msg:?}"
            ),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    // ── Mutation-CLI verbs (set/check/label/apply via mutate.rs) ──────

    fn seed_with_body(root: &Path, slug: &str, body: &str) -> String {
        let dir = root.join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            format!(
                "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n{body}",
            ),
        )
        .unwrap();
        let parsed =
            crate::parser::parse_item_md_with_warnings(&dir.join("item.md"), slug, "open");
        let mut issue = parsed.issue;
        issue.folder = folder_for_status(&issue.status).to_string();
        canonical_hash(&issue)
    }

    #[test]
    fn dry_run_does_not_write_and_returns_pending_serialized() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "dryrun-target-x", "open");
        let before = fs::read_to_string(tmp.path().join("issues/dryrun-target-x/item.md")).unwrap();
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "dryrun-target-x", req, None).unwrap();
        assert!(out.pending_serialized.is_some());
        let pending = out.pending_serialized.unwrap();
        assert!(pending.contains("priority: high"));
        let after = fs::read_to_string(tmp.path().join("issues/dryrun-target-x/item.md")).unwrap();
        assert_eq!(before, after, "dry-run must not touch disk");
    }

    #[test]
    fn toggle_checkbox_flips_unique_match() {
        let tmp = fresh_repo();
        let body = "# T\n\n## Tasks\n\n- [ ] write the parser\n- [ ] deploy script wiring\n- [x] tests passing\n";
        let _v0 = seed_with_body(tmp.path(), "checkbox-target-y", body);
        let out =
            toggle_checkbox(tmp.path(), "checkbox-target-y", "deploy", None, None, false).unwrap();
        assert!(out.pending_serialized.is_none());
        let after =
            fs::read_to_string(tmp.path().join("issues/checkbox-target-y/item.md")).unwrap();
        assert!(after.contains("- [x] deploy script wiring"));
        assert!(after.contains("- [ ] write the parser"));
    }

    #[test]
    fn toggle_checkbox_errors_on_zero_or_multiple_matches() {
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] alpha task\n- [ ] beta task\n";
        let _ = seed_with_body(tmp.path(), "checkbox-amb-z", body);
        let zero = toggle_checkbox(tmp.path(), "checkbox-amb-z", "missing", None, None, false)
            .unwrap_err();
        assert!(matches!(zero, MutateError::Validation(s) if s.contains("no checkbox")));
        let many =
            toggle_checkbox(tmp.path(), "checkbox-amb-z", "task", None, None, false).unwrap_err();
        assert!(matches!(many, MutateError::Validation(s) if s.contains("matched")));
    }

    #[test]
    fn toggle_checkbox_dry_run_does_not_write() {
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] only one\n";
        let _ = seed_with_body(tmp.path(), "checkbox-dry-q", body);
        let before =
            fs::read_to_string(tmp.path().join("issues/checkbox-dry-q/item.md")).unwrap();
        let out = toggle_checkbox(tmp.path(), "checkbox-dry-q", "only one", None, None, true)
            .unwrap();
        assert!(out.pending_serialized.is_some());
        assert!(out.pending_serialized.unwrap().contains("- [x] only one"));
        let after = fs::read_to_string(tmp.path().join("issues/checkbox-dry-q/item.md")).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn label_add_is_idempotent_under_update_issue() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "label-idem-w", "open");
        for _ in 0..2 {
            let req = UpdateIssueRequest {
                add_labels: vec!["backend".into()],
                ..Default::default()
            };
            update_issue(tmp.path(), "label-idem-w", req, None).unwrap();
        }
        let after = fs::read_to_string(tmp.path().join("issues/label-idem-w/item.md")).unwrap();
        assert_eq!(after.matches("backend").count(), 1);
    }

    #[test]
    fn apply_rolls_back_on_schema_violation() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "apply-rollback-q", "open");
        let before =
            fs::read_to_string(tmp.path().join("issues/apply-rollback-q/item.md")).unwrap();
        // Schema requires `team:` — applying a patch without it must
        // be rejected and leave disk untouched.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            add_labels: vec!["backend".into()],
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "apply-rollback-q", req, None).unwrap_err();
        assert!(matches!(err, MutateError::SchemaViolation(_)));
        let after =
            fs::read_to_string(tmp.path().join("issues/apply-rollback-q/item.md")).unwrap();
        assert_eq!(
            before, after,
            "schema violation must leave the file unchanged"
        );
    }

    #[test]
    fn note_decisions_section_appends_to_decisions() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "note-decision-p", "open");
        note_issue(
            tmp.path(),
            "note-decision-p",
            "alice",
            "go with option B",
            crate::body_sections::DECISIONS,
            None,
            None,
            false,
        )
        .unwrap();
        let after =
            fs::read_to_string(tmp.path().join("issues/note-decision-p/item.md")).unwrap();
        assert!(after.contains("## Decisions"));
        assert!(after.contains("go with option B"));
        assert!(!after.contains("## Comments"));
    }
}
