//! First-class **intake** domain operations — the reception → triage →
//! disposition lifecycle from `docs/design/intake-flow.md`.
//!
//! Each verb is a single domain mutation, not a script chaining `set` +
//! `note`: it takes the repo `flock` once, validates the source state,
//! then applies the status change, the structured field(s), and the
//! `## Comments` audit note in **one** atomic write via
//! `super::update_issue_under_lock` — so a crash can never leave the
//! status changed but the note lost.
//!
//! Two enforcement tiers, per OD-9:
//!
//! - **Intrinsic invariants** (`intrinsic_transition_violations`) hold
//!   with or without `.issuectl/transitions.yaml`. They live in
//!   `super::update_issue_under_lock`, so the generic `set status`
//!   path routes through the exact same validators — the intake verbs
//!   are not a privileged bypass, and `set` is not an unprivileged one.
//! - **Per-verb source-state preconditions** (e.g. `accept` refuses a
//!   closed item) are checked here, before the request is built, so the
//!   refusal is precise and side-effect-free.
//!
//! Concurrency is deliberately **out of scope** (design's approved
//! decision): no verb here takes `expected_version` / `--if-version`.
//! The repo `flock` is the only write-safety mechanism.

use std::path::{Path, PathBuf};

use crate::canonical::canonical_hash;
use crate::repo::folder_for_status;
use crate::write;

use super::new_issue::{do_new_locked, NewArgs};
use super::{
    AppendNoteOp, BodyOp, MutateError, NoteSection, Patch, UpdateIssueRequest, UpdateOutcome,
    WriteLock,
};

// ── Field vocabulary (shared with the schema; no synonyms) ───────────────
const F_PROVENANCE: &str = "provenance";
const F_PROVENANCE_DETAIL: &str = "provenance_detail";
const F_SOURCE_REF: &str = "source_ref";
const F_DISPOSITION_REASON: &str = "disposition_reason";
const F_DISPOSITION_NOTE: &str = "disposition_note";
const F_DUPLICATE_OF: &str = "duplicate_of";
const F_DEFERRED_UNTIL: &str = "deferred_until";
const F_SUPERSEDED_BY: &str = "superseded_by";

/// Author stamped on every intake audit note.
const NOTE_AUTHOR: &str = "intake";

/// Keys a reporter may never inject through `intake file --field`
/// (OD-6). Lifecycle-bearing keys have dedicated flags or are managed by
/// the tool; letting `--field status=fixed` through would hollow out the
/// "always `untriaged`, reporter can't spoof" guarantee.
const PROTECTED_FILE_FIELDS: &[&str] = &[
    "status",
    "type",
    "closed",
    "created",
    "updated",
    "version",
    "reporter",
    // `provenance` / `source_ref` have dedicated flags and drive dedup;
    // letting them through `--field` (e.g. `--source-ref A --field
    // source_ref=B`) would corrupt the idempotency key. The disposition /
    // lifecycle fields are tool-managed by the triage verbs, never set at
    // filing time.
    "provenance",
    "provenance_detail",
    "source_ref",
    "disposition_reason",
    "disposition_note",
    "duplicate_of",
    "deferred_until",
    "superseded_by",
];

/// Active intake/holding states an item can be dispositioned *from*.
const INTAKE_STATES: &[&str] = &["untriaged", "deferred", "needs-info"];

// ── Errors ───────────────────────────────────────────────────────────────

/// Intake-specific error surface. Wraps [`MutateError`] for anything the
/// shared mutation layer raises, and adds the intake-only refusals the
/// CLI maps to the documented `--json` codes.
#[derive(Debug)]
pub enum IntakeError {
    /// Anything the shared mutation layer raised. Boxed so this enum
    /// stays small (`MutateError` carries a full `Issue` in one variant).
    Mutate(Box<MutateError>),
    /// A protected key was passed to `intake file --field`.
    ProtectedField(String),
    /// One or more existing issues already carry this
    /// `(provenance, source_ref)` and it is ambiguous which is canonical.
    DuplicateSourceRef {
        provenance: String,
        source_ref: String,
        existing: Vec<String>,
    },
    /// `--provenance` is not among the repo-configured accepted values.
    UnknownProvenance {
        value: String,
        accepted: Vec<String>,
    },
    /// `intake duplicate --of` pointed the item at itself.
    DuplicateSelf(String),
    /// `--of` target does not exist.
    DuplicateTargetMissing(String),
    /// Following `--of`'s `duplicate_of` chain loops back to the item.
    DuplicateCycle { of: String, chain: Vec<String> },
}

impl From<MutateError> for IntakeError {
    fn from(e: MutateError) -> Self {
        IntakeError::Mutate(Box::new(e))
    }
}

