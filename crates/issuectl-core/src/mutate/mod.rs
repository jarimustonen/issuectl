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

pub mod archive;
pub mod attach;
pub mod intake;
pub mod new_issue;
pub mod triage;

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Deserializer};

use crate::canonical::canonical_hash;
use crate::models::Issue;
use crate::repo::{self, folder_for_status, IssueSummary};
use crate::repo_config::ConfigSource;
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
    /// Issue type (`bug`, `feature`, `task`, ...). `Set` only — clearing
    /// is rejected (every issue must have a type). When the new value
    /// actually differs from the current type, three additional
    /// post-mutation checks fire (option 2 of three; see AGENTS.md):
    /// (a) reject if combined with a close→open reopen on the same
    /// call, (b) reject if `epic` is paired with assignee/reporter or
    /// a non-epic type is paired with owner, (c) reject with
    /// `MutateError::SchemaViolation` if the new type's schema-required
    /// body sections are missing — naming each missing heading. Same-
    /// value sets are a true no-op so idempotent clients don't trip
    /// the checks. The CLI mirrors this via `--type`.
    #[serde(default, rename = "type")]
    pub issue_type: Patch<String>,
    #[serde(default)]
    pub priority: Patch<String>,
    #[serde(default)]
    pub assignee: Patch<String>,
    #[serde(default)]
    pub owner: Patch<String>,
    #[serde(default)]
    pub epic: Patch<String>,
    /// Closer attribution, managed in lockstep with `closed:`. `Set`
    /// stamps `closed_by:` on the active→closing edge (and re-attributes
    /// on a closing→closing re-status); reopening (closing→active) clears
    /// it. Populated by `close --as <author>`; a raw PATCH may also set it
    /// on a closing transition. Not writable through `custom_fields` — the
    /// key is reserved (see [`RESERVED_CUSTOM_FIELD_KEYS`]) so the only
    /// way in is this validated slot, which enforces the same author
    /// grammar as `note --as`.
    #[serde(default)]
    pub closed_by: Patch<String>,
    #[serde(default)]
    pub add_labels: Vec<String>,
    #[serde(default)]
    pub remove_labels: Vec<String>,
    #[serde(default)]
    pub add_related: Vec<String>,
    #[serde(default)]
    pub remove_related: Vec<String>,
    /// `blocked_by:` list operations. Mirrors `add_related`/`remove_related`
    /// — the value type is a slug (with or without the `@` sigil) that
    /// the under-lock path normalizes via `refs::normalize_related_refs`.
    /// Driven by `issuectl depend add/remove <slug> --blocked-by <other>`.
    /// The reverse `blocks` edge is intentionally not stored: it's
    /// derived at read time from every issue's `blocked_by` array.
    #[serde(default)]
    pub add_blocked_by: Vec<String>,
    #[serde(default)]
    pub remove_blocked_by: Vec<String>,
    #[serde(default)]
    pub add_commits: Vec<CommitSpec>,
    /// Per-key custom-frontmatter PATCH. Mirrors the top-level `Patch`
    /// ternary: omitted (no entry) leaves the key alone; `null` removes
    /// the key; a string sets it. Built-in keys (`status`, `priority`,
    /// dates, etc.) are reserved here — use the dedicated request slots.
    ///
    /// Duplicate keys in the wire payload are rejected during
    /// deserialization, mirroring `NewIssueRequest::custom_fields` so
    /// `PATCH /api/issues/<slug>` enforces the same invariant the CLI
    /// `--field foo=a --field foo=b` rejection enforces — without this
    /// gate `serde_json` silently keeps whichever value the parser
    /// happens to see last.
    #[serde(default, deserialize_with = "deserialize_patch_map_no_dups")]
    pub custom_fields: std::collections::BTreeMap<String, Patch<String>>,
    /// In-order body mutations applied under the same flock as the
    /// frontmatter PATCHes above. Each op is one of `append_note` or
    /// `set_checkbox`; ops apply in vector order so a patch can
    /// (e.g.) append a note that documents a checkbox flip happening
    /// right after it. Schema + transition validation runs once on
    /// the post-body state, matching the all-or-nothing contract of
    /// the rest of `UpdateIssueRequest`. Capped at `MAX_BODY_OPS` so
    /// a runaway agent or hostile PATCH can't pin the repo-wide flock
    /// scanning the body 50k times in one request.
    #[serde(default)]
    pub body_ops: Vec<BodyOp>,
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
    // List-typed built-ins. The hint leads with the mutate-existing
    // flags (`--add-*`/`--remove-*`, exposed by `update`/`bulk`) because
    // the failing paths are `set`/`update` on an existing issue; the
    // create-time `--label`/`--related` form is named second. Both forms
    // are real flags that work verbatim in their stated context.
    (
        "labels",
        "list-typed: use `update --add-label` / `--remove-label` (or `--label` at `new`)",
    ),
    (
        "related",
        "list-typed: use `update --add-related` / `--remove-related` (or `--related` at `new`)",
    ),
    (
        "blocked_by",
        "use `issuectl depend add/remove <slug> --blocked-by <other>`",
    ),
    (
        "closed_by",
        "set automatically by `close --as <author>`; cleared on reopen",
    ),
    ("status", "set automatically by `new` (always `open`)"),
    ("created", "set automatically by `new` (today)"),
    ("updated", "set automatically by `new`/`update` (today)"),
    (
        "closed",
        "set automatically when status moves to a closing value",
    ),
    (
        "commits",
        "use `update --add-commit` (or commit trailers + `sync-commits`) after creation",
    ),
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

/// Shared per-key predicate for all four create/update paths: rejects
/// invalid key shape and reserved built-in names. Returns the formatted
/// message so each call site wraps it in the typed error variant that
/// matches its return type (`MutateError::Validation` /
/// `DoNewError::Validation`).
///
/// Routed through:
/// - clap parsers `parse_custom_field` / `parse_custom_field_key` (CLI new + update)
/// - `UpdateIssueRequest::validate` (API update)
/// - `do_new_locked` (API new — primary defense; CLI new is already covered by the parser)
///
/// "Shared" applies to *predicates*, not necessarily to every error
/// string: the CLI parsers also do a separate leading/trailing-whitespace
/// pre-check whose wording stays parser-local.
pub fn validate_custom_field_key(key: &str) -> Result<(), String> {
    if !is_valid_custom_field_key(key) {
        return Err(format!(
            "custom field key {key:?} must be alphanumeric / underscore / hyphen"
        ));
    }
    if let Some(hint) = reserved_custom_field_hint(key) {
        return Err(format!("custom field {key:?} is built-in: {hint}"));
    }
    Ok(())
}

