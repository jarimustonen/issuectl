//! Mutation contract shared by the CLI and the web server.
//!
//! Implements the M1 protocol from `docs/design/web-edit-sync.md` §3:
//! repo-wide `flock` → locate → read → optimistic-version check →
//! mutate in memory → atomic write (and directory rename for status
//! crossings) → recompute canonical hash → publish before releasing
//! the lock. The lock guard is RAII so panic / cancellation during
//! the sequence drops the file and releases the advisory lock.
//!
//! The CLI (`do_update`, `do_close`, `do_new`) and the axum PATCH/POST
//! handlers both call into this module so a) every writer obtains the
//! same `flock`, b) every writer emits the same canonical version
//! token, and c) the web server never has to fork a second process to
//! mutate state.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Deserializer};

use crate::canonical::canonical_hash;
use crate::models::Issue;
use crate::repo::{self, IssueSummary};
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
    /// Directory containing the issue's `item.md` after the mutation
    /// (changes when the mutation crosses `open/` ↔ `closed/`).
    pub issue_dir: PathBuf,
    pub moved_to_closed: bool,
    pub moved_to_open: bool,
}

/// Errors that the mutate layer surfaces to its callers. The CLI maps
/// these to anyhow + exit codes; the server maps them to HTTP statuses
/// in `api.rs`.
#[derive(Debug)]
pub enum MutateError {
    NotFound,
    AmbiguousSlug,
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
            MutateError::AmbiguousSlug => {
                write!(f, "slug exists in both open/ and closed/ — resolve manually")
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
        fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;
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

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;

    // 1) locate
    let (folder, item_path) = locate_for_mutation(root, slug)?;

    // 2) read + parse + hash. Refuse to mutate a corrupt file —
    // overwriting parser fallback defaults would silently destroy the
    // user's real (but malformed) on-disk content (§8.6 / M7).
    let parsed = crate::parser::parse_item_md_with_warnings(&item_path, slug, &folder);
    if !parsed.warnings.is_empty() {
        return Err(MutateError::Corrupt {
            warnings: parsed.warnings,
        });
    }
    let current_issue = parsed.issue;
    let current_version = canonical_hash(&current_issue);

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
    let mut new_folder = folder.clone();
    let mut moved_to_closed = false;
    let mut moved_to_open = false;

    // status: special-cased because closing statuses cross folders
    if let Patch::Set(s) = &req.status {
        write::set_string(&mut item.frontmatter, "status", s);
        if crate::is_closing_status(s) {
            new_folder = "closed".to_string();
            write::set_string(&mut item.frontmatter, "closed", &write::today());
            if folder == "open" {
                moved_to_closed = true;
            }
        } else if folder == "closed" {
            write::remove_key(&mut item.frontmatter, "closed");
            new_folder = "open".to_string();
            moved_to_open = true;
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
        write::add_to_string_list(&mut item.frontmatter, "labels", label).map_err(MutateError::Io)?;
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

    // 5+6) write-then-rename. The reviewers all flagged the original
    // "rename first" order (per design doc §3.4) as crash-unsafe: a
    // panic between rename and write would leave new-folder + old
    // content. Writing first means a crash between write and rename
    // leaves `status: <closing>` content at `open/<slug>/` — which
    // the startup reconciler can resolve cleanly from frontmatter
    // alone. Net: no torn states across folder/content axes.
    write_item_atomic(&item_path, &item).map_err(MutateError::Io)?;
    let final_path = if new_folder != folder {
        rename_status_dir(root, slug, &folder, &new_folder).map_err(MutateError::Io)?;
        write::issue_dir(root, &new_folder, slug).join("item.md")
    } else {
        item_path.clone()
    };

    // 7) recompute canonical hash from final on-disk content
    let after = crate::parser::parse_item_md_with_warnings(&final_path, slug, &new_folder);
    let new_issue = after.issue;
    let new_version = canonical_hash(&new_issue);

    // 8) publish while still inside the lock so seq order matches
    //    disk order. Cross-folder moves emit a single atomic
    //    `IssueMoved` event (one seq, one client-side handler) — no
    //    Remove+Upsert race window. The watcher will also fire on
    //    the rename; clients dedupe by `version`.
    if let Some(hub) = hub {
        if folder != new_folder {
            hub.publish(EventPayload::IssueMoved {
                slug: slug.to_string(),
                from_folder: folder.clone(),
                to_folder: new_folder.clone(),
                version: new_version.clone(),
                issue: Box::new(IssueSummary::from(new_issue.clone())),
            });
        } else {
            hub.publish(EventPayload::IssueUpserted {
                slug: slug.to_string(),
                version: new_version.clone(),
                issue: Box::new(IssueSummary::from(new_issue.clone())),
            });
        }
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

/// Apply a `Patch<String>` onto a frontmatter mapping. `Unspecified`
/// is a no-op; `Clear` removes the key; `Set(v)` sets the key.
fn apply_string_patch(item: &mut ItemFile, key: &str, p: &Patch<String>) {
    match p {
        Patch::Unspecified => {}
        Patch::Clear => write::remove_key(&mut item.frontmatter, key),
        Patch::Set(v) => write::set_string(&mut item.frontmatter, key, v),
    }
}

/// Locate the issue with M1's stricter ambiguous-slug semantics. The
/// existing `repo::locate_issue` short-circuits on the first match; we
/// add an explicit "both folders" check so the mutate layer can return
/// `AmbiguousSlug` instead of silently picking one side.
fn locate_for_mutation(root: &Path, slug: &str) -> Result<(String, PathBuf), MutateError> {
    let issues = root.join("issues");
    let open_dir = issues.join("open").join(slug);
    let closed_dir = issues.join("closed").join(slug);
    let open_exists = real_dir(&open_dir);
    let closed_exists = real_dir(&closed_dir);
    if open_exists && closed_exists {
        return Err(MutateError::AmbiguousSlug);
    }
    match repo::locate_issue(root, slug) {
        Ok((folder, item_path)) => Ok((folder, item_path)),
        Err(_) => Err(MutateError::NotFound),
    }
}

fn real_dir(p: &Path) -> bool {
    match fs::symlink_metadata(p) {
        Ok(m) => m.is_dir() && !m.file_type().is_symlink(),
        Err(_) => false,
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

/// Rename `<root>/issues/<from>/<slug>` to `<root>/issues/<to>/<slug>`.
/// On all platforms we pre-check that the target does not exist (a
/// TOCTOU window which is acceptable: only writers under our `flock`
/// can race with us; uncoordinated writers are out of scope per §3).
fn rename_status_dir(root: &Path, slug: &str, from: &str, to: &str) -> Result<()> {
    let old = write::issue_dir(root, from, slug);
    let new = write::issue_dir(root, to, slug);
    if let Some(parent) = new.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    if new.exists() {
        bail!("target directory already exists: {}", new.display());
    }
    fs::rename(&old, &new)
        .with_context(|| format!("cannot rename {} → {}", old.display(), new.display()))?;
    #[cfg(unix)]
    {
        if let Some(p) = old.parent() {
            let _ = fsync_dir(p);
        }
        if let Some(p) = new.parent() {
            let _ = fsync_dir(p);
        }
    }
    Ok(())
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

    // `do_new` itself acquires the WriteLock (M1 contract: every
    // writer holds the repo flock). Don't double-acquire here.
    let outcome = crate::do_new(
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

    // Re-read for canonical hash + Issue.
    let parsed = crate::parser::parse_item_md_with_warnings(&outcome.item_path, &outcome.slug, "open");
    let issue = parsed.issue;
    let version = canonical_hash(&issue);

    if let Some(hub) = hub {
        hub.publish(EventPayload::IssueUpserted {
            slug: outcome.slug.clone(),
            version: version.clone(),
            issue: Box::new(IssueSummary::from(issue.clone())),
        });
    }

    Ok(NewOutcome {
        issue_dir: outcome.item_path.parent().unwrap().to_path_buf(),
        issue,
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fresh_repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        fs::create_dir_all(tmp.path().join("issues/closed")).unwrap();
        tmp
    }

    fn seed_issue(root: &Path, folder: &str, slug: &str, status: &str) -> String {
        let dir = root.join("issues").join(folder).join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            format!(
                "---\ntype: bug\ncreated: 2026-05-06\nstatus: {status}\npriority: normal\n---\n\n# Title\n",
            ),
        )
        .unwrap();
        let parsed = crate::parser::parse_item_md_with_warnings(
            &dir.join("item.md"),
            slug,
            folder,
        );
        canonical_hash(&parsed.issue)
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
    fn update_status_to_closing_renames_directory() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "close-me-now", "open");
        let req = UpdateIssueRequest {
            status: Patch::Set("fixed".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "close-me-now", req, None).unwrap();
        assert!(out.moved_to_closed);
        assert!(out
            .issue_dir
            .to_string_lossy()
            .contains("/closed/close-me-now"));
        assert!(!tmp.path().join("issues/open/close-me-now").exists());
    }

    #[test]
    fn patch_clear_removes_field() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/has-epic-here");
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
        let dir = tmp.path().join("issues/open/keep-epic-as-is");
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
        let result: Result<UpdateIssueRequest, _> =
            serde_json::from_str(r#"{"priorty": "high"}"#);
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