impl std::fmt::Display for IntakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntakeError::Mutate(e) => write!(f, "{e}"),
            IntakeError::ProtectedField(k) => write!(
                f,
                "field {k:?} is protected on `intake file` and cannot be set (managed by the tool)"
            ),
            IntakeError::DuplicateSourceRef {
                provenance,
                source_ref,
                existing,
            } => write!(
                f,
                "more than one issue already carries provenance={provenance:?} source_ref={source_ref:?} ({}); resolve the duplicates before filing",
                existing.join(", ")
            ),
            IntakeError::UnknownProvenance { value, accepted } => write!(
                f,
                "unknown provenance {value:?}; this repo accepts [{}]",
                accepted.join(", ")
            ),
            IntakeError::DuplicateSelf(slug) => {
                write!(f, "issue {slug:?} cannot be a duplicate of itself")
            }
            IntakeError::DuplicateTargetMissing(of) => {
                write!(f, "duplicate target {of:?} does not exist")
            }
            IntakeError::DuplicateCycle { of, chain } => write!(
                f,
                "marking this a duplicate of {of:?} would form a cycle ({})",
                chain.join(" → ")
            ),
        }
    }
}

impl std::error::Error for IntakeError {}

impl IntakeError {
    /// Stable `--json` error code (house contract).
    pub fn code(&self) -> &'static str {
        match self {
            IntakeError::ProtectedField(_) => "protected-field",
            IntakeError::DuplicateSourceRef { .. } => "duplicate-source-ref",
            IntakeError::UnknownProvenance { .. }
            | IntakeError::DuplicateSelf(_)
            | IntakeError::DuplicateTargetMissing(_)
            | IntakeError::DuplicateCycle { .. } => "validation",
            IntakeError::Mutate(m) => match m.as_ref() {
                MutateError::NotFound => "not-found",
                MutateError::TransitionViolation(_) => "transition-illegal",
                MutateError::SchemaViolation(_) => "schema-violation",
                MutateError::VersionMismatch { .. } => "version-conflict",
                MutateError::Validation(_) | MutateError::ConflictingIntent(_) => "validation",
                _ => "command-failed",
            },
        }
    }

    /// Process exit code: `2` for refused-but-actionable (a legal item in
    /// the wrong state — retry a different verb/target), `1` otherwise
    /// (validation / not-found / usage).
    pub fn exit_code(&self) -> i32 {
        match self {
            IntakeError::Mutate(m) if matches!(m.as_ref(), MutateError::TransitionViolation(_)) => {
                2
            }
            _ => 1,
        }
    }
}

// ── file ─────────────────────────────────────────────────────────────────

/// Arguments for [`file`]. Mirrors the guarded `intake file` CLI surface.
pub struct FileRequest {
    pub issue_type: String,
    pub title: String,
    pub body: Option<String>,
    pub reporter: Option<String>,
    pub provenance: String,
    pub provenance_detail: Option<String>,
    pub source_ref: Option<String>,
    pub priority: Option<String>,
    pub slug: Option<String>,
    pub labels: Vec<String>,
    /// Non-protected custom metadata (`--field key=value`).
    pub fields: Vec<(String, String)>,
}

/// Result of [`file`].
#[derive(Debug)]
pub struct FileOutcome {
    pub slug: String,
    pub version: String,
    pub issue_dir: PathBuf,
    /// True when an existing issue with the same `(provenance,
    /// source_ref)` was returned instead of creating a new one (OD-10).
    pub deduplicated: bool,
}

/// File a new intake item. Creates it directly in the `untriaged`
/// reception state (never `open`, so the transition matrix is not
/// tripped), guards the field surface (OD-6), and is idempotent on
/// `(provenance, source_ref)` (OD-10).
pub fn file(root: &Path, req: FileRequest) -> Result<FileOutcome, IntakeError> {
    if req.issue_type == "epic" {
        return Err(MutateError::Validation(
            "intake file does not accept epics — an epic is planning scaffolding, not a report"
                .into(),
        )
        .into());
    }
    // Guard the field surface: no lifecycle-bearing key via --field.
    for (k, _) in &req.fields {
        if PROTECTED_FILE_FIELDS.contains(&k.as_str()) {
            return Err(IntakeError::ProtectedField(k.clone()));
        }
    }

    let lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    let schema =
        crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;

    // Provenance value-set check (friendly message before schema
    // validation would raise a terser one). Only when the repo narrowed
    // the set with an `enum:`; otherwise any source string is accepted.
    if let Some(allowed) = schema
        .fields
        .get(F_PROVENANCE)
        .and_then(|s| s.allowed.as_ref())
    {
        if !allowed.iter().any(|a| a == &req.provenance) {
            return Err(IntakeError::UnknownProvenance {
                value: req.provenance.clone(),
                accepted: allowed.clone(),
            });
        }
    }

    // Idempotency: a retry with the same (provenance, source_ref) returns
    // the existing item rather than creating a second one.
    if let Some(source_ref) = req.source_ref.as_deref() {
        let matches = matching_source_ref(root, &req.provenance, source_ref);
        match matches.len() {
            0 => {}
            1 => {
                let existing = &matches[0];
                let item_path =
                    super::locate_for_dry_run(root, existing).map_err(IntakeError::from)?;
                let parsed =
                    crate::parser::parse_item_md_with_warnings(&item_path, existing, "open");
                let mut issue = parsed.issue;
                issue.folder = folder_for_status(&schema, &issue.status).to_string();
                return Ok(FileOutcome {
                    slug: existing.clone(),
                    version: canonical_hash(&issue),
                    issue_dir: item_path
                        .parent()
                        .expect("item.md has a parent")
                        .to_path_buf(),
                    deduplicated: true,
                });
            }
            _ => {
                return Err(IntakeError::DuplicateSourceRef {
                    provenance: req.provenance.clone(),
                    source_ref: source_ref.to_string(),
                    existing: matches,
                });
            }
        }
    }

    // Assemble the guarded custom-field set (provenance et al. are set
    // here, not via --field).
    let mut custom_fields: Vec<(String, String)> = Vec::new();
    custom_fields.push((F_PROVENANCE.to_string(), req.provenance.clone()));
    if let Some(d) = &req.provenance_detail {
        custom_fields.push((F_PROVENANCE_DETAIL.to_string(), d.clone()));
    }
    if let Some(s) = &req.source_ref {
        custom_fields.push((F_SOURCE_REF.to_string(), s.clone()));
    }
    custom_fields.extend(req.fields);

    let new_args = NewArgs {
        issue_type: req.issue_type,
        title: req.title,
        // Intake titles come from untrusted external reporters (chat or
        // webform filings, etc.) and may carry customer names / secrets that
        // must not land in a directory or branch name. Default to a random
        // slug unless the filer supplied an explicit one.
        slug: req.slug,
        slug_random: true,
        reporter: req.reporter,
        assignee: None,
        owner: None,
        priority: req.priority.unwrap_or_else(|| "normal".to_string()),
        epic: None,
        labels: req.labels,
        related: vec![],
        source: None,
        description: req.body,
        custom_fields,
        lane: None,
        lane_seq: None,
        collision: vec![],
        status: Some("untriaged".to_string()),
        inbox: false,
    };

    let outcome = do_new_locked(&lock, root, new_args).map_err(MutateError::from)?;
    let parsed =
        crate::parser::parse_item_md_with_warnings(&outcome.item_path, &outcome.slug, "open");
    let mut issue = parsed.issue;
    issue.folder = folder_for_status(&schema, &issue.status).to_string();
    Ok(FileOutcome {
        slug: outcome.slug,
        version: canonical_hash(&issue),
        issue_dir: outcome
            .item_path
            .parent()
            .expect("item.md has a parent")
            .to_path_buf(),
        deduplicated: false,
    })
}

