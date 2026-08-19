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
//! The CLI entry points (`do_update`, `do_close`, `do_new`) all call
//! into this module so a) every writer obtains the same `flock` and
//! b) every writer emits the same canonical version token.

pub mod archive;
pub mod attach;
pub mod intake;
pub mod intake_migrate;
pub mod new_issue;
pub mod triage;

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Deserializer};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::canonical::canonical_hash;
use crate::clock::{Clock, SystemClock};
use crate::models::Issue;
use crate::repo::{self, folder_for_status};
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
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateIssueRequest {
    #[serde(default)]
    pub expected_version: Option<String>,
    #[serde(default)]
    pub status: Patch<String>,
    /// Issue title stored in the markdown body's first `# ` heading.
    /// `Set` rewrites that heading, or prepends one when repairing a
    /// headingless body. Clearing is rejected because every issue needs a
    /// title. The title remains body-backed and therefore participates in
    /// the canonical hash without a schema field.
    #[serde(default)]
    pub title: Patch<String>,
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
    pub reporter: Patch<String>,
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
    /// Scheduling lane (`crate::dag`). Scalar `Patch` like `epic`: `Set`
    /// assigns the lane, `Clear` removes it (`update --no-lane`). Reserved
    /// from `custom_fields` so this validated slot is the only writer.
    #[serde(default)]
    pub lane: Patch<String>,
    /// Coarse intra-lane precedence key (`crate::dag`). Integer `Patch`
    /// like `lane` but numeric: `Set` writes `lane_seq: <int>`, `Clear`
    /// removes it (`update --no-lane-seq`). Reserved from `custom_fields`
    /// so this validated slot is the only writer.
    #[serde(default)]
    pub lane_seq: Patch<i64>,
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
    /// `collision:` list operations, same shape as `add_labels` /
    /// `remove_labels`: free-form hot-file tokens with no ref
    /// normalization (they are file/family identifiers, not slugs).
    #[serde(default)]
    pub add_collision: Vec<String>,
    #[serde(default)]
    pub remove_collision: Vec<String>,
    #[serde(default)]
    pub add_commits: Vec<CommitSpec>,
    /// Per-key custom-frontmatter PATCH. Mirrors the top-level `Patch`
    /// ternary: omitted (no entry) leaves the key alone; `null` removes
    /// the key; a string sets it. Built-in keys (`status`, `priority`,
    /// dates, etc.) are reserved here — use the dedicated request slots.
    ///
    /// Duplicate keys in the deserialized payload are rejected during
    /// deserialization, mirroring `NewIssueRequest::custom_fields` so the
    /// JSON/YAML patch input (`issuectl apply`) enforces the same invariant
    /// the CLI `--field foo=a --field foo=b` rejection enforces — without
    /// this gate `serde_json` silently keeps whichever value the parser
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
    /// Replace the entire markdown body under the same flock as the
    /// frontmatter PATCHes above. `None` leaves the body untouched;
    /// `Some(body)` swaps it wholesale (the plain markdown a client
    /// sends, sans the leading `---`/frontmatter). The replacement
    /// applies *before* `body_ops` and the reopen-notes append, so those
    /// layer on top of the new body, and before the type-change
    /// required-section check, so that check validates the final body.
    /// A reserved-legacy section heading (`## Notes`) surfaces the same
    /// non-fatal warning `update_body` / `body set` raise. Drives
    /// `issuectl update --description`/`--body`/`--body-file`.
    #[serde(default)]
    pub set_body: Option<String>,
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
    ("lane", "use `update --lane <name>` / `--no-lane`"),
    (
        "lane_seq",
        "use `update --lane-seq <int>` / `--no-lane-seq`",
    ),
    (
        "collision",
        "list-typed: use `update --add-collision` / `--remove-collision`",
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
    /// Closing rationale recorded by `close --comment/--note`.
    Resolution,
}

impl NoteSection {
    fn as_str(self) -> &'static str {
        match self {
            NoteSection::Comments => crate::body_sections::COMMENTS,
            NoteSection::Decisions => crate::body_sections::DECISIONS,
            NoteSection::AgentRuns => crate::body_sections::AGENT_RUNS,
            NoteSection::Resolution => crate::body_sections::RESOLUTION,
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
            && matches!(self.title, Patch::Unspecified)
            && matches!(self.issue_type, Patch::Unspecified)
            && matches!(self.priority, Patch::Unspecified)
            && matches!(self.reporter, Patch::Unspecified)
            && matches!(self.assignee, Patch::Unspecified)
            && matches!(self.owner, Patch::Unspecified)
            && matches!(self.epic, Patch::Unspecified)
            && matches!(self.closed_by, Patch::Unspecified)
            && matches!(self.lane, Patch::Unspecified)
            && matches!(self.lane_seq, Patch::Unspecified)
            && self.add_labels.is_empty()
            && self.remove_labels.is_empty()
            && self.add_related.is_empty()
            && self.remove_related.is_empty()
            && self.add_blocked_by.is_empty()
            && self.remove_blocked_by.is_empty()
            && self.add_collision.is_empty()
            && self.remove_collision.is_empty()
            && self.add_commits.is_empty()
            && self.custom_fields.is_empty()
            && self.body_ops.is_empty()
            && self.set_body.is_none()
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
        if matches!(self.title, Patch::Clear) {
            return Err(MutateError::Validation(
                "title cannot be cleared (issues always have a title)".into(),
            ));
        }
        if matches!(self.issue_type, Patch::Clear) {
            return Err(MutateError::Validation(
                "type cannot be cleared (issues always have a type)".into(),
            ));
        }
        check_set_nonempty("status", &self.status)?;
        check_set_nonempty("title", &self.title)?;
        if let Patch::Set(title) = &self.title {
            if title.trim() != title || title.contains(['\r', '\n']) {
                return Err(MutateError::Validation(
                    "title must be a single line without leading or trailing whitespace".into(),
                ));
            }
        }
        check_set_nonempty("type", &self.issue_type)?;
        // No `crate::issue_fields::ISSUE_TYPES` membership check here: the schema
        // (`fields.type.enum`) is the source of truth for allowed
        // values, and a custom schema may declare additional types
        // (e.g. `spike`). Validation runs in step 4b under lock against
        // the post-mutation frontmatter; that's the right layer.
        check_set_nonempty("priority", &self.priority)?;
        check_set_nonempty("reporter", &self.reporter)?;
        check_set_nonempty("assignee", &self.assignee)?;
        check_set_nonempty("owner", &self.owner)?;
        check_set_nonempty("epic", &self.epic)?;
        check_set_nonempty("closed_by", &self.closed_by)?;
        check_set_nonempty("lane", &self.lane)?;
        // Closer attribution follows the same author grammar as
        // `note --as`, so the recorded value is a well-formed,
        // hash-stable token regardless of entry point (CLI `close --as`
        // or a raw PATCH populating the slot).
        if let Patch::Set(author) = &self.closed_by {
            crate::body_sections::validate_author(author)
                .map_err(|e| MutateError::Validation(format!("closed_by: {e}")))?;
        }

        // A body replacement must carry content: an empty (or whitespace-
        // only) `set_body` would blank the document. The CLI already
        // guards this (`parse_non_empty` / `read_body_file_arg`), so this
        // catches a raw JSON/YAML PATCH — parity with the non-empty
        // contract of `new --description`/`--body-file`.
        if let Some(body) = &self.set_body {
            if body.trim().is_empty() {
                return Err(MutateError::Validation(
                    "set_body cannot be empty (a body replacement must carry content)".into(),
                ));
            }
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
            ("add_collision", &self.add_collision),
            ("remove_collision", &self.remove_collision),
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
        if let Some(overlap) = first_overlap(&self.add_collision, &self.remove_collision) {
            return Err(MutateError::ConflictingIntent(format!(
                "collision token {overlap:?} appears in both add_collision and remove_collision"
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
/// caller (CLI arg); the request body carries the rest.
mod body;
mod close;
#[cfg(test)]
mod mutate_tests;
mod new_api;
mod shared;
mod update;

use body::*;
pub use body::{
    note_issue, note_issue_via, toggle_checkbox, toggle_checkbox_via, update_body, update_body_via,
};
pub use close::{bulk_update, close_issue, close_issue_via};
use new_api::*;
pub use new_api::{new_issue, NewIssueRequest, NewOutcome};
pub use shared::write_item_atomic;
use shared::*;
use update::*;
pub use update::{update_issue, update_issue_via};