/// Shared per-value predicate. A `Set` value must be non-empty after
/// trimming AND must equal its trim — leading or trailing whitespace is
/// rejected, not silently stripped, so the on-disk frontmatter matches
/// what the caller asked for. Mirrors the CLI parser's
/// `parse_custom_field` post-`=` value check so all four create/update
/// paths converge on the same value contract (closes the gap where API
/// new previously accepted `{"team": "   "}` while CLI new and API
/// update rejected it).
pub fn validate_custom_field_value(key: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!(
            "custom field {key:?}: empty-string Set is not allowed (use null to clear)"
        ));
    }
    if value.trim() != value {
        return Err(format!(
            "custom field {key:?}: leading or trailing whitespace is not allowed"
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct CommitSpec {
    pub hash: String,
    pub summary: String,
}

/// Hard cap on the number of body operations one mutation request can
/// carry. Each `set_checkbox` rescans the entire body (`O(body_lines)`),
/// so an unbounded vector lets a single PATCH pin the repo-wide flock
/// long enough to starve every other writer. The cap is enforced in
/// `UpdateIssueRequest::validate` so it fires before any disk I/O.
pub const MAX_BODY_OPS: usize = 64;

/// Body-mutation operation carried inside an `UpdateIssueRequest`.
/// Externally tagged so `apply` patch.yaml entries read like
/// `- set_checkbox: { match: "needle", checked: true }` and
/// `- append_note: { ... }`. JSON clients send the same shape
/// (`{"set_checkbox": {...}}`). The wire shape is enforced by a
/// hand-rolled `MapAccess` visitor (see `Deserialize` impl) so unknown
/// keys, multi-key entries, and null-sibling-key bypasses are all
/// rejected explicitly.
#[derive(Debug, Clone)]
pub enum BodyOp {
    /// Drive the unique `- [ ]` / `- [x]` line whose text contains the
    /// needle to a target state. Idempotent — if the line is already
    /// in the requested state the op is a no-op for that body, which
    /// lets agents retry timed-out requests safely. Errors when zero
    /// or multiple lines match.
    SetCheckbox(SetCheckboxOp),
    /// Append a timestamped block to the named section, creating the
    /// section if missing. Same shape as `cmd_note`.
    AppendNote(AppendNoteOp),
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SetCheckboxOp {
    /// Substring matched against checkbox lines (fence-aware). Must
    /// uniquely identify one `- [ ]` / `- [x]` line.
    #[serde(rename = "match")]
    pub match_substring: String,
    /// Target state. `true` ticks the box (or leaves it ticked);
    /// `false` clears it (or leaves it cleared).
    pub checked: bool,
}

// Manual external-tag deserialize: serde_yaml does not accept the
// derive-based external-tag shape without `!Tag` directives, and the
// previous helper-struct version (`#[derive(Deserialize)] struct Wire
// { toggle_checkbox: Option<_>, append_note: Option<_> }`) silently
// dropped unknown keys and accepted null-sibling-key bypasses. The
// `MapAccess` visitor enforces:
//   - exactly one variant key per entry;
//   - unknown keys rejected with the canonical `unknown_field` shape;
//   - null-valued sibling keys rejected explicitly (the previous
//     `Option<T>` collapsed `null` to `None` and let through
//     `{"set_checkbox": {...}, "append_note": null}`).
// `serde_json` and `serde_yaml` both read the same single-key-mapping
// shape through this visitor.
impl<'de> Deserialize<'de> for BodyOp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{Error as DeError, IgnoredAny, MapAccess, Visitor};
        use std::fmt;

        const VARIANTS: &[&str] = &["set_checkbox", "append_note"];

        struct BodyOpVisitor;
        impl<'de> Visitor<'de> for BodyOpVisitor {
            type Value = BodyOp;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a single-key mapping: \
                     {set_checkbox: {match, checked}} or \
                     {append_note: {author, message, section?}}",
                )
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<BodyOp, M::Error> {
                let Some(key) = map.next_key::<String>()? else {
                    return Err(M::Error::custom(
                        "body_ops entry must declare exactly one operation",
                    ));
                };
                let op = match key.as_str() {
                    "set_checkbox" => BodyOp::SetCheckbox(map.next_value::<SetCheckboxOp>()?),
                    "append_note" => BodyOp::AppendNote(map.next_value::<AppendNoteOp>()?),
                    other => return Err(M::Error::unknown_field(other, VARIANTS)),
                };
                if let Some(extra) = map.next_key::<String>()? {
                    // Consume the value before erroring so the
                    // deserializer state stays valid for callers
                    // walking multiple entries; reject regardless of
                    // whether the value is null (the previous
                    // `Option<T>` helper accepted `null` siblings,
                    // which let multi-key entries slip through).
                    let _: IgnoredAny = map.next_value()?;
                    return Err(M::Error::custom(format!(
                        "body_ops entry must be a single-key mapping; \
                         unexpected extra key {extra:?} (valid keys: {VARIANTS:?})"
                    )));
                }
                Ok(op)
            }
        }

        d.deserialize_map(BodyOpVisitor)
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct AppendNoteOp {
    pub author: String,
    pub message: String,
    #[serde(default)]
    pub section: NoteSection,
}

#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NoteSection {
    #[default]
    Comments,
    Decisions,
    AgentRuns,
}

impl NoteSection {
    fn as_str(self) -> &'static str {
        match self {
            NoteSection::Comments => crate::body_sections::COMMENTS,
            NoteSection::Decisions => crate::body_sections::DECISIONS,
            NoteSection::AgentRuns => crate::body_sections::AGENT_RUNS,
        }
    }
}

impl UpdateIssueRequest {
    /// True when no field would actually change on disk — every patch
    /// slot is `Unspecified` and every list/commit collection is empty.
    /// `expected_version` is *not* a mutation; an empty body with only a
    /// version token is still a no-op (M13).
    pub fn is_noop(&self) -> bool {
        matches!(self.status, Patch::Unspecified)
            && matches!(self.issue_type, Patch::Unspecified)
            && matches!(self.priority, Patch::Unspecified)
            && matches!(self.assignee, Patch::Unspecified)
            && matches!(self.owner, Patch::Unspecified)
            && matches!(self.epic, Patch::Unspecified)
            && matches!(self.closed_by, Patch::Unspecified)
            && self.add_labels.is_empty()
            && self.remove_labels.is_empty()
            && self.add_related.is_empty()
            && self.remove_related.is_empty()
            && self.add_blocked_by.is_empty()
            && self.remove_blocked_by.is_empty()
            && self.add_commits.is_empty()
            && self.custom_fields.is_empty()
            && self.body_ops.is_empty()
    }

    /// Reject empty-string Sets, type-set vs enum mismatches, and
    /// add_X/remove_X intent collisions. Runs once after both serde
    /// and clap have produced the request.
    ///
    /// Runs *before* the lock + schema-bootstrap + legacy-migration
    /// path so request-level validation failures don't leak side
    /// effects (a `status: null` patch used to migrate legacy issues
    /// and create `.schema.yaml` before erroring out — round-2 #1).
    pub fn validate(&self) -> Result<(), MutateError> {
        // `status: null` / `--clear` is rejected here rather than
        // deeper in `update_issue_under_lock` so the rejection short-
        // circuits before any disk side effect. The under-lock check
        // remains as defence in depth.
        if matches!(self.status, Patch::Clear) {
            return Err(MutateError::Validation(
                "status cannot be cleared (issues always have a status)".into(),
            ));
        }
        if matches!(self.issue_type, Patch::Clear) {
            return Err(MutateError::Validation(
                "type cannot be cleared (issues always have a type)".into(),
            ));
        }
        check_set_nonempty("status", &self.status)?;
        check_set_nonempty("type", &self.issue_type)?;
        // No `crate::issue_fields::ISSUE_TYPES` membership check here: the schema
        // (`fields.type.enum`) is the source of truth for allowed
        // values, and a custom schema may declare additional types
        // (e.g. `spike`). Validation runs in step 4b under lock against
        // the post-mutation frontmatter; that's the right layer.
        check_set_nonempty("priority", &self.priority)?;
        check_set_nonempty("assignee", &self.assignee)?;
        check_set_nonempty("owner", &self.owner)?;
        check_set_nonempty("epic", &self.epic)?;
        check_set_nonempty("closed_by", &self.closed_by)?;
        // Closer attribution follows the same author grammar as
        // `note --as`, so the recorded value is a well-formed,
        // hash-stable token regardless of entry point (CLI `close --as`
        // or a raw PATCH populating the slot).
        if let Patch::Set(author) = &self.closed_by {
            crate::body_sections::validate_author(author)
                .map_err(|e| MutateError::Validation(format!("closed_by: {e}")))?;
        }

        // No built-in `all_statuses()` membership check here, mirroring
        // the `type` policy above: the schema (`fields.status.enum`) is
        // the source of truth, and a project may legitimately add
        // `archived` (or similar) to its enum. Schema validation in
        // step 4b runs under lock against the post-mutation
        // frontmatter — wrong statuses fail there with a clearer
        // message that lists the actual allowed set.
        if let Patch::Set(s) = &self.status {
            if s.is_empty() {
                return Err(MutateError::Validation(
                    "status cannot be empty (use Patch::Clear to remove)".into(),
                ));
            }
        }
        if let Patch::Set(p) = &self.priority {
            if !crate::issue_fields::PRIORITIES.iter().any(|v| v == p) {
                return Err(MutateError::Validation(format!(
                    "priority {p:?} is not one of the known priorities"
                )));
            }
        }
        if let Patch::Set(e) = &self.epic {
            if !crate::slug::is_valid(e) {
                return Err(MutateError::Validation(format!(
                    "epic {e:?} is not a valid slug (lowercase ASCII, kebab-case, ≥2 segments)"
                )));
            }
        }

        for (name, list) in [
            ("add_labels", &self.add_labels),
            ("remove_labels", &self.remove_labels),
            ("add_related", &self.add_related),
            ("remove_related", &self.remove_related),
            ("add_blocked_by", &self.add_blocked_by),
            ("remove_blocked_by", &self.remove_blocked_by),
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
        if let Some(overlap) = first_overlap(&self.add_blocked_by, &self.remove_blocked_by) {
            return Err(MutateError::ConflictingIntent(format!(
                "blocked_by ref {overlap:?} appears in both add_blocked_by and remove_blocked_by"
            )));
        }

        for (key, patch) in &self.custom_fields {
            validate_custom_field_key(key).map_err(MutateError::Validation)?;
            if let Patch::Set(v) = patch {
                validate_custom_field_value(key, v).map_err(MutateError::Validation)?;
            }
        }

        if self.body_ops.len() > MAX_BODY_OPS {
            return Err(MutateError::Validation(format!(
                "body_ops length {} exceeds maximum {MAX_BODY_OPS}; \
                 split into multiple `apply` calls",
                self.body_ops.len()
            )));
        }
        for (i, op) in self.body_ops.iter().enumerate() {
            match op {
                BodyOp::SetCheckbox(set) => {
                    if set.match_substring.trim().is_empty() {
                        return Err(MutateError::Validation(format!(
                            "body_ops[{i}].match: set_checkbox match cannot be empty"
                        )));
                    }
                }
                BodyOp::AppendNote(note) => {
                    crate::body_sections::validate_author(&note.author).map_err(|e| {
                        MutateError::Validation(format!("body_ops[{i}].author: {e}"))
                    })?;
                    crate::body_sections::validate_message(&note.message).map_err(|e| {
                        MutateError::Validation(format!("body_ops[{i}].message: {e}"))
                    })?;
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
    /// Raw pre-mutation `item.md` bytes captured under the same flock
    /// as `pending_serialized`. Lets the CLI render a dry-run diff
    /// against the same state the mutation planned against (no race
    /// with concurrent writers), and faithfully shows YAML
    /// normalization the real write would also apply (e.g. dropped
    /// comments, scalar-style changes, key reordering) — using
    /// `serialize_item(&item)` here would silently hide those
    /// destructive normalizations from the dry-run preview.
    /// `None` for real writes (the diff is the final on-disk state,
    /// not a plan).
    pub before_serialized: Option<String>,
    /// Non-fatal advisories surfaced to the caller. The standalone
    /// body-mutation verbs (`cmd_note`, `cmd_check`) detect transition-
    /// rule violations and emit them here rather than refusing the
    /// write — the CLI prints them to stderr, the JSON envelope adds
    /// a `warnings` key. The frontmatter PATCH path never warns; rule
    /// violations there are hard errors via `MutateError::TransitionViolation`.
    pub warnings: Vec<String>,
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
    /// `.issuectl/transitions.yaml` is malformed or rejected at load
    /// time. Same 5xx reasoning as `SchemaConfig` but distinct so the
    /// API client can route the operator to the correct file.
    TransitionConfig(String),
    /// Post-mutation issue violates the declarative status-transition
    /// rules in `.issuectl/transitions.yaml` (e.g. `done` requires
    /// assignee, or transition `open` → `done` is forbidden). Mapped
    /// to 422 — client-actionable by adjusting the request.
    TransitionViolation(String),
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
            MutateError::TransitionConfig(s) => write!(f, "transition config: {s}"),
            MutateError::TransitionViolation(s) => write!(f, "transition: {s}"),
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
/// `acquire(root)` and released on `Drop` (panic-safe). Carries the
/// canonical repo root so callers (e.g. `migrate_layout`) can verify
/// that the lock protects the repo they are about to mutate.
pub struct WriteLock {
    _file: File,
    canonical_root: PathBuf,
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
        let canonical_root = fs::canonicalize(root)
            .with_context(|| format!("cannot canonicalize repo root {}", root.display()))?;
        Ok(WriteLock {
            _file: f,
            canonical_root,
        })
    }

    /// Canonical path of the repo root this lock was acquired against.
    /// Used by mutation helpers to verify the lock protects the repo
    /// they are about to write — see e.g.
    /// `migrate_layout::execute_migrate_layout_plan`.
    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
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
    config: &dyn ConfigSource,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }

    // Normalize related-ref shapes BEFORE validate() so a typo'd ref
    // like `add_related: ["123"]` + `remove_related: ["#123"]`
    // (which both normalize to `#123`) is caught by the overlap check.
    let normalized_add_related = crate::refs::normalize_related_refs(&req.add_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let normalized_remove_related = crate::refs::normalize_related_refs(&req.remove_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let normalized_add_blocked_by = crate::refs::normalize_related_refs(&req.add_blocked_by)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let normalized_remove_blocked_by = crate::refs::normalize_related_refs(&req.remove_blocked_by)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    // Reject self-blockers up front. Doctor flags them too, but the
    // mutation API is the authoring surface — failing here keeps
    // `issuectl depend add foo --blocked-by foo` from producing a
    // file the next `doctor` run will immediately complain about.
    if normalized_add_blocked_by
        .iter()
        .any(|s| s.trim_start_matches('@') == slug)
    {
        return Err(MutateError::Validation(format!(
            "issue {slug:?} cannot block itself (blocked_by must reference a different slug)"
        )));
    }
    let mut req_normalized = req;
    req_normalized.add_related = normalized_add_related.clone();
    req_normalized.remove_related = normalized_remove_related.clone();
    req_normalized.add_blocked_by = normalized_add_blocked_by.clone();
    req_normalized.remove_blocked_by = normalized_remove_blocked_by.clone();
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
        let schema = config
            .schema(root)
            .map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
        issue.folder = folder_for_status(&schema, &issue.status).to_string();
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
            before_serialized: None,
            warnings: Vec::new(),
        });
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    // Locate without migrating, regardless of dry_run. The legacy →
    // flat directory rename and the default-`.schema.yaml` *bootstrap*
    // used to run *before* `update_issue_under_lock`, which meant a
    // body op (or any other validation failure) could roll the issue's
    // content back while leaving `.schema.yaml` newly created and the
    // legacy directory permanently moved — directly contradicting the
    // documented "all-or-nothing under one flock" contract. We now
    // defer both side effects until validation has passed
    // (`update_issue_under_lock` runs them just before the atomic
    // write). Schema *load* and transition-rules load still happen
    // here so that a malformed config fails fast before any work is
    // attempted; those two paths produce typed errors
    // (`SchemaConfig` / `TransitionConfig`) and never write to disk.
    let item_path = locate_for_dry_run(root, slug)?;
    let schema = config
        .schema(root)
        .map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let rules = load_validated_rules(root, &schema, config)?;
    update_issue_under_lock(root, slug, item_path, req, hub, &schema, &rules)
}

/// Load `.issuectl/transitions.yaml` and cross-validate every status
/// it mentions against the schema's `status` enum. Typo'd status
/// names silently fail open (rule no-ops or denies everything), so
/// failing fast here gives the operator a precise pointer. All write
/// paths route through this helper.
fn load_validated_rules(
    root: &Path,
    schema: &crate::schema::Schema,
    config: &dyn ConfigSource,
) -> Result<std::sync::Arc<crate::transitions::TransitionRules>, MutateError> {
    let rules = config
        .rules(root)
        .map_err(|e| MutateError::TransitionConfig(format!("{e:#}")))?;
    let universe = crate::schema::status_universe(schema);
    crate::transitions::validate_status_refs(&rules, &universe)
        .map_err(|e| MutateError::TransitionConfig(format!("{e:#}")))?;
    Ok(rules)
}

/// Project the in-flight `ItemFile` into the canonical `Issue` shape
/// the rules engine consumes. Serializes the item to the same byte
/// layout `write_item_atomic` would produce, then runs it through
/// `parser::parse_item_md_text_with_warnings`. This guarantees the
/// post-mutation projection cannot drift from the canonical reader
/// (the alternative — a hand-rolled subset parser — silently diverges
/// every time `parser` learns to normalise a new field). Cost is one
/// serialize + one parse per validated mutation.
fn projected_issue_for_rules(
    slug: &str,
    item: &write::ItemFile,
    item_path: &Path,
    schema: &crate::schema::Schema,
) -> Result<Issue, MutateError> {
    let text = write::serialize_item(item).map_err(MutateError::Io)?;
    let parsed = crate::parser::parse_item_md_text_with_warnings(&text, slug, "open", item_path);
    if !parsed.warnings.is_empty() {
        return Err(MutateError::Corrupt {
            warnings: parsed.warnings,
        });
    }
    let mut issue = parsed.issue;
    issue.folder = folder_for_status(schema, &issue.status).to_string();
    Ok(issue)
}

/// Body of `update_issue` that runs with the flock already held. Used
/// by `close_issue` to read+decide+mutate atomically without
/// double-acquiring the lock (which deadlocks on Linux because fs2's
/// advisory lock is per-fd).
///
/// `root` is threaded in (rather than derived from `item_path`) so the
/// dry-run branch can predict the *flat* `issue_dir` even when the
/// issue currently lives at a legacy path — a real write would migrate
/// it to flat layout, and the JSON envelope's `final_dir` must agree
/// (round-2 #3).
fn update_issue_under_lock(
    root: &Path,
    slug: &str,
    item_path: PathBuf,
    req: UpdateIssueRequest,
    hub: Option<&Arc<EventHub>>,
    schema: &crate::schema::Schema,
    rules: &crate::transitions::TransitionRules,
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
    current_issue.folder = folder_for_status(schema, &current_issue.status).to_string();
    let current_version = canonical_hash(&current_issue);
    let prev_status = current_issue.status.clone();
    let prev_type = current_issue.issue_type.clone();

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
    // Capture the canonicalised pre-mutation bytes before any in-memory
    // edit. Done under the held flock so the dry-run diff is against
    // the same state the mutation planned against — a concurrent
    // writer can't slip a different "before" into the diff.
    let before_serialized =
        if req.dry_run {
            // Raw on-disk bytes — not `serialize_item(&item)`. The
            // canonicalised re-serialization would mask formatting
            // changes (dropped YAML comments, key reordering, scalar-
            // style shifts) that the real write would also apply,
            // making the dry-run preview lie about disk impact (round-2 #2).
            Some(fs::read_to_string(&item_path).map_err(|e| {
                MutateError::Io(anyhow!("cannot read {}: {e}", item_path.display()))
            })?)
        } else {
            None
        };
    let mut moved_to_closed = false;
    let mut moved_to_open = false;

    // Frontmatter keys this mutation actually writes. Threaded into
    // `hard_schema_failure` so a `RequiredWhen` produced by this very
    // write (e.g. clearing `closed:` on a closing-status issue) is
    // rejected, while a pre-existing inconsistency on an untouched field
    // stays exempt (doctor heals those). NOTE: any new frontmatter write
    // added below must record its key here, or a violation it introduces
    // will be silently dropped.
    let mut written: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // Status change is a pure frontmatter PATCH; no directory rename
    // (post-flat-layout). The `moved_to_*` booleans now track the
    // active↔closing transition for messaging parity with the old API.
    if let Patch::Set(s) = &req.status {
        write::set_string(&mut item.frontmatter, "status", s);
        // The status branch always (re)evaluates the two close-lifecycle
        // fields — `closed:` and `closed_by:` — stamping, backfilling, or
        // removing each, so all three keys count as written.
        written.insert("status".into());
        written.insert("closed".into());
        written.insert("closed_by".into());
        let prev_closing = crate::schema::is_closing(schema, &prev_status);
        let new_closing = crate::schema::is_closing(schema, s);
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
            // `closed_by:` tracks `closed:`. An explicit attribution
            // (`close --as`, or a PATCH populating the slot) is written /
            // re-attributed. Without one, the active→closing edge scrubs
            // any stray value so an anonymous close never inherits a
            // stale closer, while a closing→closing re-status preserves
            // the recorded closer for the same provenance reason as the
            // `closed:` date above.
            match &req.closed_by {
                Patch::Set(author) => write::set_string(&mut item.frontmatter, "closed_by", author),
                Patch::Clear => write::remove_key(&mut item.frontmatter, "closed_by"),
                Patch::Unspecified => {
                    if !prev_closing {
                        write::remove_key(&mut item.frontmatter, "closed_by");
                    }
                }
            }
            if !prev_closing {
                moved_to_closed = true;
            }
        } else {
            write::remove_key(&mut item.frontmatter, "closed");
            // Closer attribution is close-time provenance; on reopen (or
            // any active status) it is stale, so drop it in lockstep with
            // `closed:`. Because `closed_by` is a reserved key, the only
            // writers are this lifecycle branch and the validated
            // request slot — so clearing here on the shared active edge
            // is authoritative and can't be re-added by a later
            // custom-field patch in the same call.
            write::remove_key(&mut item.frontmatter, "closed_by");
            if prev_closing {
                moved_to_open = true;
            }
        }
    } else if let Patch::Clear = &req.status {
        return Err(MutateError::Validation(
            "status cannot be cleared (issues always have a status)".into(),
        ));
    }

    if let Patch::Set(t) = &req.issue_type {
        write::set_string(&mut item.frontmatter, "type", t);
        written.insert("type".into());
    }
    for (key, patch) in [
        ("priority", &req.priority),
        ("assignee", &req.assignee),
        ("owner", &req.owner),
        ("epic", &req.epic),
    ] {
        apply_string_patch(&mut item, key, patch);
        if !matches!(patch, Patch::Unspecified) {
            written.insert(key.into());
        }
    }

    if !req.add_labels.is_empty() || !req.remove_labels.is_empty() {
        written.insert("labels".into());
    }
    for label in &req.add_labels {
        write::add_to_string_list(&mut item.frontmatter, "labels", label)
            .map_err(MutateError::Io)?;
    }
    for label in &req.remove_labels {
        write::remove_from_string_list(&mut item.frontmatter, "labels", label)
            .map_err(MutateError::Io)?;
    }

    // related refs were normalized before validate(), so use them as-is.
    if !req.add_related.is_empty() || !req.remove_related.is_empty() {
        written.insert("related".into());
    }
    for r in &req.add_related {
        write::add_to_string_list(&mut item.frontmatter, "related", r).map_err(MutateError::Io)?;
    }
    for r in &req.remove_related {
        write::remove_from_string_list(&mut item.frontmatter, "related", r)
            .map_err(MutateError::Io)?;
    }

    // blocked_by: same shape contract as `related`. Normalization
    // already ran in `update_issue` so the list elements are bare
    // slugs by the time we get here.
    if !req.add_blocked_by.is_empty() || !req.remove_blocked_by.is_empty() {
        written.insert("blocked_by".into());
    }
    for r in &req.add_blocked_by {
        write::add_to_string_list(&mut item.frontmatter, "blocked_by", r)
            .map_err(MutateError::Io)?;
    }
    for r in &req.remove_blocked_by {
        write::remove_from_string_list(&mut item.frontmatter, "blocked_by", r)
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
            Patch::Clear => {
                write::remove_key(&mut item.frontmatter, key);
                written.insert(key.clone());
            }
            Patch::Set(v) => {
                write::set_string(&mut item.frontmatter, key, v);
                written.insert(key.clone());
            }
        }
    }

    write::set_string(&mut item.frontmatter, "updated", &write::today());

    // Reopen flow: when transitioning closing → active, append a
    // `## Reopen Notes — <date>` section so the rationale isn't
    // implicit. One section per transition (multiple reopens stack).
    if moved_to_open {
        let trimmed_body = item.body.trim_start_matches('\n');
        let with_section = crate::body_sections::append_reopen_notes(trimmed_body, &write::today());
        item.body = crate::body_sections::canonicalise_body_leading(&with_section);
    }

    // Type-change rules. Only fire when `--type` is set AND the new
    // value actually differs from the current type — same-value sets
    // are a true no-op so idempotent JSON clients don't trip the
    // checks below.
    if let Patch::Set(new_type) = &req.issue_type {
        if new_type != &prev_type {
            // C4: forbid combining a close→open reopen with `--type`.
            // Both are body-mutating in different ways; bundling them
            // makes the resulting document harder to reason about and
            // is a rare combination in practice. Splitting into two
            // calls is the path forward.
            if moved_to_open {
                return Err(MutateError::Validation(
                    "cannot change --type while reopening (close→open) in the same call; \
                     run --status open first, then --type as a separate call"
                        .into(),
                ));
            }
            // D1: epic↔non-epic frontmatter invariants. Mirrors
            // `cmd_new`'s rule (`epic` uses `--owner` not assignee /
            // reporter; non-epic types use neither). The user has to
            // clear the offending frontmatter field manually first —
            // CLI flags like `--no-assignee` don't exist (only
            // `--no-epic`), so the error message points the user at a
            // concrete next step rather than auto-clearing.
            if let Err(e) = check_type_invariants(new_type, &item.frontmatter) {
                return Err(e);
            }
            // C2: option 2 — reject when the new type's required body
            // sections aren't already present. Empty stubs would pass
            // `doctor` while the content is semantically blank, which
            // is worse than a clear error message (especially for AI
            // agents whose retry loop is well-defined here: edit body,
            // resubmit). Schema is fence-aware via
            // `body_sections::all_h2_sections`.
            let missing = crate::schema::missing_body_sections(
                schema,
                new_type,
                item.body.trim_start_matches('\n'),
            );
            if !missing.is_empty() {
                return Err(MutateError::SchemaViolation(format!(
                    "type {new_type:?} requires body sections that are missing: {}; \
                     add the section headings to the body first, then re-run --type",
                    missing
                        .iter()
                        .map(|s| format!("## {s}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
    }

    // Body ops apply in vector order so a patch's narrative reads top-
    // to-bottom: the user can toggle a checkbox and append a note that
    // refers to the toggle, all under the single flock above. Schema
    // and transition validation below run on the post-body state, so a
    // failing op rolls back the entire transaction (the in-memory
    // `item` is dropped without writing).
    for (i, op) in req.body_ops.iter().enumerate() {
        apply_body_op(&mut item, i, op)?;
    }

    // 4b) schema validation against the post-mutation frontmatter. The
    //     built-in clap parsers already guard known enums; this layer
    //     enforces user-declared required fields and custom enums
    //     (e.g. a constrained `labels` enum). Schema is loaded once by
    //     the caller and threaded in so we don't re-read the file on
    //     each mutation.
    let violations = crate::schema::validate(schema, &item.frontmatter);
    if let Some(msg) = hard_schema_failure(&violations, &written) {
        return Err(MutateError::SchemaViolation(msg));
    }
    // Belt-and-braces status check. `schema::validate` only flags
    // out-of-enum values when `fields.status.enum` is declared — but
    // the schema's whole-spec replacement semantics let a user redeclare
    // `fields.status` without `enum:`, which would otherwise let any
    // string land here and silently default-classify as Active.
    // `status_universe()` falls back to the built-in `all_statuses()`
    // list in that no-enum case, so a typo can't sneak past.
    if let Some(status) = item
        .frontmatter
        .get(serde_yaml::Value::String("status".into()))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        let universe = crate::schema::status_universe(schema);
        if !universe.contains(status) {
            let mut allowed: Vec<&str> = universe.iter().map(String::as_str).collect();
            allowed.sort();
            return Err(MutateError::SchemaViolation(format!(
                "status {status:?} is not in the allowed set [{}]",
                allowed.join(", ")
            )));
        }
    }

    // 4c) declarative transition-rule check. Coordinates with the
    //     mutation verbs in the same module by sharing a single hook
    //     surface — both write paths apply rules through the
    //     post-mutation `Issue` projection. Rules are loaded once by
    //     the caller (same pattern as `schema`).
    let projected = projected_issue_for_rules(slug, &item, &item_path, schema)?;

    // Intrinsic intake invariants (OD-9 A) — always on, independent of
    // `.issuectl/transitions.yaml`. Routed through here so the generic
    // `set status` / `update --status` path enforces the same type ×
    // status and reception-state rules as the first-class intake verbs;
    // neither is a bypass. Gated on an actual status/type change so a
    // no-op re-assert or an unrelated field PATCH against legacy data is
    // never retroactively rejected.
    if projected.status != prev_status || projected.issue_type != prev_type {
        let intrinsic = intake::intrinsic_transition_violations(
            &prev_status,
            &prev_type,
            &projected.status,
            &projected.issue_type,
        );
        if !intrinsic.is_empty() {
            return Err(MutateError::TransitionViolation(intrinsic.join("; ")));
        }
    }

    let mut rule_violations =
        crate::transitions::evaluate_transition(rules, &projected, &prev_status);
    let (dod_warnings, dod_errors) =
        crate::transitions::evaluate_dod(schema, &projected, &prev_status);
    rule_violations.extend(dod_errors);
    if !rule_violations.is_empty() {
        return Err(MutateError::TransitionViolation(rule_violations.join("; ")));
    }

    // Post-mutation closing classification drives both the dry-run dir
    // prediction and the real unarchive decision below — an archived
    // issue left non-closing gets lifted back to the active root.
    let post_closing = crate::schema::is_closing(schema, &projected.status);

    // 5) Either dry-run (compute serialized bytes, skip write/publish)
    //    or atomic write. The only directory move is unarchiving (an
    //    archived issue reopened to a non-closing status); flat-layout
    //    status changes keep `item_path` as the canonical location.
    if req.dry_run {
        let pending = write::serialize_item(&item).map_err(MutateError::Io)?;
        let new_issue = parse_serialized(&pending, slug, schema);
        let new_version = canonical_hash(&new_issue);
        return Ok(UpdateOutcome {
            issue: new_issue,
            version: new_version,
            issue_dir: predicted_issue_dir(root, slug, &item_path, post_closing),
            moved_to_closed,
            moved_to_open,
            pending_serialized: Some(pending),
            before_serialized,
            warnings: dod_warnings,
        });
    }
    // 5b) Side effects deferred from `update_issue` so they only fire
    //     after every validation step above has passed. Schema
    //     bootstrap and the legacy → flat directory migration would
    //     otherwise leak past a rolled-back transaction (failed body
    //     op, schema violation, transition rejection): the on-disk
    //     `item.md` would be unchanged but `.schema.yaml` would be
    //     newly created and the legacy directory permanently moved.
    crate::schema::ensure_default_written(root).map_err(MutateError::Io)?;
    // An archived issue left in a non-closing status must be lifted out of
    // cold storage: otherwise the frontmatter write below lands on the
    // `issues/archive/YYYY/MM/<slug>/` path and the issue reads as active
    // in `list`/`show` while physically still living in the archive. This
    // is the inverse of the `archive` move and runs under the same flock.
    let item_path = unarchive_if_active(root, slug, item_path, post_closing)?;
    let final_path = migrate_to_flat_if_legacy(root, slug, &item_path)?;
    write_item_atomic(&final_path, &item).map_err(MutateError::Io)?;

    // 6) recompute canonical hash from final on-disk content
    let after = crate::parser::parse_item_md_with_warnings(&final_path, slug, "open");
    let mut new_issue = after.issue;
    new_issue.folder = folder_for_status(schema, &new_issue.status).to_string();
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
        before_serialized: None,
        warnings: dod_warnings,
    })
}

/// Re-parse already-serialized `item.md` bytes into a domain `Issue`.
/// Dry-run paths serialize the post-mutation `ItemFile` to compute
/// `pending_serialized` for the diff; passing those same bytes back
/// here avoids serializing a second time.
fn parse_serialized(serialized: &str, slug: &str, schema: &crate::schema::Schema) -> Issue {
    let parsed = crate::parser::parse_item_md_text_with_warnings(
        serialized,
        slug,
        "open",
        Path::new("<dry-run>"),
    );
    let mut issue = parsed.issue;
    issue.folder = folder_for_status(schema, &issue.status).to_string();
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
    closed_by: Option<String>,
    commits: Vec<CommitSpec>,
    expected_version: Option<String>,
    hub: Option<&Arc<EventHub>>,
    config: &dyn ConfigSource,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }
    // `--as` is optional on `close` (unlike `note`, where it is
    // required), but when present it must satisfy the same author
    // grammar `note` uses so the closer attribution is a well-formed,
    // hash-stable token in the same vocabulary. Recorded as the
    // `closed_by:` frontmatter field alongside the auto-stamped
    // `closed:` date — see the status branch in `update_issue_under_lock`.
    if let Some(author) = &closed_by {
        crate::body_sections::validate_author(author)
            .map_err(|e| MutateError::Validation(e.to_string()))?;
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    // `update_issue_under_lock` runs `ensure_default_written` and the
    // legacy → flat migration only after every validation step has
    // passed. We therefore locate read-only here so a status-precondition
    // failure (already-closing issue) leaves no repo side effects.
    let item_path = locate_for_dry_run(root, slug)?;
    let item = write::read_item(&item_path).map_err(MutateError::Io)?;
    let schema = config
        .schema(root)
        .map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let current_status = item
        .frontmatter
        .get(serde_yaml::Value::String("status".into()))
        .and_then(|v| v.as_str())
        .unwrap_or("open")
        .to_string();
    if crate::schema::is_closing(&schema, &current_status) {
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
    // Default-status selection: bug → `fixed`, anything else → `done`.
    // The brief explicitly keeps built-in defaults — projects that
    // want a custom closing status as the `close` default must pass
    // it via `--status`. Schema validation under-lock then rejects
    // values the project's `status` enum disallows.
    let resolved_status = status_override.unwrap_or_else(|| {
        if issue_type == "bug" {
            "fixed".to_string()
        } else {
            "done".to_string()
        }
    });
    // `close` must land on a *closing* status. Schema validation under
    // lock only checks that the value is in the `status` enum, not that
    // it closes the issue — so without this guard `close --status open`
    // (or a schema that reclassifies `fixed` as active) would run the
    // reopen branch, leaving the issue active. Combined with `--as` that
    // produced an active issue carrying `closed_by`. Reject early.
    if !crate::schema::is_closing(&schema, &resolved_status) {
        return Err(MutateError::Validation(format!(
            "close status {resolved_status:?} is not a closing status; \
             use `update --status` to move an issue between active states"
        )));
    }

    let req = UpdateIssueRequest {
        expected_version,
        status: Patch::Set(resolved_status),
        // Closer attribution rides the same under-lock write as the
        // status flip via the first-class `closed_by` slot (NOT a custom
        // field): it is validated in `UpdateIssueRequest::validate`,
        // stamped alongside `closed:` in the status branch, surfaces in
        // `show --json` via `Issue::extra`, and is folded into the
        // version hash. Reopening clears it in lockstep with `closed:`.
        closed_by: match closed_by {
            Some(author) => Patch::Set(author),
            None => Patch::Unspecified,
        },
        add_commits: commits,
        ..Default::default()
    };
    // _lock drops at end-of-scope after the locked update path returns.
    // We call the under-lock helper directly so we don't double-acquire
    // (fs2 advisory flock is per-fd; nested `WriteLock::acquire` would
    // deadlock on Linux).
    let mut req_normalized = req;
    let normalized_add_related = crate::refs::normalize_related_refs(&req_normalized.add_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let normalized_remove_related =
        crate::refs::normalize_related_refs(&req_normalized.remove_related)
            .map_err(|e| MutateError::Validation(e.to_string()))?;
    req_normalized.add_related = normalized_add_related;
    req_normalized.remove_related = normalized_remove_related;
    req_normalized.validate()?;
    let rules = load_validated_rules(root, &schema, config)?;
    update_issue_under_lock(root, slug, item_path, req_normalized, hub, &schema, &rules)
}

/// Apply the *same* mutation to many issues under a single repo-wide
/// flock. Powers `issuectl bulk`.
///
/// `make_req(dry_run)` must return a fresh, content-identical request
/// each time it is called. The mutation is the same for every target,
/// but [`UpdateIssueRequest`] is not `Clone` (it owns `Vec`s and a map)
/// and each write consumes its own request — hence the factory rather
/// than one shared value.
///
/// Semantics, in order:
/// 1. Acquire the repo-wide write lock **once** for the whole batch.
/// 2. Load schema + transition rules **once**.
/// 3. Phase 1 — validate and plan every target as an in-memory dry-run.
///    No file is written. Any validation failure aborts here with the
///    offending slug, so a bad value on the last target writes nothing.
/// 4. Phase 2 (skipped when `dry_run`) — write every target for real.
///
/// Holding one lock across both phases closes the time-of-check /
/// time-of-use window a per-call-locking loop would open: no concurrent
/// writer can slip between a target's validation and its write, and the
/// whole batch is serialized against other writers. This is the "one
/// commit" guarantee `bulk` advertises. The only residual non-atomicity
/// is a mid-phase-2 I/O error (disk full, EIO): earlier targets are
/// already on disk. That case returns an `Io` error naming how many
/// landed so the caller can surface the partial set.
pub fn bulk_update(
    root: &Path,
    slugs: &[String],
    mut make_req: impl FnMut(bool) -> UpdateIssueRequest,
    dry_run: bool,
    hub: Option<&Arc<EventHub>>,
    config: &dyn ConfigSource,
) -> Result<Vec<UpdateOutcome>, MutateError> {
    for slug in slugs {
        if !crate::slug::is_valid(slug) {
            return Err(MutateError::Validation(format!(
                "invalid slug shape: {slug:?}"
            )));
        }
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    let schema = config
        .schema(root)
        .map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let rules = load_validated_rules(root, &schema, config)?;

    // Phase 1: validate + plan every target with a dry-run request, so
    // nothing is written until all targets are known-good. Dry-run mode
    // returns these planned outcomes directly (they carry the diff bytes).
    let mut planned = Vec::with_capacity(slugs.len());
    for slug in slugs {
        let req = prepare_bulk_req(make_req(true))?;
        let item_path = locate_for_dry_run(root, slug)?;
        let outcome = update_issue_under_lock(root, slug, item_path, req, hub, &schema, &rules)
            .map_err(|e| with_slug_context(slug, e))?;
        planned.push(outcome);
    }
    if dry_run {
        return Ok(planned);
    }

    // Phase 2: real writes. Every target already validated under this
    // same lock, so only I/O failures are expected from here on.
    let mut outcomes = Vec::with_capacity(slugs.len());
    for (i, slug) in slugs.iter().enumerate() {
        let req = prepare_bulk_req(make_req(false))?;
        let item_path = locate_for_dry_run(root, slug)?;
        match update_issue_under_lock(root, slug, item_path, req, hub, &schema, &rules) {
            Ok(o) => outcomes.push(o),
            Err(e) => {
                let written = slugs[..i]
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(MutateError::Io(anyhow!(
                    "bulk write failed on {slug} after writing {} issue(s) [{written}]: {}",
                    i,
                    e
                )));
            }
        }
    }
    Ok(outcomes)
}

/// Normalize related-ref shapes and run request validation — the part
/// of `update_issue` that runs before the lock. Shared by every
/// `bulk_update` target so a bulk write enforces the exact same
/// per-request contract as a single `update`.
fn prepare_bulk_req(req: UpdateIssueRequest) -> Result<UpdateIssueRequest, MutateError> {
    let add = crate::refs::normalize_related_refs(&req.add_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let remove = crate::refs::normalize_related_refs(&req.remove_related)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let add_bb = crate::refs::normalize_related_refs(&req.add_blocked_by)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let rem_bb = crate::refs::normalize_related_refs(&req.remove_blocked_by)
        .map_err(|e| MutateError::Validation(e.to_string()))?;
    let mut req = req;
    req.add_related = add;
    req.remove_related = remove;
    req.add_blocked_by = add_bb;
    req.remove_blocked_by = rem_bb;
    req.validate()?;
    Ok(req)
}

/// Prefix a per-target error with its slug while preserving the error
/// variant (so the server/CLI keep their status mapping). Bulk writes
/// fail one slug at a time; naming it is the difference between an
/// actionable error and a mystery.
fn with_slug_context(slug: &str, e: MutateError) -> MutateError {
    use MutateError::*;
    match e {
        Validation(s) => Validation(format!("{slug}: {s}")),
        ConflictingIntent(s) => ConflictingIntent(format!("{slug}: {s}")),
        SchemaViolation(s) => SchemaViolation(format!("{slug}: {s}")),
        TransitionViolation(s) => TransitionViolation(format!("{slug}: {s}")),
        NotFound => Validation(format!("{slug}: issue not found")),
        other => other,
    }
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
    config: &dyn ConfigSource,
) -> Result<UpdateOutcome, MutateError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!(
            "invalid slug shape: {slug:?}"
        )));
    }

    let _lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    // Locate read-only regardless of dry_run so the legacy → flat
    // migration and `.schema.yaml` bootstrap fire only after every
    // validation step has passed (parity with `update_issue`).
    let item_path = locate_for_dry_run(root, slug)?;
    let schema = config
        .schema(root)
        .map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;

    let parsed = crate::parser::parse_item_md_with_warnings(&item_path, slug, "open");
    if !parsed.warnings.is_empty() {
        return Err(MutateError::Corrupt {
            warnings: parsed.warnings,
        });
    }
    let mut prev_issue = parsed.issue;
    prev_issue.folder = folder_for_status(&schema, &prev_issue.status).to_string();
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
    let before_serialized =
        if dry_run {
            // Raw on-disk bytes — not `serialize_item(&item)`. The
            // canonicalised re-serialization would mask formatting
            // changes (dropped YAML comments, key reordering, scalar-
            // style shifts) that the real write would also apply,
            // making the dry-run preview lie about disk impact (round-2 #2).
            Some(fs::read_to_string(&item_path).map_err(|e| {
                MutateError::Io(anyhow!("cannot read {}: {e}", item_path.display()))
            })?)
        } else {
            None
        };
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
    let violations = crate::schema::validate(&schema, &item.frontmatter);
    // Body-replace never writes status/closed, so an empty `written`
    // set keeps the lenient RequiredWhen handling.
    if let Some(msg) = hard_schema_failure(&violations, &std::collections::BTreeSet::new()) {
        return Err(MutateError::SchemaViolation(msg));
    }

    // Transition rules apply on the body-replace path too. Without
    // this, a client that PATCHed status=done with checked AC could
    // `update_body` afterwards to wipe / uncheck them, leaving the
    // issue in a state that violates the rule it just satisfied.
    // Status doesn't change here, so only `requires_*` checks matter
    // (graph rules are skipped by the prev==new guard).
    let rules = load_validated_rules(root, &schema, config)?;
    let projected = projected_issue_for_rules(slug, &item, &item_path, &schema)?;
    let rule_violations =
        crate::transitions::evaluate_transition(&rules, &projected, &prev_issue.status);
    if !rule_violations.is_empty() {
        return Err(MutateError::TransitionViolation(rule_violations.join("; ")));
    }

    if dry_run {
        let pending = write::serialize_item(&item).map_err(MutateError::Io)?;
        let new_issue = parse_serialized(&pending, slug, &schema);
        let new_version = canonical_hash(&new_issue);
        // Body verbs never change status, so an archived issue stays
        // archived — predict its current dir, not the active root.
        let post_closing = crate::schema::is_closing(&schema, &new_issue.status);
        return Ok(UpdateOutcome {
            issue: new_issue,
            version: new_version,
            issue_dir: predicted_issue_dir(root, slug, &item_path, post_closing),
            moved_to_closed: false,
            moved_to_open: false,
            pending_serialized: Some(pending),
            before_serialized,
            warnings: Vec::new(),
        });
    }

    // Side effects deferred from the top of the function so a failed
    // validation above leaves no `.schema.yaml` bootstrap and no
    // legacy → flat migration on disk.
    crate::schema::ensure_default_written(root).map_err(MutateError::Io)?;
    let final_path = migrate_to_flat_if_legacy(root, slug, &item_path)?;
    write_item_atomic(&final_path, &item).map_err(MutateError::Io)?;

    let after = crate::parser::parse_item_md_with_warnings(&final_path, slug, "open");
    let mut new_issue = after.issue;
    new_issue.folder = folder_for_status(&schema, &new_issue.status).to_string();
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
        issue_dir: final_path
            .parent()
            .expect("item.md has a parent")
            .to_path_buf(),
        moved_to_closed: false,
        moved_to_open: false,
        pending_serialized: None,
        before_serialized: None,
        warnings: Vec::new(),
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
    config: &dyn ConfigSource,
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
    // Locate read-only regardless of `dry_run`. Migration / schema
    // bootstrap deferred to just before atomic write so that any
    // validation failure below leaves no repo side effects.
    let item_path = locate_for_dry_run(root, slug)?;
    let schema = config
        .schema(root)
        .map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;

    let parsed = crate::parser::parse_item_md_with_warnings(&item_path, slug, "open");
    if !parsed.warnings.is_empty() {
        return Err(MutateError::Corrupt {
            warnings: parsed.warnings,
        });
    }
    let mut prev_issue = parsed.issue;
    prev_issue.folder = folder_for_status(&schema, &prev_issue.status).to_string();
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
    let before_serialized =
        if dry_run {
            // Raw on-disk bytes — not `serialize_item(&item)`. The
            // canonicalised re-serialization would mask formatting
            // changes (dropped YAML comments, key reordering, scalar-
            // style shifts) that the real write would also apply,
            // making the dry-run preview lie about disk impact (round-2 #2).
            Some(fs::read_to_string(&item_path).map_err(|e| {
                MutateError::Io(anyhow!("cannot read {}: {e}", item_path.display()))
            })?)
        } else {
            None
        };
    let block =
        crate::body_sections::render_note_block(&crate::body_sections::now_iso(), author, message)
            .map_err(|e| MutateError::Validation(e.to_string()))?;
    let trimmed_body = item.body.trim_start_matches('\n');
    let appended = crate::body_sections::append_block(trimmed_body, section, &block);
    // Canonicalise leading-newline shape so `serialize_item` always
    // produces `---\n\n<body>` rather than leaving a legacy
    // no-blank-line file in a state `fmt` would still want to change.
    item.body = crate::body_sections::canonicalise_body_leading(&appended);
    write::set_string(&mut item.frontmatter, "updated", &write::today());

    // Schema validation runs on every write surface for parity with
    // `update_body` / `update_issue` — without this, a tightened
    // schema could block `body set` while letting `note` keep
    // mutating the same invalid issue (review finding #6).
    validate_against_schema(root, &item.frontmatter, config)?;

    // Transition rules also evaluated for parity with `update_body`,
    // BUT — by design — violations are surfaced as warnings rather
    // than hard errors. `cmd_note` is a body-only verb agents reach
    // for to record intent (decisions, agent runs, comments); blocking
    // the write would force them to back out and replay through the
    // unified `apply` envelope just to log "I noticed AC#2 is
    // unticked." We let the write through, leave the issue in a
    // (potentially) rule-violating state, and tell the caller. The
    // unified PATCH path (`update_issue_under_lock`) keeps the strict
    // rejection — body_ops there compose with frontmatter mutations,
    // so the caller has the tools to fix the violation in the same
    // transaction.
    let warnings = transition_warnings(root, slug, &item, &item_path, &prev_issue.status, config);

    if dry_run {
        let pending = write::serialize_item(&item).map_err(MutateError::Io)?;
        let new_issue = parse_serialized(&pending, slug, &schema);
        let new_version = canonical_hash(&new_issue);
        // Body verbs never change status, so an archived issue stays
        // archived — predict its current dir, not the active root.
        let post_closing = crate::schema::is_closing(&schema, &new_issue.status);
        return Ok(UpdateOutcome {
            issue: new_issue,
            version: new_version,
            issue_dir: predicted_issue_dir(root, slug, &item_path, post_closing),
            moved_to_closed: false,
            moved_to_open: false,
            pending_serialized: Some(pending),
            before_serialized,
            warnings,
        });
    }

    crate::schema::ensure_default_written(root).map_err(MutateError::Io)?;
    let final_path = migrate_to_flat_if_legacy(root, slug, &item_path)?;
    write_item_atomic(&final_path, &item).map_err(MutateError::Io)?;

    let after = crate::parser::parse_item_md_with_warnings(&final_path, slug, "open");
    let mut new_issue = after.issue;
    new_issue.folder = folder_for_status(&schema, &new_issue.status).to_string();
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
        issue_dir: final_path
            .parent()
            .expect("item.md has a parent")
            .to_path_buf(),
        moved_to_closed: false,
        moved_to_open: false,
        pending_serialized: None,
        before_serialized: None,
        warnings,
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
    config: &dyn ConfigSource,
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
    // Locate read-only regardless of `dry_run`. Migration / schema
    // bootstrap deferred to just before atomic write.
    let item_path = locate_for_dry_run(root, slug)?;
    let schema = config
        .schema(root)
        .map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let parsed = crate::parser::parse_item_md_with_warnings(&item_path, slug, "open");
    if !parsed.warnings.is_empty() {
        return Err(MutateError::Corrupt {
            warnings: parsed.warnings,
        });
    }
    let mut prev_issue = parsed.issue;
    prev_issue.folder = folder_for_status(&schema, &prev_issue.status).to_string();
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
    let before_serialized =
        if dry_run {
            // Raw on-disk bytes — not `serialize_item(&item)`. The
            // canonicalised re-serialization would mask formatting
            // changes (dropped YAML comments, key reordering, scalar-
            // style shifts) that the real write would also apply,
            // making the dry-run preview lie about disk impact (round-2 #2).
            Some(fs::read_to_string(&item_path).map_err(|e| {
                MutateError::Io(anyhow!("cannot read {}: {e}", item_path.display()))
            })?)
        } else {
            None
        };
    let new_body = toggle_checkbox_in_body(&item.body, substring)?;
    item.body = new_body;
    write::set_string(&mut item.frontmatter, "updated", &write::today());

    validate_against_schema(root, &item.frontmatter, config)?;

    // Transition rules: surface as warnings on this body-only verb,
    // matching `note_issue`. Toggling a checkbox cannot legitimately
    // be blocked by a transition rule (the verb doesn't change
    // status), but a `requires_*` rule that pins acceptance criteria
    // for an already-closing issue WILL fire here — and the user
    // probably wants to know without being blocked from making the
    // edit. The unified `body_ops` PATCH path keeps the strict
    // rejection.
    let warnings = transition_warnings(root, slug, &item, &item_path, &prev_issue.status, config);

    if dry_run {
        let pending = write::serialize_item(&item).map_err(MutateError::Io)?;
        let new_issue = parse_serialized(&pending, slug, &schema);
        let new_version = canonical_hash(&new_issue);
        // Body verbs never change status, so an archived issue stays
        // archived — predict its current dir, not the active root.
        let post_closing = crate::schema::is_closing(&schema, &new_issue.status);
        return Ok(UpdateOutcome {
            issue: new_issue,
            version: new_version,
            issue_dir: predicted_issue_dir(root, slug, &item_path, post_closing),
            moved_to_closed: false,
            moved_to_open: false,
            pending_serialized: Some(pending),
            before_serialized,
            warnings,
        });
    }

    crate::schema::ensure_default_written(root).map_err(MutateError::Io)?;
    let final_path = migrate_to_flat_if_legacy(root, slug, &item_path)?;
    write_item_atomic(&final_path, &item).map_err(MutateError::Io)?;
    let after = crate::parser::parse_item_md_with_warnings(&final_path, slug, "open");
    let mut new_issue = after.issue;
    new_issue.folder = folder_for_status(&schema, &new_issue.status).to_string();
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
        issue_dir: final_path
            .parent()
            .expect("item.md has a parent")
            .to_path_buf(),
        moved_to_closed: false,
        moved_to_open: false,
        pending_serialized: None,
        before_serialized: None,
        warnings,
    })
}

/// Evaluate transition rules against the post-mutation projection and
/// return any violations as warning strings. Used by body-only verbs
/// (`note_issue`, `toggle_checkbox`) that surface rule mismatches as
/// warnings rather than hard errors. `prev_status` is the on-disk
/// status before the mutation; for body-only verbs that's the same as
/// the post-mutation status, so only `requires_*` rules can fire.
///
/// On schema / transitions config load failure we *surface* the error
/// as a warning rather than swallow it. The body verbs predate the
/// rules engine and shouldn't refuse the write because the operator
/// broke `transitions.yaml`, but they also shouldn't go silent on it —
/// without a warning, agents iterating with `note` / `check` against a
/// broken config would never know the rules engine is dead, which is a
/// trust violation. The unified PATCH path keeps the strict
/// `MutateError::TransitionConfig` rejection.
fn transition_warnings(
    root: &Path,
    slug: &str,
    item: &write::ItemFile,
    item_path: &Path,
    prev_status: &str,
    config: &dyn ConfigSource,
) -> Vec<String> {
    let schema = match config.schema(root) {
        Ok(s) => s,
        Err(e) => {
            return vec![format!(
                "rules engine: schema load failed, transition checks skipped: {e:#}"
            )]
        }
    };
    let rules = match config.rules(root) {
        Ok(r) => r,
        Err(e) => {
            return vec![format!(
                "rules engine: transitions config load failed, transition checks skipped: {e:#}"
            )]
        }
    };
    let universe = crate::schema::status_universe(&schema);
    if let Err(e) = crate::transitions::validate_status_refs(&rules, &universe) {
        return vec![format!(
            "rules engine: transitions reference unknown statuses, transition checks skipped: {e:#}"
        )];
    }
    let projected = match projected_issue_for_rules(slug, item, item_path, &schema) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    crate::transitions::evaluate_transition(&rules, &projected, prev_status)
}

/// Apply a single `BodyOp` against the in-flight `ItemFile`. Shared by
/// the `body_ops` vector in `UpdateIssueRequest` so the same primitives
/// `cmd_note` / `cmd_check` use also drive the transactional `apply`
/// path — keeping the rendering, fence handling, and error messages
/// identical across surfaces.
fn apply_body_op(item: &mut ItemFile, index: usize, op: &BodyOp) -> Result<(), MutateError> {
    match op {
        BodyOp::SetCheckbox(set) => {
            let new_body = set_checkbox_in_body(&item.body, &set.match_substring, set.checked)
                .map_err(|e| prefix_body_op_error(index, e))?;
            item.body = new_body;
        }
        BodyOp::AppendNote(note) => {
            // `validate()` already checked author/message, but
            // `render_note_block` re-validates defensively. Mirror the
            // same `body_ops[i].<field>:` shape here so the under-lock
            // error path matches the pre-lock validation path
            // byte-for-byte (LLM review consensus #5).
            crate::body_sections::validate_author(&note.author)
                .map_err(|e| MutateError::Validation(format!("body_ops[{index}].author: {e}")))?;
            crate::body_sections::validate_message(&note.message)
                .map_err(|e| MutateError::Validation(format!("body_ops[{index}].message: {e}")))?;
            let block = crate::body_sections::render_note_block(
                &crate::body_sections::now_iso(),
                &note.author,
                &note.message,
            )
            .map_err(|e| MutateError::Validation(format!("body_ops[{index}]: {e}")))?;
            let trimmed = item.body.trim_start_matches('\n');
            let appended =
                crate::body_sections::append_block(trimmed, note.section.as_str(), &block);
            item.body = crate::body_sections::canonicalise_body_leading(&appended);
        }
    }
    Ok(())
}

/// Attach `body_ops[{index}]:` context to *every* error variant a body
/// op might surface. The previous `match` only wrapped `Validation`
/// and let other variants pass through unprefixed — dead today (the
/// body primitives only return `Validation`), but a footgun the
/// moment one of them grows an Io / SchemaViolation path.
fn prefix_body_op_error(index: usize, err: MutateError) -> MutateError {
    match err {
        MutateError::Validation(s) => MutateError::Validation(format!("body_ops[{index}]: {s}")),
        MutateError::SchemaViolation(s) => {
            MutateError::SchemaViolation(format!("body_ops[{index}]: {s}"))
        }
        MutateError::TransitionViolation(s) => {
            MutateError::TransitionViolation(format!("body_ops[{index}]: {s}"))
        }
        MutateError::ConflictingIntent(s) => {
            MutateError::ConflictingIntent(format!("body_ops[{index}]: {s}"))
        }
        MutateError::Io(e) => {
            // `e.context(...)` preserves the anyhow `source()` chain so
            // downstream `{e:#}` rendering and `e.chain()` walking still
            // work; the previous `format!("{e:#}")` flattened the chain
            // into the inner message and threw away the source links.
            MutateError::Io(e.context(format!("body_ops[{index}]")))
        }
        // Variants we deliberately pass through unchanged. `NotFound`,
        // `VersionMismatch`, and `AmbiguousSlug` describe whole-document
        // state from before the body-op loop — the index doesn't help.
        // `Corrupt { warnings: Vec<String> }` does carry a payload, but
        // it's parser warnings about the on-disk file, not a single
        // body-op error; splicing the index in would mislead. Operator-
        // facing config errors (`SchemaConfig`, `TransitionConfig`) are
        // about the repo's configuration, not the request.
        other => other,
    }
}

/// Drive the unique checkbox line containing `substring` to the target
/// `checked` state. Idempotent: if the matched line is already in the
/// target state, returns the body unchanged so retried agent requests
/// don't flip the box back and forth. Errors when zero or multiple
/// lines match — same shape as `toggle_checkbox_in_body`.
fn set_checkbox_in_body(body: &str, substring: &str, checked: bool) -> Result<String, MutateError> {
    let lines: Vec<&str> = body.split('\n').collect();
    let mut matches: Vec<(usize, bool)> = Vec::new();
    crate::body_sections::for_each_line_outside_fences(body, |i, line| {
        if let Some(state) = checkbox_state(line) {
            if line.contains(substring) {
                matches.push((i, state));
            }
        }
    });
    match matches.as_slice() {
        [] => Err(MutateError::Validation(format!(
            "no checkbox line matched {substring:?}"
        ))),
        [(idx, current_state)] => {
            if *current_state == checked {
                return Ok(body.to_string());
            }
            let new_line = set_line_checkbox(lines[*idx], checked).ok_or_else(|| {
                MutateError::Validation(format!(
                    "internal: matched line {:?} is not a checkbox after match",
                    lines[*idx]
                ))
            })?;
            let mut out = Vec::with_capacity(lines.len());
            for (i, l) in lines.iter().enumerate() {
                if i == *idx {
                    out.push(new_line.clone());
                } else {
                    out.push((*l).to_string());
                }
            }
            Ok(out.join("\n"))
        }
        many => Err(MutateError::Validation(format!(
            "{} checkbox lines matched {substring:?}; refine to a unique substring",
            many.len()
        ))),
    }
}

/// Find a unique checkbox line containing `substring` and return the
/// body with that one line's `[ ]` / `[x]` toggled. Fence-aware:
/// checkbox lines inside fenced code blocks are skipped so example
/// task lists in documentation snippets don't get silently mutated.
/// The checkbox shape matched is `^\s*[-*+]\s+\[[ xX]\]\s` so common
/// GFM variants work, while non-checkbox brackets like `- [n]` are
/// rejected.
fn toggle_checkbox_in_body(body: &str, substring: &str) -> Result<String, MutateError> {
    let lines: Vec<&str> = body.split('\n').collect();
    let mut matches: Vec<usize> = Vec::new();
    // Fence-aware enumeration so `- [ ]` examples inside ```fenced```
    // code blocks aren't toggled. Routes through the borrowing
    // callback wrapper rather than `lines_outside_fences` so we don't
    // allocate one `String` per scanned line.
    crate::body_sections::for_each_line_outside_fences(body, |i, line| {
        if checkbox_state(line).is_some() && line.contains(substring) {
            matches.push(i);
        }
    });
    match matches.len() {
        0 => Err(MutateError::Validation(format!(
            "no checkbox line matched {substring:?}"
        ))),
        1 => {
            let idx = matches[0];
            let toggled = toggle_line_checkbox(lines[idx]).ok_or_else(|| {
                MutateError::Validation(format!(
                    "internal: matched line {:?} is not a checkbox after match",
                    lines[idx]
                ))
            })?;
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

/// `Some(true)` for `- [x]`, `Some(false)` for `- [ ]`, `None`
/// otherwise. Byte-based parsing so multibyte mark chars (e.g.
/// `[✓]`, `[é]`) return `None` rather than panicking on a
/// non-char-boundary slice.
fn checkbox_state(line: &str) -> Option<bool> {
    let bytes = line.as_bytes();
    // Skip leading ASCII whitespace. A non-ASCII leading char means
    // this can't be a GFM checkbox line — return None.
    let mut i = 0;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    let bullet = *bytes.get(i)?;
    if !matches!(bullet, b'-' | b'*' | b'+') {
        return None;
    }
    i += 1;
    if !matches!(bytes.get(i), Some(b' ' | b'\t')) {
        return None;
    }
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if bytes.get(i) != Some(&b'[') {
        return None;
    }
    let mark = *bytes.get(i + 1)?;
    if bytes.get(i + 2) != Some(&b']') {
        return None;
    }
    if !matches!(bytes.get(i + 3), Some(b' ' | b'\t')) {
        return None;
    }
    match mark {
        b' ' => Some(false),
        b'x' | b'X' => Some(true),
        _ => None,
    }
}

/// Toggle the `[ ]` / `[x]` mark on a known-checkbox line. Returns
/// `None` if the line doesn't match the byte-safe checkbox shape —
/// callers should have validated via `checkbox_state` first. Builds
/// the result from raw bytes to avoid the implicit "ASCII-only first
/// 4 chars" invariant the previous string-slice version assumed.
fn toggle_line_checkbox(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if !matches!(bytes.get(i), Some(b'-' | b'*' | b'+')) {
        return None;
    }
    i += 1;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if bytes.get(i) != Some(&b'[') {
        return None;
    }
    let mark_idx = i + 1;
    let mark = *bytes.get(mark_idx)?;
    if bytes.get(i + 2) != Some(&b']') {
        return None;
    }
    let new_mark = if mark == b' ' { b'x' } else { b' ' };
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[..mark_idx]);
    out.push(new_mark);
    out.extend_from_slice(&bytes[mark_idx + 1..]);
    // Safe: we replaced one ASCII byte with another at a position
    // already proven to be ASCII by the byte checks above; the rest
    // of the line (including any non-ASCII content after `]`) is
    // copied byte-for-byte and therefore preserves UTF-8 validity.
    String::from_utf8(out).ok()
}

/// Drive a checkbox line to the target `checked` state regardless of
/// its current state. Returns `None` when the line doesn't match the
/// byte-safe checkbox shape — callers should have validated via
/// `checkbox_state` first. Mirror of `toggle_line_checkbox` but with
/// an explicit target so `set_checkbox_in_body` can be idempotent.
fn set_line_checkbox(line: &str, checked: bool) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if !matches!(bytes.get(i), Some(b'-' | b'*' | b'+')) {
        return None;
    }
    i += 1;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if bytes.get(i) != Some(&b'[') {
        return None;
    }
    let mark_idx = i + 1;
    if bytes.get(mark_idx).is_none() || bytes.get(i + 2) != Some(&b']') {
        return None;
    }
    let new_mark = if checked { b'x' } else { b' ' };
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[..mark_idx]);
    out.push(new_mark);
    out.extend_from_slice(&bytes[mark_idx + 1..]);
    String::from_utf8(out).ok()
}

/// Run schema validation against a post-mutation frontmatter mapping.
/// Centralised so every body- and frontmatter-mutation entry point
/// (`update_issue_under_lock`, `update_body`, `note_issue`,
/// `toggle_checkbox`) enforces the same contract — schema runs once
/// per write, immediately before atomic write or dry-run return.
/// Join schema violations into a hard-fail message, conditionally
/// dropping `RequiredWhen` violations. A `required_when` constraint
/// (today: closing status implies `closed:`) is a lifecycle-consistency
/// rule that `doctor` owns and heals. When a mutation leaves a field's
/// `required_when` condition unsatisfied *without having touched either
/// that field or the `status` that drives the condition*, the violation
/// is a pre-existing inconsistency the user didn't introduce — blocking
/// an unrelated edit (e.g. a checkbox toggle on an already-`done` issue)
/// would be surprising, so it's dropped.
///
/// But when the mutation *did* write the field or `status`, the
/// violation is something this very write produced — e.g. explicitly
/// clearing `closed:` on a closing-status issue (`set closed ""`). That
/// must be rejected, not silently healed later, so the `RequiredWhen` is
/// kept. `written` is the set of frontmatter keys this mutation wrote;
/// body-only paths pass an empty set and so keep the lenient behaviour.
/// Returns `None` when nothing remains to fail on.
fn hard_schema_failure(
    violations: &[crate::schema::ViolationKind],
    written: &std::collections::BTreeSet<String>,
) -> Option<String> {
    // A `RequiredWhen` condition is gated solely on the issue's status
    // class (`schema::RequiredWhen` only carries `status_class`), so
    // `status` is the condition driver for *every* such violation. If
    // the format ever grows non-status drivers, this check must learn
    // the per-violation driver (see `ViolationKind::RequiredWhen`)
    // instead of assuming `status`.
    let status_written = written.contains("status");
    let msgs: Vec<String> = violations
        .iter()
        .filter(|v| match v {
            crate::schema::ViolationKind::RequiredWhen { field, .. } => {
                // Keep (enforce) only when this mutation touched the
                // required field itself or the status that triggers it.
                status_written || written.contains(field)
            }
            _ => true,
        })
        .map(|v| v.message())
        .collect();
    (!msgs.is_empty()).then(|| msgs.join("; "))
}

/// Body-only schema gate. Callers here never mutate frontmatter, so the
/// `written` set passed to `hard_schema_failure` is always empty and
/// `RequiredWhen` violations stay lenient. A future frontmatter-mutating
/// caller must NOT route through this helper — it would silently drop a
/// `RequiredWhen` it introduced; use `hard_schema_failure` with a real
/// `written` set instead (as `update_issue_under_lock` does).
fn validate_against_schema(
    root: &Path,
    frontmatter: &serde_yaml::Mapping,
    config: &dyn ConfigSource,
) -> Result<(), MutateError> {
    let schema = config
        .schema(root)
        .map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let violations = crate::schema::validate(&schema, frontmatter);
    // Body-only mutation: status/closed are never written here, so the
    // empty `written` set preserves the lenient RequiredWhen behaviour.
    if let Some(msg) = hard_schema_failure(&violations, &std::collections::BTreeSet::new()) {
        return Err(MutateError::SchemaViolation(msg));
    }
    Ok(())
}

/// Apply a `Patch<String>` onto a frontmatter mapping. `Unspecified`
/// is a no-op; `Clear` removes the key; `Set(v)` sets the key.
/// Enforce the same epic↔non-epic frontmatter invariants `cmd_new`
/// enforces, against the post-mutation frontmatter. `cmd_new` rejects
/// `epic` with `assignee`/`reporter` and rejects `owner` on non-epic
/// types; without this, `update --type` would let you cross those
/// lines silently. Only invoked on real type changes — same-value
/// sets short-circuit before this so idempotent calls don't break.
fn check_type_invariants(new_type: &str, fm: &serde_yaml::Mapping) -> Result<(), MutateError> {
    let has_nonempty = |key: &str| -> bool {
        fm.get(serde_yaml::Value::String(key.into()))
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    };
    if new_type == "epic" {
        if has_nonempty("assignee") || has_nonempty("reporter") {
            return Err(MutateError::Validation(format!(
                "type {new_type:?} uses owner, not reporter/assignee; \
                 clear assignee/reporter from the frontmatter first \
                 (edit the YAML directly, or use the JSON API with \
                 `assignee: null` / `reporter: null`)"
            )));
        }
    } else if has_nonempty("owner") {
        return Err(MutateError::Validation(format!(
            "type {new_type:?} does not use owner (only `epic` does); \
             clear owner from the frontmatter first \
             (edit the YAML directly, or use the JSON API with `owner: null`)"
        )));
    }
    Ok(())
}

fn apply_string_patch(item: &mut ItemFile, key: &str, p: &Patch<String>) {
    match p {
        Patch::Unspecified => {}
        Patch::Clear => write::remove_key(&mut item.frontmatter, key),
        Patch::Set(v) => write::set_string(&mut item.frontmatter, key, v),
    }
}

/// Where a real write to `slug` would land on disk after any layout
/// transition the write performs. Used by dry-run paths so the JSON
/// envelope's `final_dir` agrees with what a follow-up real write would
/// produce. Three cases:
///   - currently archived AND the post-mutation status is non-closing →
///     the real write unarchives it (see [`unarchive_if_active`]), so it
///     lands at the active flat root `issues/<slug>/`.
///   - currently archived AND staying closing → the real write leaves it
///     in cold storage, so the dir is its current archive path.
///   - active or legacy → the active flat root (legacy migrates to flat).
/// Without the archive cases, dry-run on an archived issue reported the
/// active root unconditionally while a non-reopening real write actually
/// lands back in the archive (and the inverse for reopens).
fn predicted_issue_dir(root: &Path, slug: &str, item_path: &Path, post_closing: bool) -> PathBuf {
    let archive_root = root.join("issues").join(repo::ARCHIVE_DIR);
    let in_archive = item_path
        .parent()
        .is_some_and(|p| p.starts_with(&archive_root));
    if in_archive && post_closing {
        // Stays in cold storage — report its current archive dir.
        return item_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.join("issues").join(slug));
    }
    root.join("issues").join(slug)
}

/// Locate the issue without migrating. Every mutation entry point
/// (CLI verbs and the unified PATCH path) now uses this read-only
/// locate; the legacy → flat directory rename and the default
/// `.schema.yaml` bootstrap are deferred until just before
/// `write_item_atomic` via `migrate_to_flat_if_legacy`. That guarantees
/// validation failures (schema, transition rules, body op match) leave
/// no repo side effects.
fn locate_for_dry_run(root: &Path, slug: &str) -> Result<PathBuf, MutateError> {
    use repo::LayoutState;
    match repo::resolve_layout(root, slug) {
        LayoutState::Flat { item_path }
        | LayoutState::Inbox { item_path }
        | LayoutState::Legacy { item_path, .. } => Ok(item_path),
        LayoutState::Ambiguous { paths } => Err(MutateError::AmbiguousSlug { paths }),
        LayoutState::Absent => Err(MutateError::NotFound),
        LayoutState::Invalid { reason, .. } => Err(MutateError::Io(anyhow!("{reason}"))),
    }
}

/// If the located item path is at a legacy `issues/{open,closed}/<slug>/`
/// directory, run the legacy → flat migration in-place and return the
/// new flat path. Otherwise return the path unchanged. Called after
/// validation has passed so the rename is part of the same atomic
/// success — never on a rolled-back transaction.
fn migrate_to_flat_if_legacy(
    root: &Path,
    slug: &str,
    item_path: &Path,
) -> Result<PathBuf, MutateError> {
    use repo::LayoutState;
    let needs_migration = matches!(repo::resolve_layout(root, slug), LayoutState::Legacy { .. });
    if !needs_migration {
        return Ok(item_path.to_path_buf());
    }
    repo::migrate_to_flat_inplace(root, slug).map_err(MutateError::Io)?;
    match repo::resolve_layout(root, slug) {
        LayoutState::Flat { item_path } | LayoutState::Inbox { item_path } => Ok(item_path),
        LayoutState::Absent => Err(MutateError::NotFound),
        LayoutState::Ambiguous { paths } => Err(MutateError::AmbiguousSlug { paths }),
        LayoutState::Invalid { reason, .. } => Err(MutateError::Io(anyhow!("{reason}"))),
        LayoutState::Legacy { .. } => Err(MutateError::Io(anyhow!(
            "post-migration state still classifies as legacy"
        ))),
    }
}

/// Lift an issue out of cold storage when a mutation leaves it in an
/// active (non-closing) state. When `post_closing` is false and the
/// issue's `item.md` currently lives under `issues/archive/YYYY/MM/`,
/// rename the issue directory back to the active root (`issues/<slug>/`)
/// — the inverse of the `archive` move. Returns the new `item.md` path
/// so the subsequent write lands on the active copy. No-op for issues
/// that aren't archived or whose post-mutation status is still closing.
///
/// The trigger is "archived AND now active", not strictly "reopened
/// (closing→active)": that also heals an archived issue whose status was
/// dragged active out-of-band (manual edit / external git op) and then
/// touched by an unrelated PATCH — `resolve_layout` already reads such an
/// issue as active, so leaving it physically archived is the same bug.
///
/// Runs under the caller's held flock and only after validation passed,
/// so it shares the archive move's all-or-nothing guarantee. Refuses
/// (rather than clobbering) if an active directory for the slug already
/// exists — that collision is `Ambiguous` and would have failed the
/// read-time locate, but the check is kept as defence in depth.
///
/// Failure mode if the later `write_item_atomic` errors after this
/// rename: the dir is at the active root carrying its still-closing
/// pre-mutation `item.md`, i.e. a closed-but-unarchived issue — a
/// self-consistent state (closed issues live at the active root until
/// the next `archive` run), not the "active-but-archived" inconsistency
/// this fix targets. Re-running the mutation completes cleanly.
fn unarchive_if_active(
    root: &Path,
    slug: &str,
    item_path: PathBuf,
    post_closing: bool,
) -> Result<PathBuf, MutateError> {
    if post_closing {
        return Ok(item_path);
    }
    // `archive_root` and `cur_dir` are both derived by joining the same
    // `root` (`item_path` came from `resolve_layout(root, …)`), so the
    // `starts_with` prefix test is robust to whatever base `root` carries
    // (relative, symlinked) — both sides share it.
    let archive_root = root.join("issues").join(repo::ARCHIVE_DIR);
    let cur_dir = item_path.parent().ok_or_else(|| {
        MutateError::Io(anyhow!("item.md has no parent: {}", item_path.display()))
    })?;
    if !cur_dir.starts_with(&archive_root) {
        return Ok(item_path); // not archived — nothing to lift
    }
    let dest_dir = root.join("issues").join(slug);
    if dest_dir.exists() {
        return Err(MutateError::Io(anyhow!(
            "cannot unarchive {slug}: active destination already exists: {} — resolve manually",
            dest_dir.display()
        )));
    }
    fs::rename(cur_dir, &dest_dir).map_err(|e| {
        MutateError::Io(anyhow!(
            "cannot unarchive {slug}: rename {} -> {} failed: {e}",
            cur_dir.display(),
            dest_dir.display()
        ))
    })?;
    // Best-effort prune of the now-possibly-empty YYYY/MM (and YYYY)
    // buckets, symmetric with the `archive` move creating them.
    // `remove_dir` only removes empty dirs, so a bucket still holding
    // other archived issues is left untouched.
    prune_empty_archive_buckets(cur_dir, &archive_root);
    Ok(dest_dir.join("item.md"))
}

/// Remove the now-orphaned `YYYY/MM` (then `YYYY`) archive bucket dirs
/// after a slug dir was moved out, walking up but never past — or onto —
/// `archive_root`. Best-effort: any non-empty dir stops the walk.
fn prune_empty_archive_buckets(moved_dir: &Path, archive_root: &Path) {
    let mut cur = moved_dir.parent();
    while let Some(dir) = cur {
        if dir == archive_root || !dir.starts_with(archive_root) {
            break;
        }
        if fs::remove_dir(dir).is_err() {
            break; // non-empty or gone — stop pruning
        }
        cur = dir.parent();
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
    ///
    /// JSON shape: an object (`{"team": "payments"}`). Duplicate keys
    /// in the wire payload are rejected during deserialization so
    /// `POST /api/issues` enforces the same invariant the CLI
    /// `--field foo=a --field foo=b` rejection enforces — calling
    /// agents need a deterministic error rather than silent last-write-
    /// wins behavior.
    #[serde(default, deserialize_with = "deserialize_custom_fields_no_dups")]
    pub custom_fields: Vec<(String, String)>,
}

fn deserialize_custom_fields_no_dups<'de, D>(de: D) -> Result<Vec<(String, String)>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{MapAccess, Visitor};
    use std::fmt;

    struct CustomFieldsVisitor;
    impl<'de> Visitor<'de> for CustomFieldsVisitor {
        type Value = Vec<(String, String)>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an object of custom field key=value pairs with no duplicate keys")
        }
        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            // serde_json's MapAccess yields raw JSON object entries in
            // input order — duplicates are NOT pre-deduplicated by the
            // parser, so we see both and can reject. Switching to
            // BTreeMap here would silently keep the last value.
            //
            // Pull `next_key` and `next_value` separately so the
            // duplicate-key check fires BEFORE value deserialization;
            // otherwise a payload like `{"team":"a","team":1}` would
            // surface a type error from the bad second value rather
            // than the duplicate-key diagnostic the test pins.
            let mut out: Vec<(String, String)> = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            while let Some(k) = map.next_key::<String>()? {
                if !seen.insert(k.clone()) {
                    return Err(serde::de::Error::custom(format!(
                        "custom field {k:?} given more than once"
                    )));
                }
                let v = map.next_value::<String>()?;
                out.push((k, v));
            }
            Ok(out)
        }
    }

    de.deserialize_map(CustomFieldsVisitor)
}

/// Sister of `deserialize_custom_fields_no_dups` for the update path,
/// where the wire shape is `{key: Patch<String>}` instead of
/// `{key: String}`. Same duplicate-key rejection contract — without it
/// a `PATCH {"custom_fields": {"team":"a","team":null}}` would silently
/// keep whichever entry `serde_json` saw last.
fn deserialize_patch_map_no_dups<'de, D>(
    de: D,
) -> Result<std::collections::BTreeMap<String, Patch<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{MapAccess, Visitor};
    use std::fmt;

    struct PatchMapVisitor;
    impl<'de> Visitor<'de> for PatchMapVisitor {
        type Value = std::collections::BTreeMap<String, Patch<String>>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an object of custom field key=value pairs with no duplicate keys")
        }
        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut out: std::collections::BTreeMap<String, Patch<String>> =
                std::collections::BTreeMap::new();
            while let Some(k) = map.next_key::<String>()? {
                if out.contains_key(&k) {
                    return Err(serde::de::Error::custom(format!(
                        "custom field {k:?} given more than once"
                    )));
                }
                let v = map.next_value::<Patch<String>>()?;
                out.insert(k, v);
            }
            Ok(out)
        }
    }

    de.deserialize_map(PatchMapVisitor)
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
    config: &dyn ConfigSource,
) -> Result<NewOutcome, MutateError> {
    if req.title.trim().is_empty() {
        return Err(MutateError::Validation("title cannot be empty".into()));
    }
    if !crate::issue_fields::ISSUE_TYPES
        .iter()
        .any(|t| t == &req.issue_type)
    {
        return Err(MutateError::Validation(format!(
            "type {:?} is not one of the known types",
            req.issue_type
        )));
    }
    if !crate::issue_fields::PRIORITIES
        .iter()
        .any(|p| p == &req.priority)
    {
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
    let outcome = new_issue::do_new_locked(
        &lock,
        root,
        new_issue::NewArgs {
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
            custom_fields: req.custom_fields,
            status: None,
            inbox: false,
        },
        config,
    )
    .map_err(MutateError::from)?;

    // Re-read for canonical hash + Issue. Still holding the lock.
    let parsed =
        crate::parser::parse_item_md_with_warnings(&outcome.item_path, &outcome.slug, "open");
    let mut issue = parsed.issue;
    let schema = config
        .schema(root)
        .map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    issue.folder = folder_for_status(&schema, &issue.status).to_string();
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
    use crate::repo_config::UncachedConfig;
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
        let schema = crate::schema::default_schema();
        issue.folder = crate::repo::folder_for_status(&schema, &issue.status).to_string();
        canonical_hash(&issue)
    }

    #[test]
    fn depend_add_writes_blocked_by_and_normalizes_at_sigil() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "subject-issue-here", "open");
        seed_issue(tmp.path(), "open", "blocker-one-here", "open");
        let req = UpdateIssueRequest {
            add_blocked_by: vec!["@blocker-one-here".into()],
            ..Default::default()
        };
        update_issue(tmp.path(), "subject-issue-here", req, None, &UncachedConfig).unwrap();
        let after =
            fs::read_to_string(tmp.path().join("issues/subject-issue-here/item.md")).unwrap();
        // Normalization strips the sigil before writing.
        assert!(
            after.contains("blocked_by:") && after.contains("blocker-one-here"),
            "{after}"
        );
    }

    #[test]
    fn depend_remove_drops_blocker_and_removes_empty_key() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "subject-issue-here", "open");
        seed_issue(tmp.path(), "open", "blocker-one-here", "open");
        let add = UpdateIssueRequest {
            add_blocked_by: vec!["blocker-one-here".into()],
            ..Default::default()
        };
        update_issue(tmp.path(), "subject-issue-here", add, None, &UncachedConfig).unwrap();
        let remove = UpdateIssueRequest {
            remove_blocked_by: vec!["blocker-one-here".into()],
            ..Default::default()
        };
        update_issue(
            tmp.path(),
            "subject-issue-here",
            remove,
            None,
            &UncachedConfig,
        )
        .unwrap();
        let after =
            fs::read_to_string(tmp.path().join("issues/subject-issue-here/item.md")).unwrap();
        assert!(
            !after.contains("blocked_by:"),
            "empty list must drop the key: {after}"
        );
    }

    #[test]
    fn depend_rejects_self_blocker() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "loop-target-here", "open");
        let req = UpdateIssueRequest {
            add_blocked_by: vec!["loop-target-here".into()],
            ..Default::default()
        };
        let err =
            update_issue(tmp.path(), "loop-target-here", req, None, &UncachedConfig).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cannot block itself"), "got: {msg}");
    }

    #[test]
    fn depend_add_and_remove_overlap_is_conflicting_intent() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "subject-issue-here", "open");
        let req = UpdateIssueRequest {
            add_blocked_by: vec!["blocker-x-here".into()],
            remove_blocked_by: vec!["blocker-x-here".into()],
            ..Default::default()
        };
        let err =
            update_issue(tmp.path(), "subject-issue-here", req, None, &UncachedConfig).unwrap_err();
        assert!(matches!(err, MutateError::ConflictingIntent(_)));
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
        let out = update_issue(tmp.path(), "test-slug-one", req, None, &UncachedConfig).unwrap();
        assert!(out.version.starts_with("sha256:"));
        let after = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(after.contains("priority: high"));
    }

    #[test]
    fn schema_override_of_built_in_done_to_active_drops_closed_stamp() {
        // A project that re-classifies the built-in `done` as active
        // via `status_classes:` must see closing→active edge clear
        // `closed:`. Pins down the override-permitted policy
        // documented in `schema::status_class`.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nstatus_classes:\n  done: active\n",
        )
        .unwrap();
        // Seed an issue that's already at `done` with `closed:` stamped.
        let dir = tmp.path().join("issues/done-active-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: done\nclosed: 2026-05-06\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        // Move it to `done` again as if the project just re-classified
        // the status — actually we bump priority so the file rewrites.
        let req = UpdateIssueRequest {
            status: Patch::Set("done".into()),
            ..Default::default()
        };
        let out =
            update_issue(tmp.path(), "done-active-target", req, None, &UncachedConfig).unwrap();
        // Now `done` is active, so the lifecycle treats this as
        // active→active → no moved_to_closed, and `closed:` should be
        // dropped because the new status is classified Active.
        assert!(!out.moved_to_closed);
        let after =
            fs::read_to_string(tmp.path().join("issues/done-active-target/item.md")).unwrap();
        assert!(
            !after.contains("closed:"),
            "schema-overridden active `done` must drop closed:; got:\n{after}"
        );
        assert_eq!(out.issue.folder, "open");
    }

    #[test]
    fn update_with_no_enum_status_field_rejects_unknown_status() {
        // Whole-spec replacement of `fields.status` without an `enum:`
        // used to leave `status: pizza` to land. The under-lock
        // `status_universe` belt-and-braces gate now catches it
        // (falls back to built-in all_statuses() when no enum is
        // declared).
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\n",
        )
        .unwrap();
        let _v0 = seed_issue(tmp.path(), "open", "no-enum-target", "open");
        let req = UpdateIssueRequest {
            status: Patch::Set("pizza".into()),
            ..Default::default()
        };
        let err =
            update_issue(tmp.path(), "no-enum-target", req, None, &UncachedConfig).unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => {
                assert!(
                    msg.contains("pizza"),
                    "expected pizza in violation message: {msg}"
                );
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn update_to_schema_declared_custom_closing_status_stamps_closed() {
        // A project that adds `archived` to its schema's status enum
        // and declares it as closing must get the full lifecycle
        // treatment: `closed:` stamped, folder = "closed",
        // `moved_to_closed` reported. Regression-anchors the
        // schema-derived classifier replacing the static
        // CLOSING_STATUSES list.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\n    enum: [open, in-progress, archived]\nstatus_classes:\n  archived: closing\n",
        )
        .unwrap();
        let _v0 = seed_issue(tmp.path(), "open", "archive-target", "open");
        let req = UpdateIssueRequest {
            status: Patch::Set("archived".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "archive-target", req, None, &UncachedConfig).unwrap();
        assert!(
            out.moved_to_closed,
            "active→archived must report moved_to_closed"
        );
        assert_eq!(out.issue.folder, "closed");
        let after = fs::read_to_string(tmp.path().join("issues/archive-target/item.md")).unwrap();
        assert!(after.contains("status: archived"));
        assert!(
            after.contains("closed:"),
            "closed: must be stamped on schema-classified closing status; got:\n{after}"
        );
    }

    #[test]
    fn status_write_rejects_empty_closed_on_closing_status() {
        // An issue at a closing status whose `closed:` is empty (an
        // explicit unset). Re-asserting a closing status *touches* the
        // status/closed pair, so the resulting RequiredWhen is one this
        // write is responsible for — it must be rejected, not silently
        // accepted and left for `doctor` to heal.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/empty-closed-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: done\nclosed: \"\"\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            status: Patch::Set("done".into()),
            ..Default::default()
        };
        let err = update_issue(
            tmp.path(),
            "empty-closed-target",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => {
                assert!(
                    msg.contains("closed"),
                    "expected the closed RequiredWhen in the message: {msg}"
                );
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn unrelated_edit_keeps_requiredwhen_exempt_on_closing_status() {
        // The mirror case: the same inconsistent on-disk state, but the
        // mutation only bumps `priority` — it touches neither `status`
        // nor `closed`. Blocking an unrelated edit because of a
        // pre-existing inconsistency the user didn't introduce would be
        // surprising, so the RequiredWhen stays exempt and the write
        // succeeds (doctor owns healing the empty `closed:`).
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/unrelated-edit-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: done\nclosed: \"\"\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let out = update_issue(
            tmp.path(),
            "unrelated-edit-target",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap();
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
        let err =
            update_issue(tmp.path(), "test-slug-two", req, None, &UncachedConfig).unwrap_err();
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
        let out = update_issue(
            tmp.path(),
            "concurrent-distinct",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap();
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
        let err = update_issue(
            tmp.path(),
            "concurrent-same-key",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap_err();
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
        let err = update_issue(
            tmp.path(),
            "concurrent-delete-key",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap_err();
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
        let err =
            update_issue(tmp.path(), "bad-nested-key", req, None, &UncachedConfig).unwrap_err();
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
        let out = update_issue(tmp.path(), "dnd-status-only", req, None, &UncachedConfig).unwrap();
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
        let out = update_issue(tmp.path(), "reopen-me", req, None, &UncachedConfig).unwrap();
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

    /// Seed an issue directly into cold storage at
    /// `issues/archive/YYYY/MM/<slug>/item.md`, as the `archive` verb
    /// would leave it.
    fn seed_archived(root: &Path, year: &str, month: &str, slug: &str, body: &str) -> PathBuf {
        let dir = root
            .join("issues/archive")
            .join(year)
            .join(month)
            .join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), body).unwrap();
        dir
    }

    #[test]
    fn reopening_archived_issue_lifts_it_out_of_cold_storage() {
        // The arch-stale-archive feature moves closed issues to
        // issues/archive/YYYY/MM/<slug>/. Reopening one (closing→active)
        // must move the directory back to the active root, else the issue
        // reads as active in list/show while physically still archived.
        let tmp = fresh_repo();
        let archived_dir = seed_archived(
            tmp.path(),
            "2020",
            "01",
            "old-archived-fox",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# Title\n",
        );

        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "old-archived-fox", req, None, &UncachedConfig).unwrap();

        assert!(out.moved_to_open);
        assert_eq!(out.issue.status, "open");
        // Physically relocated to the active root.
        let active_dir = tmp.path().join("issues/old-archived-fox");
        assert_eq!(out.issue_dir, active_dir, "issue_dir must be active root");
        assert!(
            active_dir.join("item.md").is_file(),
            "active copy must exist"
        );
        assert!(!archived_dir.exists(), "archive copy must be gone");
        let on_disk = fs::read_to_string(active_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("status: open"));
        assert!(!on_disk.contains("closed:"), "closed: cleared on reopen");
        // No leftover empty archive month/year/root tree is required, but
        // the slug dir itself must not linger.
        assert!(!tmp
            .path()
            .join("issues/archive/2020/01/old-archived-fox")
            .exists());
    }

    #[test]
    fn reopening_archived_issue_via_close_status_change_stays_in_archive() {
        // Changing one closing status to another (fixed→wontfix) is NOT a
        // reopen: the issue stays closed and must remain in cold storage.
        let tmp = fresh_repo();
        let archived_dir = seed_archived(
            tmp.path(),
            "2020",
            "01",
            "still-closed-elk",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# Title\n",
        );

        let req = UpdateIssueRequest {
            status: Patch::Set("wontfix".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "still-closed-elk", req, None, &UncachedConfig).unwrap();

        assert!(!out.moved_to_open);
        assert!(archived_dir.join("item.md").is_file(), "stays archived");
        assert!(
            !tmp.path().join("issues/still-closed-elk").exists(),
            "must not appear in active root"
        );
        // Historical close date preserved on closing→closing.
        let on_disk = fs::read_to_string(archived_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("status: wontfix"));
        assert!(on_disk.contains("closed: 2020-01-01"));
    }

    #[test]
    fn reopening_archived_issue_refuses_when_active_copy_exists() {
        // Defence in depth: a slug present both active and archived is
        // Ambiguous and fails the read-time locate, but if it somehow got
        // past, the unarchive move must refuse rather than clobber.
        let tmp = fresh_repo();
        seed_archived(
            tmp.path(),
            "2020",
            "01",
            "dup-slug-newt",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# Title\n",
        );
        // Also seed an active copy so the locate is Ambiguous.
        let active = tmp.path().join("issues/dup-slug-newt");
        fs::create_dir_all(&active).unwrap();
        fs::write(
            active.join("item.md"),
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: open\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();

        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        let err =
            update_issue(tmp.path(), "dup-slug-newt", req, None, &UncachedConfig).unwrap_err();
        // Ambiguous locate fires before the write; either way both copies
        // survive untouched.
        assert!(matches!(err, MutateError::AmbiguousSlug { .. }));
        assert!(tmp
            .path()
            .join("issues/archive/2020/01/dup-slug-newt/item.md")
            .is_file());
        assert!(active.join("item.md").is_file());
    }

    #[test]
    fn unarchive_if_active_is_noop_when_post_status_closing() {
        // A mutation that leaves the issue closing (post_closing == true)
        // on an archived issue must leave the path untouched.
        let tmp = fresh_repo();
        let item = tmp
            .path()
            .join("issues/archive/2020/01/keep-archived-owl/item.md");
        let out = unarchive_if_active(tmp.path(), "keep-archived-owl", item.clone(), true).unwrap();
        assert_eq!(
            out, item,
            "still-closing leaves the archived path unchanged"
        );
    }

    #[test]
    fn unarchive_if_active_refuses_when_active_copy_exists() {
        // Defence-in-depth branch: exercised directly because the normal
        // entry points reject the active+archived collision as Ambiguous
        // before this helper runs.
        let tmp = fresh_repo();
        let archived = seed_archived(
            tmp.path(),
            "2020",
            "01",
            "collide-fox",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\npriority: normal\n---\n# T\n",
        );
        let active = tmp.path().join("issues/collide-fox");
        fs::create_dir_all(&active).unwrap();
        let err = unarchive_if_active(tmp.path(), "collide-fox", archived.join("item.md"), false)
            .unwrap_err();
        assert!(matches!(err, MutateError::Io(_)));
        assert!(archived.join("item.md").exists(), "archive copy untouched");
    }

    #[test]
    fn dry_run_reopen_of_archived_issue_predicts_active_dir() {
        let tmp = fresh_repo();
        seed_archived(
            tmp.path(),
            "2020",
            "01",
            "dry-reopen-fox",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# T\n",
        );
        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "dry-reopen-fox", req, None, &UncachedConfig).unwrap();
        assert_eq!(out.issue_dir, tmp.path().join("issues/dry-reopen-fox"));
        // Dry-run wrote nothing: still physically archived.
        assert!(tmp
            .path()
            .join("issues/archive/2020/01/dry-reopen-fox/item.md")
            .is_file());
        assert!(!tmp.path().join("issues/dry-reopen-fox").exists());
    }

    #[test]
    fn dry_run_non_reopen_of_archived_issue_predicts_archive_dir() {
        // A priority patch that keeps the issue closing must report the
        // archive dir, matching where a real write would land.
        let tmp = fresh_repo();
        let archived = seed_archived(
            tmp.path(),
            "2020",
            "01",
            "dry-stay-elk",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# T\n",
        );
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "dry-stay-elk", req, None, &UncachedConfig).unwrap();
        assert_eq!(
            out.issue_dir, archived,
            "non-reopen dry-run must predict the archive dir"
        );
    }

    #[test]
    fn unarchive_prunes_emptied_month_and_year_buckets() {
        let tmp = fresh_repo();
        seed_archived(
            tmp.path(),
            "2020",
            "01",
            "lonely-newt",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# T\n",
        );
        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "lonely-newt", req, None, &UncachedConfig).unwrap();
        // The slug was the only occupant, so its month/year buckets prune.
        assert!(!tmp.path().join("issues/archive/2020/01").exists());
        assert!(!tmp.path().join("issues/archive/2020").exists());
    }

    #[test]
    fn unarchive_keeps_bucket_with_other_archived_siblings() {
        let tmp = fresh_repo();
        seed_archived(
            tmp.path(),
            "2020",
            "01",
            "reopen-this-fox",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# T\n",
        );
        let sibling = seed_archived(
            tmp.path(),
            "2020",
            "01",
            "stay-put-owl",
            "---\ntype: bug\ncreated: 2020-01-15\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-15\n---\n\n# T\n",
        );
        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "reopen-this-fox", req, None, &UncachedConfig).unwrap();
        // Bucket still holds the sibling, so it must NOT be pruned.
        assert!(tmp.path().join("issues/archive/2020/01").exists());
        assert!(sibling.join("item.md").is_file());
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
        let out = update_issue(tmp.path(), "close-me-now", req, None, &UncachedConfig).unwrap();
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
        let out =
            update_issue(tmp.path(), "empty-patch-legacy", req, None, &UncachedConfig).unwrap();

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
        let _ = update_issue(
            tmp.path(),
            "preserve-closed-date",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap();
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
        let _ = update_issue(tmp.path(), "backfill-closed", req, None, &UncachedConfig).unwrap();
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
        let out = update_issue(tmp.path(), "legacy-one-here", req, None, &UncachedConfig).unwrap();
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
        let err =
            update_issue(tmp.path(), "dual-path-here", req, None, &UncachedConfig).unwrap_err();
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
        let _ = update_issue(tmp.path(), "has-epic-here", req, None, &UncachedConfig).unwrap();
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
        let _ = update_issue(tmp.path(), "keep-epic-as-is", req, None, &UncachedConfig).unwrap();
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
        let out = update_issue(tmp.path(), "cf-set", req, None, &UncachedConfig).unwrap();
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
        let out = update_issue(tmp.path(), "cf-clear", req, None, &UncachedConfig).unwrap();
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
        let req: UpdateIssueRequest =
            serde_json::from_str(r#"{"custom_fields": {"triage": "P1", "owner_team": null}}"#)
                .unwrap();
        let out = update_issue(tmp.path(), "cf-mixed", req, None, &UncachedConfig).unwrap();
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
        let err = update_issue(tmp.path(), "cf-schema", req, None, &UncachedConfig).unwrap_err();
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
        let out = update_issue(tmp.path(), "cf-bump", req, None, &UncachedConfig).unwrap();
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
        let err = update_issue(tmp.path(), "cf-stale", req, None, &UncachedConfig).unwrap_err();
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
            &UncachedConfig,
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
        let out =
            update_issue(tmp.path(), "cf-required-repair", req, None, &UncachedConfig).unwrap();
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
        let r: UpdateIssueRequest =
            serde_json::from_str(r#"{"custom_fields": {"a": "x", "b": null}}"#).unwrap();
        assert!(matches!(r.custom_fields.get("a"), Some(Patch::Set(s)) if s == "x"));
        assert!(matches!(r.custom_fields.get("b"), Some(Patch::Clear)));
    }

    #[test]
    fn update_request_rejects_duplicate_custom_field_keys_at_deserialization() {
        // Sister of the create-path duplicate-key rejection. Without a
        // custom visitor, BTreeMap silently keeps whichever value
        // serde_json saw last — this test pins the wire-level rejection.
        let payload = r#"{"custom_fields":{"team":"a","team":null}}"#;
        let err = serde_json::from_str::<UpdateIssueRequest>(payload).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("team") && msg.contains("more than once"),
            "expected duplicate-key rejection, got {msg:?}"
        );
    }

    #[test]
    fn validate_custom_field_key_rejects_invalid_shape_and_reserved() {
        // Single source of truth for the four create/update paths.
        // Each call site (CLI/API × new/update) wraps this in its own
        // typed error variant, so the message must stay stable.
        assert!(validate_custom_field_key("team").is_ok());
        let err = validate_custom_field_key("bad key").unwrap_err();
        assert!(err.contains("alphanumeric"), "shape rejection: {err:?}");
        let err = validate_custom_field_key("status").unwrap_err();
        assert!(err.contains("built-in"), "reserved rejection: {err:?}");
    }

    #[test]
    fn update_request_validate_rejects_reserved_custom_field_key() {
        // CLI-update + API-update share `UpdateIssueRequest::validate`.
        // Routing through `validate_custom_field_key` keeps the error
        // text identical to the new-path rejection.
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("status".into(), Patch::Set("ignored".into()));
        let err = req.validate().unwrap_err();
        match err {
            MutateError::Validation(msg) => assert!(
                msg.contains("status") && msg.contains("built-in"),
                "expected built-in rejection, got {msg:?}"
            ),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn new_issue_api_rejects_reserved_custom_field_key() {
        // API new path: previously accepted `{"custom_fields":
        // {"status":"…"}}` and let frontmatter-render ordering mask the
        // damage. Now `do_new_locked` runs the shared validator before
        // building the in-memory frontmatter, so the API surfaces the
        // same MutateError::Validation as the update path.
        let tmp = fresh_repo();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Sneaky".into();
        req.priority = "normal".into();
        req.custom_fields = vec![("status".into(), "fake".into())];
        let err = new_issue(tmp.path(), req, None, &UncachedConfig).unwrap_err();
        match err {
            MutateError::Validation(msg) => assert!(
                msg.contains("status") && msg.contains("built-in"),
                "expected built-in rejection, got {msg:?}"
            ),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn validate_custom_field_value_rejects_blank_and_padded() {
        assert!(validate_custom_field_value("team", "payments").is_ok());
        let err = validate_custom_field_value("team", "   ").unwrap_err();
        assert!(err.contains("empty-string Set"), "blank: {err:?}");
        let err = validate_custom_field_value("team", " payments").unwrap_err();
        assert!(err.contains("whitespace"), "leading ws: {err:?}");
        let err = validate_custom_field_value("team", "payments ").unwrap_err();
        assert!(err.contains("whitespace"), "trailing ws: {err:?}");
    }

    #[test]
    fn new_issue_api_rejects_whitespace_only_custom_field_value() {
        // Closes the value-validation asymmetry: API update already
        // rejected `{"team":"   "}` via UpdateIssueRequest::validate,
        // and CLI new rejected it via parse_custom_field. API new used
        // to slip blank values through to frontmatter — now both key
        // and value go through the shared validators inside
        // `do_new_locked`.
        let tmp = fresh_repo();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Blank value".into();
        req.priority = "normal".into();
        req.custom_fields = vec![("team".into(), "   ".into())];
        let err = new_issue(tmp.path(), req, None, &UncachedConfig).unwrap_err();
        match err {
            MutateError::Validation(msg) => assert!(
                msg.contains("team") && msg.contains("empty-string Set"),
                "expected blank-value rejection, got {msg:?}"
            ),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn new_issue_api_rejects_padded_custom_field_value() {
        let tmp = fresh_repo();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Padded value".into();
        req.priority = "normal".into();
        req.custom_fields = vec![("team".into(), " payments".into())];
        let err = new_issue(tmp.path(), req, None, &UncachedConfig).unwrap_err();
        match err {
            MutateError::Validation(msg) => assert!(
                msg.contains("team") && msg.contains("whitespace"),
                "expected whitespace rejection, got {msg:?}"
            ),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn new_issue_api_rejects_invalid_custom_field_key_shape() {
        let tmp = fresh_repo();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Bad shape".into();
        req.priority = "normal".into();
        req.custom_fields = vec![("bad key".into(), "x".into())];
        let err = new_issue(tmp.path(), req, None, &UncachedConfig).unwrap_err();
        match err {
            MutateError::Validation(msg) => assert!(
                msg.contains("bad key") && msg.contains("alphanumeric"),
                "expected shape rejection, got {msg:?}"
            ),
            other => panic!("expected Validation, got {other:?}"),
        }
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
            &UncachedConfig,
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
            &UncachedConfig,
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
        let _ = update_issue(tmp.path(), "reopen-section", req, None, &UncachedConfig).unwrap();
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
            &UncachedConfig,
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
            &UncachedConfig,
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
            &UncachedConfig,
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
        update_issue(tmp.path(), "bootstrap-target", req, None, &UncachedConfig).unwrap();
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
        let err =
            update_issue(tmp.path(), "label-enum-target", req, None, &UncachedConfig).unwrap_err();
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
            &UncachedConfig,
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
        req.custom_fields.push(("team".into(), "payments".into()));
        let outcome = new_issue(tmp.path(), req, None, &UncachedConfig).unwrap();
        let on_disk = fs::read_to_string(outcome.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("team: payments"), "got {on_disk}");
    }

    #[test]
    fn new_issue_request_defaults_missing_custom_fields_to_empty() {
        let req: NewIssueRequest = serde_json::from_str(r#"{"type":"bug","title":"x"}"#).unwrap();
        assert!(req.custom_fields.is_empty());
    }

    #[test]
    fn new_issue_request_accepts_empty_custom_fields_object() {
        let req: NewIssueRequest =
            serde_json::from_str(r#"{"type":"bug","title":"x","custom_fields":{}}"#).unwrap();
        assert!(req.custom_fields.is_empty());
    }

    #[test]
    fn new_issue_request_rejects_non_object_custom_fields() {
        // `custom_fields: []` (or any non-object shape) must be rejected
        // with the visitor's `expecting` text so calling agents get a
        // shape-error message rather than silent acceptance.
        let err = serde_json::from_str::<NewIssueRequest>(
            r#"{"type":"bug","title":"x","custom_fields":[]}"#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("expected"),
            "expected shape error, got {err}"
        );
    }

    #[test]
    fn new_issue_request_rejects_non_string_custom_field_value() {
        let err = serde_json::from_str::<NewIssueRequest>(
            r#"{"type":"bug","title":"x","custom_fields":{"team":1}}"#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("expected a string"),
            "expected string-type error, got {err}"
        );
    }

    #[test]
    fn new_issue_request_duplicate_key_error_precedes_bad_duplicate_value() {
        // Pinning the next_key/next_value ordering: a duplicate key
        // with a type-invalid second value must report duplicate, not
        // the type error. Otherwise the duplicate-rejection invariant
        // would be silently bypassed by anyone whose duplicate
        // happens to also be malformed.
        let err = serde_json::from_str::<NewIssueRequest>(
            r#"{"type":"bug","title":"x","custom_fields":{"team":"a","team":1}}"#,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("team") && msg.contains("more than once"),
            "expected duplicate-key error, got {msg:?}"
        );
    }

    #[test]
    fn new_issue_request_rejects_duplicate_custom_field_keys_at_deserialization() {
        // Calling agents that build their JSON dynamically can produce a
        // payload with two `team:` entries; rather than silent
        // last-write-wins (BTreeMap behavior), the wire deserializer
        // rejects duplicates to mirror CLI `--field foo=a --field foo=b`
        // rejection. This is the API-side enforcement of the
        // `do_new_locked` invariant.
        let payload = r#"{"type":"bug","title":"x","custom_fields":{"team":"a","team":"b"}}"#;
        let err = serde_json::from_str::<NewIssueRequest>(payload).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("team") && msg.contains("more than once"),
            "expected duplicate-key rejection, got {msg:?}"
        );
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
        let err = new_issue(tmp.path(), req, None, &UncachedConfig).unwrap_err();
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
        let err = new_issue(tmp.path(), req, None, &UncachedConfig).unwrap_err();
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
        let err = new_issue(tmp.path(), req, None, &UncachedConfig).unwrap_err();
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
        let err = new_issue(tmp.path(), req, None, &UncachedConfig).unwrap_err();
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
        let err = new_issue(tmp.path(), req, None, &UncachedConfig).unwrap_err();
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
        let err = new_issue(tmp.path(), req, None, &UncachedConfig).unwrap_err();

        assert!(matches!(err, MutateError::Io(_)), "got {err:?}");
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
        new_issue(tmp.path(), req, Some(&hub), &UncachedConfig).unwrap();

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
        update_issue(
            tmp.path(),
            "patch-publish-flock",
            req,
            Some(&hub),
            &UncachedConfig,
        )
        .unwrap();

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
            None,
            Vec::new(),
            Some(v0),
            Some(&hub),
            &UncachedConfig,
        )
        .unwrap();

        assert_probe_saw_held(&observed, "close_issue");
    }

    #[test]
    fn close_with_as_records_closer_in_frontmatter() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-attributed", "open");

        close_issue(
            tmp.path(),
            "close-attributed",
            Some("wontfix".into()),
            Some("jari".into()),
            Vec::new(),
            None,
            None,
            &UncachedConfig,
        )
        .unwrap();

        let after = fs::read_to_string(tmp.path().join("issues/close-attributed/item.md")).unwrap();
        assert!(after.contains("status: wontfix"), "{after}");
        assert!(after.contains("closed_by: jari"), "{after}");
        // The closer surfaces as first-class JSON via `Issue::extra`.
        let parsed = crate::parser::parse_item_md_with_warnings(
            &tmp.path().join("issues/close-attributed/item.md"),
            "close-attributed",
            "open",
        );
        assert_eq!(
            parsed.issue.extra.get("closed_by").and_then(|v| v.as_str()),
            Some("jari")
        );
    }

    #[test]
    fn close_without_as_writes_no_closed_by() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-anon", "open");

        close_issue(
            tmp.path(),
            "close-anon",
            Some("done".into()),
            None,
            Vec::new(),
            None,
            None,
            &UncachedConfig,
        )
        .unwrap();

        let after = fs::read_to_string(tmp.path().join("issues/close-anon/item.md")).unwrap();
        assert!(after.contains("status: done"), "{after}");
        assert!(!after.contains("closed_by"), "{after}");
    }

    #[test]
    fn close_rejects_malformed_as_author() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-bad-author", "open");

        let err = close_issue(
            tmp.path(),
            "close-bad-author",
            Some("wontfix".into()),
            Some("has space".into()),
            Vec::new(),
            None,
            None,
            &UncachedConfig,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "{err:?}");
        // Validation fires before any write — the issue stays open.
        let after = fs::read_to_string(tmp.path().join("issues/close-bad-author/item.md")).unwrap();
        assert!(after.contains("status: open"), "{after}");
    }

    #[test]
    fn reopen_clears_closer_attribution() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "reopen-clears-closer", "open");

        close_issue(
            tmp.path(),
            "reopen-clears-closer",
            Some("wontfix".into()),
            Some("jari".into()),
            Vec::new(),
            None,
            None,
            &UncachedConfig,
        )
        .unwrap();

        // Reopen through the general update path; `closed_by` must drop
        // in lockstep with `closed:` so a reopened issue carries neither.
        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        update_issue(
            tmp.path(),
            "reopen-clears-closer",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap();

        let after =
            fs::read_to_string(tmp.path().join("issues/reopen-clears-closer/item.md")).unwrap();
        assert!(after.contains("status: open"), "{after}");
        assert!(!after.contains("closed_by"), "{after}");
        assert!(!after.contains("closed:"), "{after}");
    }

    #[test]
    fn close_rejects_non_closing_status_override() {
        // `close --status open` must be refused: it is not a closing
        // status, so honoring it would leave the issue active — and with
        // `--as`, would strand a `closed_by` on an open issue.
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-to-open", "open");

        let err = close_issue(
            tmp.path(),
            "close-to-open",
            Some("open".into()),
            Some("jari".into()),
            Vec::new(),
            None,
            None,
            &UncachedConfig,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "{err:?}");
        let after = fs::read_to_string(tmp.path().join("issues/close-to-open/item.md")).unwrap();
        assert!(after.contains("status: open"), "{after}");
        assert!(!after.contains("closed_by"), "{after}");
    }

    #[test]
    fn reopen_with_custom_field_closed_by_is_rejected() {
        // The reopen-clears invariant must be un-defeatable: a request
        // that reopens *and* smuggles `closed_by` through `custom_fields`
        // in the same call is rejected at validation, because `closed_by`
        // is a reserved key. Previously this ordering let the custom-field
        // loop re-add the closer the status branch had just cleared.
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "reopen-smuggle", "open");
        close_issue(
            tmp.path(),
            "reopen-smuggle",
            Some("wontfix".into()),
            Some("jari".into()),
            Vec::new(),
            None,
            None,
            &UncachedConfig,
        )
        .unwrap();

        let mut req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        req.custom_fields
            .insert("closed_by".into(), Patch::Set("mallory".into()));
        let err =
            update_issue(tmp.path(), "reopen-smuggle", req, None, &UncachedConfig).unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "{err:?}");
    }

    #[test]
    fn set_closed_by_via_custom_field_is_rejected() {
        // `closed_by` is reserved, so it cannot be planted on an open
        // issue through the generic custom-field surface (`set` / `update
        // --field`). That keeps the field trustworthy: the only writer is
        // the validated lifecycle slot.
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "plant-closer", "open");

        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("closed_by".into(), Patch::Set("mallory".into()));
        let err = update_issue(tmp.path(), "plant-closer", req, None, &UncachedConfig).unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "{err:?}");
        let after = fs::read_to_string(tmp.path().join("issues/plant-closer/item.md")).unwrap();
        assert!(!after.contains("closed_by"), "{after}");
    }

    #[test]
    fn restatus_between_closing_values_preserves_closer() {
        // fixed → wontfix must keep the recorded closer (and close date)
        // — a re-disposition is not a new close, so provenance survives.
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "restatus-closer", "open");
        close_issue(
            tmp.path(),
            "restatus-closer",
            Some("fixed".into()),
            Some("jari".into()),
            Vec::new(),
            None,
            None,
            &UncachedConfig,
        )
        .unwrap();

        let req = UpdateIssueRequest {
            status: Patch::Set("wontfix".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "restatus-closer", req, None, &UncachedConfig).unwrap();

        let after = fs::read_to_string(tmp.path().join("issues/restatus-closer/item.md")).unwrap();
        assert!(after.contains("status: wontfix"), "{after}");
        assert!(after.contains("closed_by: jari"), "{after}");
    }

    #[test]
    fn anonymous_close_scrubs_preexisting_closer() {
        // If a stray `closed_by` exists on an active issue (e.g. a manual
        // hand-edit of the frontmatter), an anonymous close must not
        // inherit it as false attribution — the active→closing edge
        // scrubs any stale value when no `--as` is given.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/anon-scrub");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\npriority: normal\nclosed_by: ghost\n---\n\n# Title\n",
        )
        .unwrap();

        close_issue(
            tmp.path(),
            "anon-scrub",
            Some("done".into()),
            None,
            Vec::new(),
            None,
            None,
            &UncachedConfig,
        )
        .unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after.contains("status: done"), "{after}");
        assert!(!after.contains("closed_by"), "{after}");
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
            &UncachedConfig,
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
            &UncachedConfig,
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
        let err = new_issue(tmp.path(), req, Some(&hub), &UncachedConfig).unwrap_err();
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
            &UncachedConfig,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::SchemaConfig(_)), "got {err:?}");
    }

    #[test]
    fn update_type_rejects_when_required_sections_missing() {
        // task→feature with `feature` requiring `Plan, Risks` and the
        // seeded body containing only `# Title` must surface a typed
        // `SchemaViolation` naming the missing headings (option 2).
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "type-reject-target", "open");
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n\
             body_sections:\n  feature: [Plan, Risks]\n",
        )
        .unwrap();
        let before =
            fs::read_to_string(tmp.path().join("issues/type-reject-target/item.md")).unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("feature".into()),
            ..Default::default()
        };
        let err =
            update_issue(tmp.path(), "type-reject-target", req, None, &UncachedConfig).unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => {
                assert!(
                    msg.contains("## Plan"),
                    "expected `## Plan` in error, got {msg}"
                );
                assert!(msg.contains("## Risks"), "expected `## Risks`, got {msg}");
                assert!(
                    msg.contains("feature"),
                    "expected new type in error, got {msg}"
                );
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
        let after =
            fs::read_to_string(tmp.path().join("issues/type-reject-target/item.md")).unwrap();
        assert_eq!(before, after, "rejected mutation must not touch disk");
    }

    #[test]
    fn update_type_succeeds_when_all_required_sections_present() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-ok-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: task\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n\
             # Title\n\n## Plan\n\nplan written\n\n## Risks\n\ntracked\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n\
             body_sections:\n  feature: [Plan, Risks]\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("feature".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "type-ok-target", req, None, &UncachedConfig).unwrap();
        let content = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(content.contains("type: feature"));
        // Body untouched: counts unchanged, content preserved.
        assert_eq!(content.matches("## Plan").count(), 1, "got {content}");
        assert_eq!(content.matches("## Risks").count(), 1, "got {content}");
        assert!(content.contains("plan written"));
    }

    #[test]
    fn update_type_change_to_type_with_partial_overlap_is_rejected() {
        // feature→bug where bug requires `Repro Steps` and the body has
        // `## Plan` (from the old type) but no `## Repro Steps`. The
        // call must be rejected even though some sections are present —
        // schema requirements are evaluated against the new type only.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-overlap-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: feature\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n\
             # Title\n\n## Plan\n\nplan content\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n\
             body_sections:\n  feature: [Plan]\n  bug: [\"Repro Steps\"]\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("bug".into()),
            ..Default::default()
        };
        let err = update_issue(
            tmp.path(),
            "type-overlap-target",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => {
                assert!(msg.contains("Repro Steps"), "got {msg}");
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn update_type_to_looser_target_succeeds_and_leaves_old_stubs() {
        // feature→task where `task` has no required sections is
        // allowed; old stubs from `feature` are deliberately not pruned
        // (documented in AGENTS.md as "type change does not prune").
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-loose-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: feature\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n\
             # Title\n\n## Plan\n\nplan content\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n\
             body_sections:\n  feature: [Plan]\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("task".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "type-loose-target", req, None, &UncachedConfig).unwrap();
        let content = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(content.contains("type: task"));
        // Old stubs from `feature` remain — by design.
        assert!(content.contains("## Plan"));
        assert!(content.contains("plan content"));
    }

    #[test]
    fn update_type_same_value_skips_invariant_and_section_checks() {
        // Idempotent JSON clients sending the current type must not
        // trip the new checks. Body is intentionally missing `## Plan`,
        // and there's an `assignee` (which would block a `feature→epic`
        // change). With Patch::Set("feature") on an already-feature
        // issue, none of those should fire.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-same-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: feature\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\nassignee: bob\n---\n\n# Title\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n\
             body_sections:\n  feature: [Plan]\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("feature".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "type-same-target", req, None, &UncachedConfig).unwrap();
    }

    #[test]
    fn update_type_combined_with_reopen_is_rejected() {
        // Closed issue + status:open + type change in the same call
        // returns `Validation` and does not write. C4: reopen and
        // type-change must be split into separate calls.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-reopen-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: task\ncreated: 2026-05-06\nstatus: fixed\n\
             priority: normal\nclosed: 2026-05-06\n---\n\n# Title\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n\
             body_sections:\n  feature: [Plan]\n",
        )
        .unwrap();
        let before = fs::read_to_string(dir.join("item.md")).unwrap();
        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            issue_type: Patch::Set("feature".into()),
            ..Default::default()
        };
        let err =
            update_issue(tmp.path(), "type-reopen-target", req, None, &UncachedConfig).unwrap_err();
        match err {
            MutateError::Validation(msg) => {
                assert!(msg.contains("reopen"), "got {msg}");
                assert!(msg.contains("--type"), "got {msg}");
            }
            other => panic!("expected Validation, got {other:?}"),
        }
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert_eq!(before, after, "rejected combined call must not touch disk");
    }

    #[test]
    fn update_type_to_epic_with_assignee_is_rejected() {
        // D1: `--type epic` on an issue with an assignee must be
        // rejected; mirrors `cmd_new`'s epic invariant.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-d1-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: task\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\nassignee: alice\n---\n\n# Title\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("epic".into()),
            ..Default::default()
        };
        let err =
            update_issue(tmp.path(), "type-d1-target", req, None, &UncachedConfig).unwrap_err();
        match err {
            MutateError::Validation(msg) => {
                assert!(
                    msg.contains("owner") && msg.contains("assignee"),
                    "got {msg}"
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn update_type_away_from_epic_with_owner_is_rejected() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-d1-epic-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: epic\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\nowner: cara\n---\n\n# Title\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("task".into()),
            ..Default::default()
        };
        let err = update_issue(
            tmp.path(),
            "type-d1-epic-target",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap_err();
        match err {
            MutateError::Validation(msg) => {
                assert!(msg.contains("owner"), "got {msg}");
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn update_type_accepts_schema_extended_type() {
        // C1: a custom schema declaring `type.enum: [bug, task, spike]`
        // must allow `--type spike` end-to-end. Pre-fix this hit the
        // hardcoded `ISSUE_TYPES` check in `validate()` and returned
        // `Validation("not one of the known types")`.
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "type-custom-target", "open");
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n    \
             enum: [bug, task, spike]\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("spike".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "type-custom-target", req, None, &UncachedConfig).unwrap();
        let content =
            fs::read_to_string(tmp.path().join("issues/type-custom-target/item.md")).unwrap();
        assert!(content.contains("type: spike"), "got {content}");
    }

    #[test]
    fn update_type_clear_is_rejected() {
        let req = UpdateIssueRequest {
            issue_type: Patch::Clear,
            ..Default::default()
        };
        let err = req.validate().unwrap_err();
        assert!(matches!(err, MutateError::Validation(s) if s.contains("type cannot be cleared")));
    }

    #[test]
    fn update_request_rejects_explicit_null_type() {
        // M5: JSON `"type": null` must deserialize to Patch::Clear and
        // be rejected by validate(). The CLI surface is independent —
        // this nails the API behaviour.
        let req: UpdateIssueRequest = serde_json::from_str(r#"{"type": null}"#).unwrap();
        assert!(matches!(req.issue_type, Patch::Clear));
        let err = req.validate().unwrap_err();
        assert!(matches!(err, MutateError::Validation(s) if s.contains("type cannot be cleared")));
    }

    #[test]
    fn update_request_accepts_type_set_via_json() {
        let req: UpdateIssueRequest = serde_json::from_str(r#"{"type": "feature"}"#).unwrap();
        assert!(matches!(req.issue_type, Patch::Set(ref t) if t == "feature"));
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
        let err = update_issue(
            tmp.path(),
            "custom-required-target",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap_err();
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
        let parsed = crate::parser::parse_item_md_with_warnings(&dir.join("item.md"), slug, "open");
        let mut issue = parsed.issue;
        let schema = crate::schema::default_schema();
        issue.folder = folder_for_status(&schema, &issue.status).to_string();
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
        let out = update_issue(tmp.path(), "dryrun-target-x", req, None, &UncachedConfig).unwrap();
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
        let out = toggle_checkbox(
            tmp.path(),
            "checkbox-target-y",
            "deploy",
            None,
            None,
            false,
            &UncachedConfig,
        )
        .unwrap();
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
        let zero = toggle_checkbox(
            tmp.path(),
            "checkbox-amb-z",
            "missing",
            None,
            None,
            false,
            &UncachedConfig,
        )
        .unwrap_err();
        assert!(matches!(zero, MutateError::Validation(s) if s.contains("no checkbox")));
        let many = toggle_checkbox(
            tmp.path(),
            "checkbox-amb-z",
            "task",
            None,
            None,
            false,
            &UncachedConfig,
        )
        .unwrap_err();
        assert!(matches!(many, MutateError::Validation(s) if s.contains("matched")));
    }

    #[test]
    fn toggle_checkbox_dry_run_does_not_write() {
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] only one\n";
        let _ = seed_with_body(tmp.path(), "checkbox-dry-q", body);
        let before = fs::read_to_string(tmp.path().join("issues/checkbox-dry-q/item.md")).unwrap();
        let out = toggle_checkbox(
            tmp.path(),
            "checkbox-dry-q",
            "only one",
            None,
            None,
            true,
            &UncachedConfig,
        )
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
            update_issue(tmp.path(), "label-idem-w", req, None, &UncachedConfig).unwrap();
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
        let err =
            update_issue(tmp.path(), "apply-rollback-q", req, None, &UncachedConfig).unwrap_err();
        assert!(matches!(err, MutateError::SchemaViolation(_)));
        let after = fs::read_to_string(tmp.path().join("issues/apply-rollback-q/item.md")).unwrap();
        assert_eq!(
            before, after,
            "schema violation must leave the file unchanged"
        );
    }

    #[test]
    fn body_ops_apply_atomically_with_frontmatter() {
        // Multi-op patch: status change + label add + checkbox toggle +
        // note append must produce a single canonical-hash bump for the
        // entire transaction (one write under one flock).
        let tmp = fresh_repo();
        let body = "# T\n\n## Tasks\n\n- [ ] tests passing\n\n## Description\n\nbody.\n";
        let v0 = seed_with_body(tmp.path(), "body-ops-mix", body);
        let req = UpdateIssueRequest {
            expected_version: Some(v0),
            status: Patch::Set("testing".into()),
            add_labels: vec!["agent-friendly".into()],
            body_ops: vec![
                BodyOp::SetCheckbox(SetCheckboxOp {
                    match_substring: "tests passing".into(),
                    checked: true,
                }),
                BodyOp::AppendNote(AppendNoteOp {
                    author: "ci-bot".into(),
                    message: "all checks green".into(),
                    section: NoteSection::AgentRuns,
                }),
            ],
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "body-ops-mix", req, None, &UncachedConfig).unwrap();
        assert!(out.version.starts_with("sha256:"));
        let after = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(after.contains("status: testing"));
        assert!(after.contains("agent-friendly"));
        assert!(after.contains("- [x] tests passing"));
        assert!(after.contains("## Agent Runs"));
        assert!(after.contains("@ci-bot"));
        assert!(after.contains("all checks green"));
    }

    #[test]
    fn body_ops_dry_run_emits_diff_without_writing() {
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] only one\n";
        let _ = seed_with_body(tmp.path(), "body-ops-dry", body);
        let before = fs::read_to_string(tmp.path().join("issues/body-ops-dry/item.md")).unwrap();
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "only one".into(),
                checked: true,
            })],
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "body-ops-dry", req, None, &UncachedConfig).unwrap();
        let pending = out.pending_serialized.expect("dry-run carries pending");
        assert!(pending.contains("- [x] only one"));
        assert!(out.before_serialized.is_some());
        let after = fs::read_to_string(tmp.path().join("issues/body-ops-dry/item.md")).unwrap();
        assert_eq!(before, after, "dry-run must not touch disk");
    }

    #[test]
    fn body_ops_rollback_on_failed_op() {
        // A failing checkbox match must surface as Validation and leave
        // disk untouched — even when the patch also changes the
        // frontmatter (status, labels). The whole transaction rolls
        // back; nothing partial leaks.
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] alpha\n- [ ] beta\n";
        let _ = seed_with_body(tmp.path(), "body-ops-rollback", body);
        let before =
            fs::read_to_string(tmp.path().join("issues/body-ops-rollback/item.md")).unwrap();
        let req = UpdateIssueRequest {
            status: Patch::Set("testing".into()),
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "nope".into(),
                checked: true,
            })],
            ..Default::default()
        };
        let err =
            update_issue(tmp.path(), "body-ops-rollback", req, None, &UncachedConfig).unwrap_err();
        assert!(
            matches!(&err, MutateError::Validation(s) if s.contains("body_ops[0]") && s.contains("no checkbox")),
            "got {err:?}"
        );
        let after =
            fs::read_to_string(tmp.path().join("issues/body-ops-rollback/item.md")).unwrap();
        assert_eq!(
            before, after,
            "failed body op must roll back frontmatter changes too"
        );
    }

    #[test]
    fn body_ops_deserialize_external_tag_yaml_shape() {
        // The patch.yaml shape is externally tagged: each list entry is
        // a single-key mapping. Pin the wire format so a future serde
        // refactor can't silently change the agent contract.
        let yaml = r#"
body_ops:
  - set_checkbox:
      match: "tests passing"
      checked: true
  - append_note:
      section: agent_runs
      author: ci-bot
      message: "all green"
"#;
        let req: UpdateIssueRequest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(req.body_ops.len(), 2);
        match &req.body_ops[0] {
            BodyOp::SetCheckbox(s) => {
                assert_eq!(s.match_substring, "tests passing");
                assert!(s.checked);
            }
            other => panic!("expected SetCheckbox, got {other:?}"),
        }
        match &req.body_ops[1] {
            BodyOp::AppendNote(n) => {
                assert_eq!(n.author, "ci-bot");
                assert_eq!(n.section, NoteSection::AgentRuns);
            }
            other => panic!("expected AppendNote, got {other:?}"),
        }
    }

    #[test]
    fn body_ops_deserialize_external_tag_json_shape() {
        // PATCH /api/issues/<slug> body must accept the same external-
        // tag shape over JSON. Pin both arms so the server contract
        // round-trips with the YAML one above.
        let json = r#"{
            "body_ops": [
                {"set_checkbox": {"match": "ship it", "checked": false}},
                {"append_note": {"author": "alice", "message": "done"}}
            ]
        }"#;
        let req: UpdateIssueRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.body_ops.len(), 2);
        match &req.body_ops[0] {
            BodyOp::SetCheckbox(s) => {
                assert_eq!(s.match_substring, "ship it");
                assert!(!s.checked);
            }
            other => panic!("expected SetCheckbox, got {other:?}"),
        }
        match &req.body_ops[1] {
            BodyOp::AppendNote(n) => {
                assert_eq!(n.author, "alice");
                assert_eq!(n.section, NoteSection::Comments);
            }
            other => panic!("expected AppendNote, got {other:?}"),
        }
    }

    #[test]
    fn body_ops_deserialize_rejects_unknown_top_key() {
        // Unknown variant key — visitor must reject with the canonical
        // unknown-field shape.
        let json = r#"{"body_ops": [{"toggl_checkbox": "x"}]}"#;
        let err = serde_json::from_str::<UpdateIssueRequest>(json).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("toggl_checkbox") || msg.contains("unknown"),
            "got {msg}"
        );
    }

    #[test]
    fn body_ops_deserialize_rejects_extra_sibling_key() {
        // Unknown sibling key beside a valid op.
        let json = r#"{"body_ops": [
            {"set_checkbox": {"match": "x", "checked": true}, "junk": 1}
        ]}"#;
        let err = serde_json::from_str::<UpdateIssueRequest>(json).unwrap_err();
        assert!(err.to_string().contains("single-key mapping"), "got {err}");
    }

    #[test]
    fn body_ops_deserialize_rejects_null_sibling_key_bypass() {
        // Previous Option<T> helper struct accepted this — the null
        // collapsed to None and "exactly one variant" passed.
        let json = r#"{"body_ops": [
            {"set_checkbox": {"match": "x", "checked": true}, "append_note": null}
        ]}"#;
        let err = serde_json::from_str::<UpdateIssueRequest>(json).unwrap_err();
        assert!(err.to_string().contains("single-key mapping"), "got {err}");
    }

    #[test]
    fn append_note_op_rejects_unknown_field() {
        // Pin the pre-existing `deny_unknown_fields` on AppendNoteOp so a
        // future refactor doesn't silently drop the directive.
        let json = r#"{"body_ops": [
            {"append_note": {"author": "a", "message": "m", "junk": 1}}
        ]}"#;
        let err = serde_json::from_str::<UpdateIssueRequest>(json).unwrap_err();
        assert!(
            err.to_string().contains("junk") || err.to_string().contains("unknown"),
            "got {err}"
        );
    }

    #[test]
    fn body_ops_length_cap_rejected_by_validate() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "body-ops-too-many", "open");
        let huge: Vec<BodyOp> = (0..(MAX_BODY_OPS + 1))
            .map(|_| {
                BodyOp::SetCheckbox(SetCheckboxOp {
                    match_substring: "x".into(),
                    checked: true,
                })
            })
            .collect();
        let req = UpdateIssueRequest {
            body_ops: huge,
            ..Default::default()
        };
        let err =
            update_issue(tmp.path(), "body-ops-too-many", req, None, &UncachedConfig).unwrap_err();
        assert!(
            matches!(&err, MutateError::Validation(s) if s.contains("body_ops length") && s.contains("exceeds")),
            "got {err:?}"
        );
    }

    #[test]
    fn failed_body_op_does_not_create_default_schema() {
        // Regression: until the locate-then-validate-then-side-effects
        // refactor, `ensure_default_written` ran before body ops, so a
        // failing op left `.schema.yaml` newly created on a fresh repo.
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] alpha\n";
        let _ = seed_with_body(tmp.path(), "body-ops-no-schema-bootstrap", body);
        let schema_path = tmp.path().join("issues/.schema.yaml");
        assert!(!schema_path.exists(), "precondition: no schema yet");
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "no-such-needle".into(),
                checked: true,
            })],
            ..Default::default()
        };
        let err = update_issue(
            tmp.path(),
            "body-ops-no-schema-bootstrap",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)));
        assert!(
            !schema_path.exists(),
            "failed body op must not bootstrap .schema.yaml"
        );
    }

    #[test]
    fn failed_body_op_does_not_migrate_legacy_layout() {
        // Same regression class for the legacy → flat directory move.
        let tmp = fresh_repo();
        let legacy_dir = tmp.path().join("issues/open/body-ops-no-migrate");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(
            legacy_dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n# T\n\n- [ ] only\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "no-such-needle".into(),
                checked: true,
            })],
            ..Default::default()
        };
        let _ = update_issue(
            tmp.path(),
            "body-ops-no-migrate",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap_err();
        assert!(
            legacy_dir.join("item.md").exists(),
            "failed body op must leave legacy directory in place"
        );
        assert!(
            !tmp.path()
                .join("issues/body-ops-no-migrate/item.md")
                .exists(),
            "failed body op must NOT migrate to flat layout"
        );
    }

    #[test]
    fn standalone_note_does_not_create_default_schema_on_validation_failure() {
        // Side-effects deferral on body-only verbs: a `note_issue`
        // call that fails validation must not bootstrap `.schema.yaml`
        // (parity with the `update_issue`/`apply` path).
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "note-defer", "open");
        // Tighten the schema so the post-mutation frontmatter is
        // rejected (required field that doesn't exist on the issue).
        // The schema file *exists* before the call, so the failure
        // path we exercise is "schema validation rejects the write,"
        // not "schema file missing."
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        // Now stage a legacy directory so the migration side-effect
        // would also leak if not deferred.
        let legacy = tmp.path().join("issues/open/note-legacy-defer");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let err = note_issue(
            tmp.path(),
            "note-legacy-defer",
            "alice",
            "hi",
            crate::body_sections::COMMENTS,
            None,
            None,
            false,
            &UncachedConfig,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::SchemaViolation(_)));
        assert!(
            legacy.join("item.md").exists(),
            "legacy dir must remain on schema-violation rollback"
        );
        assert!(
            !tmp.path().join("issues/note-legacy-defer/item.md").exists(),
            "no migration must have happened"
        );
    }

    #[test]
    fn standalone_toggle_checkbox_does_not_migrate_legacy_on_match_failure() {
        let tmp = fresh_repo();
        let legacy = tmp.path().join("issues/open/cbx-legacy-defer");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n# T\n\n- [ ] only\n",
        )
        .unwrap();
        let err = toggle_checkbox(
            tmp.path(),
            "cbx-legacy-defer",
            "no-such-substring",
            None,
            None,
            false,
            &UncachedConfig,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)));
        assert!(
            legacy.join("item.md").exists(),
            "legacy dir must remain on no-match rollback"
        );
        assert!(
            !tmp.path().join("issues/cbx-legacy-defer/item.md").exists(),
            "no migration must have happened"
        );
    }

    #[test]
    fn idempotent_set_checkbox_keeps_canonical_version_stable() {
        // Pin the central retry-safety contract: replaying an
        // already-target set_checkbox produces the same canonical
        // version (false-409s would defeat optimistic concurrency).
        // `updated:` IS bumped on disk and an SSE event fires, but
        // both are excluded from / orthogonal to the canonical hash.
        let tmp = fresh_repo();
        let body = "# T\n\n- [x] already on\n";
        let _ = seed_with_body(tmp.path(), "set-cbx-version-stable", body);
        let v0 = {
            let req = UpdateIssueRequest::default();
            update_issue(
                tmp.path(),
                "set-cbx-version-stable",
                req,
                None,
                &UncachedConfig,
            )
            .unwrap()
            .version
        };
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "already on".into(),
                checked: true,
            })],
            ..Default::default()
        };
        let v1 = update_issue(
            tmp.path(),
            "set-cbx-version-stable",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap()
        .version;
        assert_eq!(
            v0, v1,
            "no-op set_checkbox must not bump the canonical version (retry-safety contract)"
        );
    }

    #[test]
    fn idempotent_set_checkbox_uncheck_already_unchecked() {
        // Mirror of `set_checkbox_is_idempotent_on_target_state` for
        // the `checked: false` arm — pin both directions.
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] already off\n";
        let _ = seed_with_body(tmp.path(), "set-cbx-uncheck-idem", body);
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "already off".into(),
                checked: false,
            })],
            ..Default::default()
        };
        update_issue(
            tmp.path(),
            "set-cbx-uncheck-idem",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap();
        let after =
            fs::read_to_string(tmp.path().join("issues/set-cbx-uncheck-idem/item.md")).unwrap();
        assert!(after.contains("- [ ] already off"));
    }

    #[test]
    fn body_ops_visitor_rejects_empty_map() {
        // `{}` body-op entry must error rather than be accepted as a
        // mystery default — pin the visitor branch.
        let json = r#"{"body_ops": [{}]}"#;
        let err = serde_json::from_str::<UpdateIssueRequest>(json).unwrap_err();
        assert!(
            err.to_string().contains("declare exactly one operation"),
            "got {err}"
        );
    }

    #[test]
    fn transition_warnings_surface_malformed_config_instead_of_silence() {
        // Regression: previously `transition_warnings` swallowed
        // `transitions.yaml` load failures, so a body verb on a repo
        // with a broken rules engine got NO warning while the unified
        // PATCH path 5xx'd. Now the body verbs surface the load
        // failure as a warning string so agents and operators see the
        // outage either way.
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "broken-rules-target", "open");
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(
            tmp.path().join(".issuectl/transitions.yaml"),
            "this is not: [valid yaml: at all\n",
        )
        .unwrap();
        let out = note_issue(
            tmp.path(),
            "broken-rules-target",
            "alice",
            "hi",
            crate::body_sections::COMMENTS,
            None,
            None,
            false,
            &UncachedConfig,
        )
        .unwrap();
        assert!(
            out.warnings.iter().any(|w| w.contains("rules engine")),
            "expected a 'rules engine: ...' warning, got {:?}",
            out.warnings
        );
    }

    #[test]
    fn standalone_toggle_checkbox_surfaces_transition_warning_without_failing() {
        // #11: standalone body verbs (`toggle_checkbox`, `note_issue`)
        // detect transition-rule violations but emit them as warnings
        // rather than refusing the write — the user wanted the change
        // through; the rule mismatch goes to the caller for them to
        // resolve. The unified `body_ops` PATCH path keeps the strict
        // rejection.
        let tmp = fresh_repo();
        // Set up a rule: `done` requires assignee. Seed a `done` issue
        // without an assignee so the rule is already violated; the
        // checkbox toggle won't change frontmatter so it just inherits.
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(
            tmp.path().join(".issuectl/transitions.yaml"),
            "version: 1\nstatus_rules:\n  done:\n    requires_assignee: true\n",
        )
        .unwrap();
        let dir = tmp.path().join("issues/warn-cbx");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: task\ncreated: 2026-05-06\nstatus: done\npriority: normal\n---\n\n# T\n\n- [ ] flip me\n",
        )
        .unwrap();
        let out = toggle_checkbox(
            tmp.path(),
            "warn-cbx",
            "flip me",
            None,
            None,
            false,
            &UncachedConfig,
        )
        .unwrap();
        assert!(
            !out.warnings.is_empty(),
            "expected at least one warning for the rule violation"
        );
        assert!(
            out.warnings.iter().any(|w| w.contains("assignee")),
            "warnings should mention the missing assignee, got {:?}",
            out.warnings
        );
        // Write went through despite the violation.
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after.contains("- [x] flip me"));
    }

    #[test]
    fn set_checkbox_is_idempotent_on_target_state() {
        // A retry of the same set_checkbox op (already at target state)
        // must NOT toggle the box back. This is the central reason
        // body_ops uses set_checkbox rather than the toggle primitive.
        let tmp = fresh_repo();
        let body = "# T\n\n- [x] already checked\n";
        let _ = seed_with_body(tmp.path(), "set-cbx-idem", body);
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "already checked".into(),
                checked: true,
            })],
            ..Default::default()
        };
        update_issue(tmp.path(), "set-cbx-idem", req, None, &UncachedConfig).unwrap();
        let after = fs::read_to_string(tmp.path().join("issues/set-cbx-idem/item.md")).unwrap();
        assert!(
            after.contains("- [x] already checked"),
            "idempotent set must leave box checked, got:\n{after}"
        );
    }

    #[test]
    fn body_ops_validate_rejects_bad_author() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "body-ops-bad-author", "open");
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::AppendNote(AppendNoteOp {
                author: "alice\n## Pwned".into(),
                message: "hi".into(),
                section: NoteSection::Comments,
            })],
            ..Default::default()
        };
        let err = update_issue(
            tmp.path(),
            "body-ops-bad-author",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::Validation(s) if s.contains("body_ops[0].author")));
    }

    #[test]
    fn dry_run_does_not_create_default_schema() {
        // Regression for review finding #1: `--dry-run` must not
        // bootstrap `.schema.yaml` (the previous version called
        // `ensure_default_written` before the dry-run branch).
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "dryrun-no-schema-x", "open");
        let schema_path = tmp.path().join("issues/.schema.yaml");
        assert!(!schema_path.exists(), "precondition: no schema yet");
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "dryrun-no-schema-x", req, None, &UncachedConfig).unwrap();
        assert!(
            !schema_path.exists(),
            "dry-run must not create issues/.schema.yaml"
        );
    }

    #[test]
    fn dry_run_does_not_migrate_legacy_layout() {
        // Regression for review finding #1: `--dry-run` must not
        // perform the legacy → flat directory rename that
        // `locate_and_migrate` does on real writes.
        let tmp = fresh_repo();
        let legacy_dir = tmp.path().join("issues/open/dryrun-no-migrate-y");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(
            legacy_dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let flat_dir = tmp.path().join("issues/dryrun-no-migrate-y");
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let _ = update_issue(
            tmp.path(),
            "dryrun-no-migrate-y",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap();
        assert!(legacy_dir.exists(), "dry-run must not move the legacy dir");
        assert!(
            !flat_dir.exists(),
            "dry-run must not create the flat-layout dir"
        );
    }

    #[test]
    fn dry_run_returns_before_serialized_for_diff() {
        // Regression for review finding #5: `before_serialized` must
        // be filled under the flock so the CLI epilogue can render
        // a diff without re-reading disk outside the lock.
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "dryrun-before-w", "open");
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "dryrun-before-w", req, None, &UncachedConfig).unwrap();
        let before = out.before_serialized.expect("dry-run must capture before");
        let after = out.pending_serialized.expect("dry-run must capture after");
        assert!(before.contains("priority: normal"));
        assert!(after.contains("priority: high"));
        assert_ne!(before, after);
    }

    #[test]
    fn checkbox_state_does_not_panic_on_unicode_marker() {
        // Regression for review finding #4: `checkbox_state` used to
        // panic on `&rest[2..3]` when the bracket content was
        // multibyte (e.g. `[✓]`).
        assert_eq!(checkbox_state("- [✓] task"), None);
        assert_eq!(checkbox_state("- [é] task"), None);
        assert_eq!(checkbox_state("- [ ] task"), Some(false));
        assert_eq!(checkbox_state("- [x] task"), Some(true));
        assert_eq!(checkbox_state("- [X] task"), Some(true));
        // Don't panic with non-ASCII content after the box either.
        assert_eq!(checkbox_state("- [ ] café"), Some(false));
    }

    #[test]
    fn toggle_checkbox_ignores_lines_inside_fenced_code() {
        // Regression for review finding #3: fenced code blocks must
        // NOT be considered when matching checkbox lines.
        let tmp = fresh_repo();
        let body = "# T\n\nIn docs:\n\n```markdown\n- [ ] only example here\n```\n\n\
                    Real:\n\n- [ ] real task\n";
        let _ = seed_with_body(tmp.path(), "fence-target-z", body);
        // The "example" substring is only inside the code fence —
        // the fence-aware scanner should report no match.
        let err = toggle_checkbox(
            tmp.path(),
            "fence-target-z",
            "example",
            None,
            None,
            false,
            &UncachedConfig,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::Validation(s) if s.contains("no checkbox")));
        // The real task should still toggle cleanly.
        toggle_checkbox(
            tmp.path(),
            "fence-target-z",
            "real",
            None,
            None,
            false,
            &UncachedConfig,
        )
        .unwrap();
        let after = fs::read_to_string(tmp.path().join("issues/fence-target-z/item.md")).unwrap();
        assert!(after.contains("- [x] real task"));
        assert!(after.contains("- [ ] only example here"));
    }

    #[test]
    fn note_validates_against_schema() {
        // Regression for review finding #6: `note_issue` previously
        // skipped schema validation, which `update_body` enforced —
        // making `body set` reject and `note` write through.
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "note-schema-target-q", "open");
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let err = note_issue(
            tmp.path(),
            "note-schema-target-q",
            "alice",
            "hello",
            crate::body_sections::COMMENTS,
            None,
            None,
            false,
            &UncachedConfig,
        )
        .unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => assert!(msg.contains("team"), "got {msg:?}"),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn check_validates_against_schema() {
        // Regression for review finding #6.
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] only one\n";
        let _ = seed_with_body(tmp.path(), "check-schema-target-r", body);
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let err = toggle_checkbox(
            tmp.path(),
            "check-schema-target-r",
            "only one",
            None,
            None,
            false,
            &UncachedConfig,
        )
        .unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => assert!(msg.contains("team"), "got {msg:?}"),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn epic_slug_shape_validated_in_request() {
        // Regression for review finding #7: `Patch::Set` for epic
        // must reject non-slug-shaped values in `validate()` so the
        // YAML / `set` paths can't bypass the CLI flag's slug check.
        let req = UpdateIssueRequest {
            epic: Patch::Set("Not a slug".into()),
            ..Default::default()
        };
        let err = req.validate().unwrap_err();
        assert!(matches!(err, MutateError::Validation(s) if s.contains("epic")));
    }

    #[test]
    fn apply_yaml_rejects_duplicate_keys() {
        // Regression for review finding #12: `serde_yaml 0.9` rejects
        // duplicate map keys at every depth. The reviewers feared a
        // last-wins silent collapse for `priority: high\npriority: low`;
        // verify the parser rejects it instead.
        let yaml = "slug: a-b\npriority: high\npriority: low\n";
        let err = serde_yaml::from_str::<serde_yaml::Value>(yaml).unwrap_err();
        assert!(
            err.to_string().contains("duplicate"),
            "expected duplicate-key error, got {err}"
        );
        let nested = "slug: a-b\ncustom_fields:\n  k: v\n  k: v2\n";
        let err = serde_yaml::from_str::<serde_yaml::Value>(nested).unwrap_err();
        assert!(
            err.to_string().contains("duplicate"),
            "expected nested duplicate-key error, got {err}"
        );
    }

    #[test]
    fn apply_multi_field_patch_lands_atomically() {
        // Positive test for the apply transaction: priority + add_label
        // + custom_field all advance the canonical hash exactly once.
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "apply-happy-path", "open");
        let req = UpdateIssueRequest {
            expected_version: Some(v0.clone()),
            priority: Patch::Set("high".into()),
            add_labels: vec!["backend".into()],
            custom_fields: {
                let mut m = std::collections::BTreeMap::new();
                m.insert("triage".into(), Patch::Set("P1".into()));
                m
            },
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "apply-happy-path", req, None, &UncachedConfig).unwrap();
        assert_ne!(out.version, v0);
        let after = fs::read_to_string(tmp.path().join("issues/apply-happy-path/item.md")).unwrap();
        assert!(after.contains("priority: high"));
        assert!(after.contains("backend"));
        assert!(after.contains("triage: P1"));
    }

    // ── Round-2 review regressions ───────────────────────────────

    #[test]
    fn status_clear_validation_rejects_before_any_disk_writes() {
        // Round-2 #1: dropping the CLI-side `status --clear` check
        // exposed a hole — `Patch::Clear` for status passed
        // `validate()` and only got rejected deeper inside
        // `update_issue_under_lock`, *after* `ensure_default_written`
        // and `locate_and_migrate` had already written `.schema.yaml`
        // and migrated legacy directories.
        let tmp = fresh_repo();
        let legacy = tmp.path().join("issues/open/status-clear-legacy");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\n---\n\n# T\n",
        )
        .unwrap();

        let req = UpdateIssueRequest {
            status: Patch::Clear,
            ..Default::default()
        };
        let err = update_issue(
            tmp.path(),
            "status-clear-legacy",
            req,
            None,
            &UncachedConfig,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)));

        assert!(
            legacy.exists(),
            "validation failure must not migrate the legacy directory"
        );
        assert!(
            !tmp.path().join("issues/status-clear-legacy").exists(),
            "validation failure must not create the flat-layout directory"
        );
        assert!(
            !tmp.path().join("issues/.schema.yaml").exists(),
            "validation failure must not bootstrap the default schema"
        );
    }

    #[test]
    fn dry_run_before_serialized_captures_raw_disk_bytes() {
        // Round-2 #2: `before_serialized` used to be the canonicalised
        // re-serialization of the parsed item, which silently hid
        // formatting changes that the real write would also apply
        // (dropped YAML comments, scalar-style shifts, etc.). Pin
        // that the field now contains the raw on-disk bytes so the
        // dry-run diff is a faithful preview.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/raw-bytes-target");
        fs::create_dir_all(&dir).unwrap();
        let raw = "---\ntype: bug\n# survives only on disk; serde_yaml drops it on round-trip\n\
                   created: 2026-05-06\nstatus: open\npriority: normal\n---\n\n# T\n";
        fs::write(dir.join("item.md"), raw).unwrap();

        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "raw-bytes-target", req, None, &UncachedConfig).unwrap();
        let before = out.before_serialized.expect("dry-run captures before");
        assert_eq!(
            before, raw,
            "before_serialized must be the raw on-disk bytes, not a canonicalised re-serialization"
        );
        // And the after must NOT contain the comment, demonstrating
        // that a real write would drop it — the dry-run diff visibly
        // reflects that loss because we don't pre-canonicalise the
        // before half.
        let after = out.pending_serialized.expect("dry-run captures after");
        assert!(!after.contains("survives only on disk"));
    }

    #[test]
    fn dry_run_final_dir_predicts_flat_path_for_legacy_issue() {
        // Round-2 #3: dry-run on a legacy-layout issue used to return
        // `issue_dir = issues/open/<slug>` (where the file currently
        // lives) but a real write would migrate to `issues/<slug>`.
        // The JSON envelope's `final_dir` must agree with the real
        // write's destination.
        let tmp = fresh_repo();
        let legacy = tmp.path().join("issues/open/legacy-finaldir");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\n---\n\n# T\n",
        )
        .unwrap();

        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "legacy-finaldir", req, None, &UncachedConfig).unwrap();
        assert_eq!(
            out.issue_dir,
            tmp.path().join("issues/legacy-finaldir"),
            "dry-run must report the flat-layout path even when the file currently lives at a legacy path"
        );
        // And legacy must remain untouched — no migration.
        assert!(legacy.exists(), "dry-run must not migrate legacy layout");
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
            &UncachedConfig,
        )
        .unwrap();
        let after = fs::read_to_string(tmp.path().join("issues/note-decision-p/item.md")).unwrap();
        assert!(after.contains("## Decisions"));
        assert!(after.contains("go with option B"));
        assert!(!after.contains("## Comments"));
    }
}