/// Slugs of existing issues carrying this exact `(provenance,
/// source_ref)` pair. Both live in `Issue::extra` (not first-class
/// fields).
fn matching_source_ref(root: &Path, provenance: &str, source_ref: &str) -> Vec<String> {
    crate::repo::load_issues(root)
        .into_iter()
        .filter(|i| {
            extra_str(i, F_PROVENANCE) == Some(provenance)
                && extra_str(i, F_SOURCE_REF) == Some(source_ref)
        })
        .map(|i| i.slug)
        .collect()
}

fn extra_str<'a>(issue: &'a crate::models::Issue, key: &str) -> Option<&'a str> {
    issue.extra.get(key).and_then(|v| v.as_str())
}

// ── Transition verbs ─────────────────────────────────────────────────────

/// A planned intake transition. Built by each verb, executed by
/// [`apply`].
#[derive(Default)]
struct Plan {
    verb: &'static str,
    new_status: Option<String>,
    new_type: Option<String>,
    assignee: Option<String>,
    priority: Option<String>,
    set_fields: Vec<(String, String)>,
    note: Option<String>,
    /// Source states this verb accepts. Empty ⇒ any state (subject to
    /// `require_closing`).
    allowed_source: &'static [&'static str],
    /// When true, the source-state precondition is "currently a closing
    /// status" per the *schema's* lifecycle classification (so a repo's
    /// custom closing status is reopenable, and a built-in reclassified
    /// active is not). Used by `reopen`; supersedes `allowed_source`.
    require_closing: bool,
}

/// Lifecycle metadata whose validity is tied to a specific state. On a
/// status-changing transition, any of these NOT written by the verb is
/// cleared, so a re-disposition (e.g. defer → reject → reopen) never
/// leaves contradictory fields like a dangling `deferred_until` on an
/// `open` item. Verbs that don't change status (`retype`) skip this so
/// they preserve the holding state's fields.
const LIFECYCLE_FIELDS: &[&str] = &[
    F_DISPOSITION_REASON,
    F_DISPOSITION_NOTE,
    F_DUPLICATE_OF,
    F_DEFERRED_UNTIL,
    F_SUPERSEDED_BY,
];

/// Execute a planned intake transition, acquiring the flock.
fn apply(root: &Path, slug: &str, plan: Plan) -> Result<UpdateOutcome, IntakeError> {
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!("invalid slug shape: {slug:?}")).into());
    }
    let lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    let schema =
        crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    apply_locked(&lock, root, slug, plan, &schema)
}

/// Body of [`apply`] with the flock already held. Split out so verbs that
/// must read repo-wide state as part of validation (`duplicate`, whose
/// cycle check reads the whole `duplicate_of` graph) can do that read
/// under the *same* flock as the write — no time-of-check/time-of-use gap.
fn apply_locked(
    _lock: &WriteLock,
    root: &Path,
    slug: &str,
    plan: Plan,
    schema: &crate::schema::Schema,
) -> Result<UpdateOutcome, IntakeError> {
    // Read-only locate first so a precondition failure leaves no repo
    // side effects (mirrors `close_issue`).
    let item_path = super::locate_for_dry_run(root, slug).map_err(IntakeError::from)?;
    let item = write::read_item(&item_path).map_err(MutateError::Io)?;
    let cur_status = item
        .frontmatter
        .get(serde_yaml::Value::String("status".into()))
        .and_then(|v| v.as_str())
        .unwrap_or("open")
        .to_string();

    // Source-state precondition (refused-but-actionable: legal item,
    // wrong state).
    if plan.require_closing {
        if !crate::schema::is_closing(schema, &cur_status) {
            return Err(MutateError::TransitionViolation(format!(
                "cannot {} {slug}: it is {cur_status:?}, which is not a closing status",
                plan.verb
            ))
            .into());
        }
    } else if !plan.allowed_source.is_empty() && !plan.allowed_source.contains(&cur_status.as_str())
    {
        return Err(MutateError::TransitionViolation(format!(
            "cannot {} {slug}: it is {cur_status:?}; {} applies to an item in [{}]",
            plan.verb,
            plan.verb,
            plan.allowed_source.join(", ")
        ))
        .into());
    }

    let mut req = UpdateIssueRequest::default();
    let changes_status = plan.new_status.is_some();
    if let Some(s) = plan.new_status {
        req.status = Patch::Set(s);
    }
    if let Some(t) = plan.new_type {
        req.issue_type = Patch::Set(t);
    }
    if let Some(a) = plan.assignee {
        req.assignee = Patch::Set(a);
    }
    if let Some(p) = plan.priority {
        req.priority = Patch::Set(p);
    }
    for (k, v) in plan.set_fields {
        req.custom_fields.insert(k, Patch::Set(v));
    }
    // Normalize stale lifecycle metadata: on a status change, clear every
    // lifecycle field the target state does not own (i.e. that this verb
    // did not just set).
    if changes_status {
        for field in LIFECYCLE_FIELDS {
            req.custom_fields
                .entry(field.to_string())
                .or_insert(Patch::Clear);
        }
    }
    if let Some(msg) = plan.note {
        req.body_ops.push(BodyOp::AppendNote(AppendNoteOp {
            author: NOTE_AUTHOR.to_string(),
            message: msg,
            section: NoteSection::Comments,
        }));
    }
    req.validate().map_err(IntakeError::from)?;
    let rules = super::load_validated_rules(root, schema).map_err(IntakeError::from)?;
    super::update_issue_under_lock(
        root,
        slug,
        item_path,
        req,
        schema,
        &rules,
        &crate::clock::SystemClock,
    )
    .map_err(IntakeError::from)
}

/// `untriaged|deferred|needs-info → open`. Refuses a closed item
/// (intrinsic invariant "cannot accept a closed item").
pub fn accept(
    root: &Path,
    slug: &str,
    assignee: Option<String>,
    priority: Option<String>,
) -> Result<UpdateOutcome, IntakeError> {
    apply(
        root,
        slug,
        Plan {
            verb: "accept",
            new_status: Some("open".into()),
            assignee,
            priority,
            allowed_source: INTAKE_STATES,
            ..Default::default()
        },
    )
}

/// `→ deferred`, recording the wake-up date and the reason.
pub fn defer(
    root: &Path,
    slug: &str,
    reason: &str,
    until: Option<String>,
) -> Result<UpdateOutcome, IntakeError> {
    let mut set_fields = vec![(F_DISPOSITION_NOTE.to_string(), reason.trim().to_string())];
    if let Some(u) = until {
        set_fields.push((F_DEFERRED_UNTIL.to_string(), u));
    }
    apply(
        root,
        slug,
        Plan {
            verb: "defer",
            new_status: Some("deferred".into()),
            set_fields,
            note: Some(format!("Deferred: {}", reason.trim())),
            allowed_source: &["untriaged", "needs-info", "open"],
            ..Default::default()
        },
    )
}

/// `→ needs-info`, awaiting reporter input.
pub fn need_info(root: &Path, slug: &str, reason: &str) -> Result<UpdateOutcome, IntakeError> {
    apply(
        root,
        slug,
        Plan {
            verb: "need-info",
            new_status: Some("needs-info".into()),
            set_fields: vec![(F_DISPOSITION_NOTE.to_string(), reason.trim().to_string())],
            note: Some(format!("Needs info: {}", reason.trim())),
            allowed_source: &["untriaged", "deferred", "open"],
            ..Default::default()
        },
    )
}

/// The `--kind` for [`reject`], mapped onto the `disposition_reason`
/// enum.
#[derive(Debug, Clone, Copy)]
pub enum RejectKind {
    ByDesign,
    Wontfix,
    OutOfScope,
}

impl RejectKind {
    fn as_reason(self) -> &'static str {
        match self {
            RejectKind::ByDesign => "by-design",
            RejectKind::Wontfix => "wontfix",
            RejectKind::OutOfScope => "out-of-scope",
        }
    }
}

/// `→ wontfix` with a structured `disposition_reason` (the kind) and the
/// free-text reason.
pub fn reject(
    root: &Path,
    slug: &str,
    kind: RejectKind,
    reason: &str,
) -> Result<UpdateOutcome, IntakeError> {
    apply(
        root,
        slug,
        Plan {
            verb: "reject",
            new_status: Some("wontfix".into()),
            set_fields: vec![
                (
                    F_DISPOSITION_REASON.to_string(),
                    kind.as_reason().to_string(),
                ),
                (F_DISPOSITION_NOTE.to_string(), reason.trim().to_string()),
            ],
            note: Some(format!(
                "Rejected ({}): {}",
                kind.as_reason(),
                reason.trim()
            )),
            // In-flight work can also be dropped (matches the default
            // matrix's `wontfix.allowed_from`), so first-class `reject`
            // and generic `set status wontfix` stay consistent.
            allowed_source: &[
                "untriaged",
                "deferred",
                "needs-info",
                "open",
                "in-progress",
                "testing",
            ],
            ..Default::default()
        },
    )
}

/// `→ cannot-reproduce` (bug-only; the type × status invariant enforces
/// that under lock).
pub fn cannot_reproduce(
    root: &Path,
    slug: &str,
    reason: &str,
) -> Result<UpdateOutcome, IntakeError> {
    apply(
        root,
        slug,
        Plan {
            verb: "cannot-reproduce",
            new_status: Some("cannot-reproduce".into()),
            set_fields: vec![(F_DISPOSITION_NOTE.to_string(), reason.trim().to_string())],
            note: Some(format!("Cannot reproduce: {}", reason.trim())),
            allowed_source: &["untriaged", "deferred", "needs-info", "open"],
            ..Default::default()
        },
    )
}

/// `→ duplicate` with a directed `duplicate_of` link. Rejects
/// self-duplicates, missing targets, and cycles.
pub fn duplicate(root: &Path, slug: &str, of: &str) -> Result<UpdateOutcome, IntakeError> {
    if of == slug {
        return Err(IntakeError::DuplicateSelf(slug.to_string()));
    }
    if !crate::slug::is_valid(slug) {
        return Err(MutateError::Validation(format!("invalid slug shape: {slug:?}")).into());
    }
    // Acquire the flock BEFORE loading the graph so the existence + cycle
    // check and the write happen under one lock — no time-of-check /
    // time-of-use gap where a concurrent writer could insert an edge that
    // makes this one cyclic.
    let lock = WriteLock::acquire(root).map_err(MutateError::Io)?;
    let schema =
        crate::schema::load(root).map_err(|e| MutateError::SchemaConfig(format!("{e:#}")))?;
    let issues = crate::repo::load_issues(root);
    if !issues.iter().any(|i| i.slug == of) {
        return Err(IntakeError::DuplicateTargetMissing(of.to_string()));
    }
    let dup_of: std::collections::BTreeMap<&str, &str> = issues
        .iter()
        .filter_map(|i| extra_str(i, F_DUPLICATE_OF).map(|d| (i.slug.as_str(), d)))
        .collect();
    // Walk `of`'s chain; if it reaches `slug`, the new edge closes a loop.
    let mut chain = vec![of.to_string()];
    let mut cursor = of;
    let mut seen = std::collections::BTreeSet::new();
    while let Some(next) = dup_of.get(cursor) {
        if *next == slug {
            chain.push(slug.to_string());
            return Err(IntakeError::DuplicateCycle {
                of: of.to_string(),
                chain,
            });
        }
        if !seen.insert(*next) {
            break; // pre-existing loop not involving slug; leave it be
        }
        chain.push((*next).to_string());
        cursor = next;
    }
    apply_locked(
        &lock,
        root,
        slug,
        Plan {
            verb: "duplicate",
            new_status: Some("duplicate".into()),
            set_fields: vec![(F_DUPLICATE_OF.to_string(), of.to_string())],
            note: Some(format!("Duplicate of {of}")),
            allowed_source: &["untriaged", "deferred", "needs-info", "open"],
            ..Default::default()
        },
        &schema,
    )
}

/// `→ obsolete`, optionally recording what superseded it.
pub fn obsolete(
    root: &Path,
    slug: &str,
    reason: &str,
    superseded_by: Option<String>,
) -> Result<UpdateOutcome, IntakeError> {
    let mut set_fields = vec![(F_DISPOSITION_NOTE.to_string(), reason.trim().to_string())];
    let mut note = format!("Obsolete: {}", reason.trim());
    if let Some(sup) = &superseded_by {
        set_fields.push((F_DISPOSITION_REASON.to_string(), "superseded".to_string()));
        set_fields.push((F_SUPERSEDED_BY.to_string(), sup.clone()));
        note.push_str(&format!(" (superseded by {sup})"));
    }
    apply(
        root,
        slug,
        Plan {
            verb: "obsolete",
            new_status: Some("obsolete".into()),
            set_fields,
            note: Some(note),
            allowed_source: &["untriaged", "deferred", "needs-info", "open"],
            ..Default::default()
        },
    )
}

/// Reclassify `type` (OD-13). Valid only while the item is in an intake
/// state; no status change. Rejects `epic` — an epic is planning
/// scaffolding, not a triageable report (mirrors [`file`]).
pub fn retype(root: &Path, slug: &str, to: &str) -> Result<UpdateOutcome, IntakeError> {
    if to == "epic" {
        return Err(MutateError::Validation(
            "cannot retype an intake item to 'epic' — an epic is not a report".into(),
        )
        .into());
    }
    apply(
        root,
        slug,
        Plan {
            verb: "retype",
            new_type: Some(to.to_string()),
            note: Some(format!("Retyped → {to}")),
            allowed_source: INTAKE_STATES,
            ..Default::default()
        },
    )
}

/// Reopen a closed item back into an active state (`untriaged` by
/// default, or `open`). The `changes_status` normalization in
/// `apply_locked` clears every stale lifecycle field; the reason lives
/// only in the audit note (and the `## Reopen Notes` section the shared
/// path appends), not in a dangling `disposition_note`.
pub fn reopen(
    root: &Path,
    slug: &str,
    to: Option<String>,
    reason: &str,
) -> Result<UpdateOutcome, IntakeError> {
    let target = to.unwrap_or_else(|| "untriaged".to_string());
    apply(
        root,
        slug,
        Plan {
            verb: "reopen",
            new_status: Some(target),
            note: Some(format!("Reopened: {}", reason.trim())),
            // Source must currently be a closing status per the schema's
            // lifecycle classification (honours custom / reclassified
            // statuses, unlike a hard-coded list).
            require_closing: true,
            ..Default::default()
        },
    )
}

/// Reporter retracts their own untriaged report: `untriaged → wontfix`
/// with `disposition_reason: withdrawn`.
pub fn withdraw(root: &Path, slug: &str, reason: &str) -> Result<UpdateOutcome, IntakeError> {
    apply(
        root,
        slug,
        Plan {
            verb: "withdraw",
            new_status: Some("wontfix".into()),
            set_fields: vec![
                (F_DISPOSITION_REASON.to_string(), "withdrawn".to_string()),
                (F_DISPOSITION_NOTE.to_string(), reason.trim().to_string()),
            ],
            note: Some(format!("Withdrawn by reporter: {}", reason.trim())),
            allowed_source: &["untriaged"],
            ..Default::default()
        },
    )
}

// ── Intrinsic invariants (shared with generic `set status`) ──────────────

/// Always-on, config-independent transition invariants (OD-9 A). Called
/// from `super::update_issue_under_lock` whenever a write changes
/// `status` or `type`, so the generic `set status` path enforces the
/// same rules as the intake verbs. Returns one message per violation.
pub(crate) fn intrinsic_transition_violations(
    prev_status: &str,
    _prev_type: &str,
    new_status: &str,
    new_type: &str,
) -> Vec<String> {
    let mut out = Vec::new();

    // Type × status completion compatibility.
    match new_status {
        "fixed" if new_type != "bug" => out.push(format!(
            "status 'fixed' is bug-only — a {new_type} completes as 'done'"
        )),
        "done" if new_type == "bug" => {
            out.push("status 'done' is for non-bug work — a bug completes as 'fixed'".to_string())
        }
        "cannot-reproduce" if new_type != "bug" => out.push(format!(
            "status 'cannot-reproduce' is bug-only (type is {new_type})"
        )),
        _ => {}
    }

    // Work cannot begin straight from reception.
    if prev_status == "untriaged" && matches!(new_status, "in-progress" | "testing") {
        out.push(format!(
            "cannot start work from 'untriaged' — accept it into 'open' first (untriaged → {new_status} is illegal)"
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fresh_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        tmp
    }

    /// Repo with the default transition matrix installed, to exercise the
    /// graph rules on top of the intrinsic invariants.
    fn repo_with_rules() -> TempDir {
        let tmp = fresh_repo();
        crate::transitions::write_default(tmp.path(), false).unwrap();
        tmp
    }

    fn file_bug(root: &Path, title: &str, slug: &str) -> FileOutcome {
        file(
            root,
            FileRequest {
                issue_type: "bug".into(),
                title: title.into(),
                body: Some("something is broken".into()),
                reporter: Some("alice".into()),
                provenance: "chat".into(),
                provenance_detail: None,
                source_ref: None,
                priority: None,
                slug: Some(slug.into()),
                labels: vec![],
                fields: vec![],
            },
        )
        .expect("file should succeed")
    }

    fn status_of(root: &Path, slug: &str) -> String {
        crate::repo::load_issues(root)
            .into_iter()
            .find(|i| i.slug == slug)
            .unwrap()
            .status
    }

    fn read_body(root: &Path, slug: &str) -> String {
        let path = root.join("issues").join(slug).join("item.md");
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn file_creates_untriaged_with_provenance() {
        let tmp = fresh_repo();
        let out = file_bug(tmp.path(), "Login loops", "login-loops");
        assert!(!out.deduplicated);
        assert_eq!(status_of(tmp.path(), "login-loops"), "untriaged");
        let body = read_body(tmp.path(), "login-loops");
        assert!(body.contains("status: untriaged"), "{body}");
        assert!(body.contains("provenance: chat"), "{body}");
        assert!(body.contains("reporter: alice"), "{body}");
    }

    #[test]
    fn file_is_idempotent_on_source_ref() {
        let tmp = fresh_repo();
        let mk = || FileRequest {
            issue_type: "bug".into(),
            title: "Crash".into(),
            body: Some("boom".into()),
            reporter: Some("bot".into()),
            provenance: "chat".into(),
            provenance_detail: None,
            source_ref: Some("chat:1/msg:2".into()),
            priority: None,
            slug: None,
            labels: vec![],
            fields: vec![],
        };
        let first = file(tmp.path(), mk()).unwrap();
        assert!(!first.deduplicated);
        let second = file(tmp.path(), mk()).unwrap();
        assert!(second.deduplicated, "retry must dedup");
        assert_eq!(first.slug, second.slug);
        // Exactly one issue on disk.
        assert_eq!(crate::repo::load_issues(tmp.path()).len(), 1);
    }

    #[test]
    fn file_rejects_protected_field() {
        let tmp = fresh_repo();
        let err = file(
            tmp.path(),
            FileRequest {
                issue_type: "bug".into(),
                title: "Spoof".into(),
                body: Some("x".into()),
                reporter: None,
                provenance: "chat".into(),
                provenance_detail: None,
                source_ref: None,
                priority: None,
                slug: Some("spoof-attempt".into()),
                labels: vec![],
                fields: vec![("status".into(), "fixed".into())],
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "protected-field");
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn file_rejects_unknown_provenance_when_repo_narrows_it() {
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  provenance:\n    enum: [chat, email]\n",
        )
        .unwrap();
        let err = file(
            tmp.path(),
            FileRequest {
                issue_type: "bug".into(),
                title: "Pigeon".into(),
                body: Some("x".into()),
                reporter: None,
                provenance: "carrier-pigeon".into(),
                provenance_detail: None,
                source_ref: None,
                priority: None,
                slug: Some("pigeon-post".into()),
                labels: vec![],
                fields: vec![],
            },
        )
        .unwrap_err();
        match err {
            IntakeError::UnknownProvenance { accepted, .. } => {
                assert_eq!(accepted, vec!["chat", "email"]);
            }
            other => panic!("expected UnknownProvenance, got {other:?}"),
        }
    }

    #[test]
    fn accept_moves_to_open_and_refuses_closed() {
        let tmp = repo_with_rules();
        file_bug(tmp.path(), "A", "accept-me");
        accept(tmp.path(), "accept-me", Some("bob".into()), None).unwrap();
        assert_eq!(status_of(tmp.path(), "accept-me"), "open");

        // Now close it, then accept must refuse (cannot accept a closed item).
        file_bug(tmp.path(), "B", "closed-item");
        reject(tmp.path(), "closed-item", RejectKind::Wontfix, "nope").unwrap();
        let err = accept(tmp.path(), "closed-item", None, None).unwrap_err();
        assert_eq!(err.code(), "transition-illegal");
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn defer_records_reason_field_and_note() {
        let tmp = fresh_repo();
        file_bug(tmp.path(), "Later", "do-later");
        defer(
            tmp.path(),
            "do-later",
            "no capacity this quarter",
            Some("2026-12-01".into()),
        )
        .unwrap();
        assert_eq!(status_of(tmp.path(), "do-later"), "deferred");
        let body = read_body(tmp.path(), "do-later");
        assert!(body.contains("deferred_until: 2026-12-01"), "{body}");
        assert!(body.contains("disposition_note:"), "{body}");
        assert!(body.contains("## Comments"), "{body}");
        assert!(body.contains("no capacity this quarter"), "{body}");
    }

    #[test]
    fn reject_sets_disposition_reason_from_kind() {
        let tmp = fresh_repo();
        file_bug(tmp.path(), "WAI", "works-as-intended");
        reject(
            tmp.path(),
            "works-as-intended",
            RejectKind::ByDesign,
            "intended behaviour",
        )
        .unwrap();
        let body = read_body(tmp.path(), "works-as-intended");
        assert!(body.contains("status: wontfix"), "{body}");
        assert!(body.contains("disposition_reason: by-design"), "{body}");
    }

    #[test]
    fn cannot_reproduce_is_bug_only() {
        let tmp = fresh_repo();
        // A feature filed, then cannot-reproduce → intrinsic type×status rejects.
        file(
            tmp.path(),
            FileRequest {
                issue_type: "feature".into(),
                title: "Feature".into(),
                body: Some("want".into()),
                reporter: None,
                provenance: "email".into(),
                provenance_detail: None,
                source_ref: None,
                priority: None,
                slug: Some("shiny-feature".into()),
                labels: vec![],
                fields: vec![],
            },
        )
        .unwrap();
        let err = cannot_reproduce(tmp.path(), "shiny-feature", "n/a").unwrap_err();
        assert_eq!(err.code(), "transition-illegal");
    }

    #[test]
    fn duplicate_rejects_self_missing_and_cycle() {
        let tmp = fresh_repo();
        file_bug(tmp.path(), "A", "issue-a");
        file_bug(tmp.path(), "B", "issue-b");

        // self
        let e = duplicate(tmp.path(), "issue-a", "issue-a").unwrap_err();
        assert!(matches!(e, IntakeError::DuplicateSelf(_)));
        // missing target
        let e = duplicate(tmp.path(), "issue-a", "ghost-slug").unwrap_err();
        assert!(matches!(e, IntakeError::DuplicateTargetMissing(_)));
        // cycle: a → b, then b → a must be rejected
        duplicate(tmp.path(), "issue-a", "issue-b").unwrap();
        let e = duplicate(tmp.path(), "issue-b", "issue-a").unwrap_err();
        assert!(matches!(e, IntakeError::DuplicateCycle { .. }), "{e}");
    }

    #[test]
    fn reopen_from_closing_clears_disposition() {
        let tmp = repo_with_rules();
        file_bug(tmp.path(), "Regressed", "regressed-bug");
        reject(tmp.path(), "regressed-bug", RejectKind::Wontfix, "later").unwrap();
        reopen(tmp.path(), "regressed-bug", None, "reproduced on main").unwrap();
        assert_eq!(status_of(tmp.path(), "regressed-bug"), "untriaged");
        let body = read_body(tmp.path(), "regressed-bug");
        assert!(!body.contains("disposition_reason:"), "cleared; {body}");
    }

    #[test]
    fn transitions_clear_stale_lifecycle_metadata() {
        // defer (sets deferred_until) → accept (→ open) must not leave a
        // dangling deferred_until, and a later reject must not carry it
        // either. Guards against contradictory frontmatter.
        let tmp = fresh_repo();
        file_bug(tmp.path(), "Parked", "parked-item");
        defer(
            tmp.path(),
            "parked-item",
            "later",
            Some("2026-12-01".into()),
        )
        .unwrap();
        assert!(read_body(tmp.path(), "parked-item").contains("deferred_until:"));

        accept(tmp.path(), "parked-item", None, None).unwrap();
        let body = read_body(tmp.path(), "parked-item");
        assert!(
            !body.contains("deferred_until:"),
            "accept must clear the wake-up date; {body}"
        );
    }

    #[test]
    fn retype_preserves_holding_state_fields() {
        // retype does not change status, so it must NOT wipe a deferred
        // item's wake-up date.
        let tmp = fresh_repo();
        file_bug(tmp.path(), "Reclass", "reclass-item");
        defer(
            tmp.path(),
            "reclass-item",
            "later",
            Some("2026-12-01".into()),
        )
        .unwrap();
        retype(tmp.path(), "reclass-item", "feature").unwrap();
        let body = read_body(tmp.path(), "reclass-item");
        assert!(
            body.contains("deferred_until:"),
            "retype must preserve; {body}"
        );
        assert!(body.contains("type: feature"), "{body}");
    }

    #[test]
    fn file_rejects_protected_source_ref_via_field() {
        let tmp = fresh_repo();
        let err = file(
            tmp.path(),
            FileRequest {
                issue_type: "bug".into(),
                title: "Sneaky".into(),
                body: Some("x".into()),
                reporter: None,
                provenance: "chat".into(),
                provenance_detail: None,
                source_ref: Some("A".into()),
                priority: None,
                slug: Some("sneaky-ref".into()),
                labels: vec![],
                fields: vec![("source_ref".into(), "B".into())],
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "protected-field");
    }

    #[test]
    fn retype_rejects_epic() {
        let tmp = fresh_repo();
        file_bug(tmp.path(), "NotAnEpic", "not-an-epic");
        let err = retype(tmp.path(), "not-an-epic", "epic").unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn withdraw_only_from_untriaged() {
        let tmp = fresh_repo();
        file_bug(tmp.path(), "Oops", "mistaken-report");
        // Move out of untriaged, then withdraw must refuse.
        accept(tmp.path(), "mistaken-report", None, None).unwrap();
        let err = withdraw(tmp.path(), "mistaken-report", "retract").unwrap_err();
        assert_eq!(err.code(), "transition-illegal");
    }

    #[test]
    fn retype_only_in_intake_state() {
        let tmp = fresh_repo();
        file_bug(tmp.path(), "Actually a feature", "misfiled-report");
        retype(tmp.path(), "misfiled-report", "feature").unwrap();
        assert_eq!(
            crate::repo::load_issues(tmp.path())
                .into_iter()
                .find(|i| i.slug == "misfiled-report")
                .unwrap()
                .issue_type,
            "feature"
        );
        // Accept it, then retype must refuse (not an intake state).
        accept(tmp.path(), "misfiled-report", None, None).unwrap();
        let err = retype(tmp.path(), "misfiled-report", "task").unwrap_err();
        assert_eq!(err.code(), "transition-illegal");
    }

    #[test]
    fn intrinsic_untriaged_cannot_start_work_via_generic_set() {
        // The generic update path must be gated the same way: a direct
        // status jump untriaged→in-progress is refused.
        let tmp = fresh_repo();
        file_bug(tmp.path(), "Jump", "queue-jumper");
        let req = UpdateIssueRequest {
            status: Patch::Set("in-progress".into()),
            ..Default::default()
        };
        let err = super::super::update_issue(tmp.path(), "queue-jumper", req).unwrap_err();
        assert!(matches!(err, MutateError::TransitionViolation(_)), "{err}");
    }

    #[test]
    fn intrinsic_violations_unit() {
        // feature → done and bug → fixed are the compatible completions.
        assert!(intrinsic_transition_violations("open", "feature", "done", "feature").is_empty());
        assert!(intrinsic_transition_violations("testing", "bug", "fixed", "bug").is_empty());
        // The incompatible completions are rejected.
        assert!(!intrinsic_transition_violations("open", "bug", "done", "bug").is_empty());
        assert!(
            !intrinsic_transition_violations("testing", "feature", "fixed", "feature").is_empty()
        );
        // Reception state cannot jump straight into work.
        assert!(
            !intrinsic_transition_violations("untriaged", "bug", "in-progress", "bug").is_empty()
        );
    }
}
