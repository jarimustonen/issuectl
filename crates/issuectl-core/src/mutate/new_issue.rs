//! New-issue creation. Owns the typed input (`NewArgs`), output
//! (`WriteOutcome`), and error (`DoNewError`) shapes shared by the
//! CLI `cmd_new` handler and the server-side `mutate::new_issue`
//! boundary.
//!
//! Lives under `mutate/` because `do_new_locked` is the third leg of
//! the mutation triad alongside `update_issue` / `close_issue` /
//! `note_issue` in `mutate/mod.rs` — same lock contract, same canonical
//! versioning, same error taxonomy. The on-disk render helpers
//! (`build_new_frontmatter`, `render_new_item_from_fm`, `slugify`) it
//! calls remain in `crate::write::` as serialization primitives.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::{MutateError, WriteLock};
use crate::{refs, repo, schema, slug, write};

/// Canonical name of the per-issue attachments directory
/// (`issues/<slug>/attachments/`) — screenshots, logs, and other binary
/// artifacts referenced from the body via relative paths.
pub const ATTACHMENTS_DIRNAME: &str = "attachments";

/// Canonical name of the per-issue fixtures directory
/// (`issues/<slug>/fixtures/`) — reproduction inputs a bug-fixing agent
/// can run against while working the issue.
pub const FIXTURES_DIRNAME: &str = "fixtures";

/// Create (idempotently) one of the canonical per-issue subdirectories
/// under an existing issue directory and return its path. `name` must be
/// [`ATTACHMENTS_DIRNAME`] or [`FIXTURES_DIRNAME`].
///
/// Deliberately NOT called from `issuectl new`: git does not track empty
/// directories, so eagerly scaffolding `attachments/` + `fixtures/` into
/// every new issue would litter the tree with dirs that vanish on clone.
/// Instead a caller materialises the directory at the moment it writes
/// the first file into it.
pub fn ensure_issue_subdir(issue_dir: &Path, name: &str) -> Result<PathBuf> {
    if name != ATTACHMENTS_DIRNAME && name != FIXTURES_DIRNAME {
        bail!("unknown issue subdir {name:?} (expected {ATTACHMENTS_DIRNAME:?} or {FIXTURES_DIRNAME:?})");
    }
    let dir = issue_dir.join(name);
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    Ok(dir)
}

pub struct NewArgs {
    pub issue_type: String,
    pub title: String,
    pub slug: Option<String>,
    /// Force a random `intensifier-adjective-noun` slug instead of the
    /// title-derived default (`issuectl new --slug-random`). Ignored when
    /// `slug` is `Some` — an explicit `--slug` always wins. Use this for a
    /// title that would leak sensitive data into the directory / branch
    /// name, or when the derived slug simply isn't wanted.
    pub slug_random: bool,
    pub reporter: Option<String>,
    pub assignee: Option<String>,
    pub owner: Option<String>,
    pub priority: String,
    pub epic: Option<String>,
    pub labels: Vec<String>,
    pub related: Vec<String>,
    pub source: Option<String>,
    pub description: Option<String>,
    pub custom_fields: Vec<(String, String)>,
    /// Scheduling lane set at creation (`issuectl new --lane`). `None`
    /// leaves the issue unclassified; `Some` writes a string `lane:`
    /// key. Mirrors the `update --lane` setter — same non-empty gate,
    /// same reserved-custom-field key — so an issue can be born into
    /// the DAG in one call. See `crate::dag`.
    pub lane: Option<String>,
    /// Coarse intra-lane precedence key set at creation
    /// (`issuectl new --lane-seq`). `None` omits it; `Some` writes an
    /// unquoted YAML integer `lane_seq:`. Numeric mirror of `lane`;
    /// undeclared in the schema (like `commits`/`estimate`) but known
    /// to doctor. See `crate::dag`.
    pub lane_seq: Option<i64>,
    /// Collision hot-file tokens set at creation
    /// (`issuectl new --add-collision`, repeatable). Empty leaves no
    /// `collision:` key; non-empty writes a deduped string list, the
    /// same list mechanics as `update --add-collision`.
    pub collision: Vec<String>,
    /// Creation status override. `None` ⇒ `open` (the historical
    /// default). Set to `untriaged` by `mutate::intake::file` so a filed
    /// item is created directly in its reception state.
    pub status: Option<String>,
    /// Drop the new issue under `issues/inbox/<slug>/` instead of the
    /// canonical flat root. Default `false`. Inbox issues stay out of
    /// `ls` by default and are promoted with `issuectl triage <slug>`.
    pub inbox: bool,
}

impl Default for NewArgs {
    fn default() -> Self {
        Self {
            issue_type: "bug".into(),
            title: String::new(),
            slug: None,
            slug_random: false,
            reporter: None,
            assignee: None,
            owner: None,
            priority: "normal".into(),
            epic: None,
            labels: Vec::new(),
            related: Vec::new(),
            source: None,
            description: None,
            custom_fields: Vec::new(),
            lane: None,
            lane_seq: None,
            collision: Vec::new(),
            status: None,
            inbox: false,
        }
    }
}

#[derive(Debug)]
pub struct WriteOutcome {
    pub slug: String,
    pub title: String,
    pub item_path: PathBuf,
    /// Non-fatal authoring-time advisories — currently reserved-legacy
    /// section headings in the body (`## Notes`). Surfaced through the
    /// shared `warnings` output field; never blocks the create.
    pub warnings: Vec<String>,
    /// Scheduling-DAG fields as committed to disk, captured while the
    /// creation lock is still held. Callers (e.g. `cmd_new --json`) echo
    /// these instead of re-reading the file — a post-lock reread would
    /// race a concurrent writer and could report another process's state
    /// (the same rule `UpdateOutcome` follows). `collision` is the deduped
    /// on-disk list; `lane`/`lane_seq` are `None` when unset.
    pub lane: Option<String>,
    pub lane_seq: Option<i64>,
    pub collision: Vec<String>,
}

/// Typed error surfaced by `do_new_locked`. Maps to `MutateError` via
/// the `From` impl below so the API picks the right HTTP status without
/// string-matching the formatted `anyhow::Error`.
#[derive(Debug)]
pub enum DoNewError {
    Validation(String),
    Conflict(String),
    SchemaViolation(String),
    SchemaConfig(String),
    /// `.issuectl/transitions.yaml` failed to load — distinct from
    /// `SchemaConfig` so the operator gets routed to the right file.
    TransitionConfig(String),
    Io(anyhow::Error),
}

/// **CLI-only** flattening for the `cmd_new` path, which still wants an
/// `anyhow::Error`. The server-side `mutate::new_issue` boundary must
/// keep using the explicit `MutateError::from(DoNewError)` mapping so
/// HTTP status codes survive — never `?`-propagate a `DoNewError`
/// through API code, since this impl collapses every variant into a
/// flat string and the variant tag is lost.
impl From<DoNewError> for anyhow::Error {
    fn from(e: DoNewError) -> Self {
        match e {
            DoNewError::Io(e) => e,
            DoNewError::Validation(s) | DoNewError::Conflict(s) | DoNewError::SchemaConfig(s) => {
                anyhow::Error::msg(s)
            }
            DoNewError::TransitionConfig(s) => {
                anyhow::Error::msg(format!("transition config: {s}"))
            }
            DoNewError::SchemaViolation(s) => anyhow::Error::msg(format!("schema: {s}")),
        }
    }
}

impl From<DoNewError> for MutateError {
    fn from(e: DoNewError) -> Self {
        match e {
            DoNewError::SchemaViolation(s) => MutateError::SchemaViolation(s),
            DoNewError::SchemaConfig(s) => MutateError::SchemaConfig(s),
            DoNewError::TransitionConfig(s) => MutateError::TransitionConfig(s),
            DoNewError::Conflict(s) => MutateError::ConflictingIntent(s),
            DoNewError::Validation(s) => MutateError::Validation(s),
            DoNewError::Io(e) => MutateError::Io(e),
        }
    }
}

pub fn do_new(root: &Path, args: NewArgs) -> Result<WriteOutcome> {
    // M1 contract: every issuectl-mediated writer holds the repo
    // `flock`. Without this acquire, concurrent `issuectl new` from
    // the terminal would race against server-side mutations and
    // bypass the protocol's serialization guarantee.
    let lock = WriteLock::acquire(root)?;
    Ok(do_new_locked(&lock, root, args)?)
}

/// Body of `do_new` that assumes the caller holds the repo `WriteLock`.
/// Server-side `mutate::new_issue` uses this so it can hold the same
/// lock through the post-write parse + publish — without splitting the
/// sequence the synthetic `IssueUpserted` lands AFTER the lock is
/// released, inverting seq order against concurrent writers (C3).
pub(crate) fn do_new_locked(
    _lock: &WriteLock,
    root: &Path,
    args: NewArgs,
) -> std::result::Result<WriteOutcome, DoNewError> {
    schema::ensure_default_written(root).map_err(DoNewError::Io)?;
    if args.issue_type == "epic" {
        if args.assignee.is_some() || args.reporter.is_some() {
            return Err(DoNewError::Validation(
                "epics use --owner, not --reporter/--assignee".into(),
            ));
        }
    } else if args.owner.is_some() {
        return Err(DoNewError::Validation(
            "--owner is only valid with --type epic".into(),
        ));
    }

    {
        // Reject `--field foo=a --field foo=b`. Silently letting the
        // last occurrence win is a reasonable default for many CLI
        // tools, but here it would mean the validated frontmatter and
        // the user's apparent intent diverge — better to fail loudly.
        let mut seen = std::collections::BTreeSet::new();
        for (k, _) in &args.custom_fields {
            if !seen.insert(k.as_str()) {
                return Err(DoNewError::Validation(format!(
                    "--field {k:?} given more than once"
                )));
            }
        }
    }

    // Shared key-shape + reserved-built-in + value-trim gate. Belt-and-
    // braces for the CLI (clap's `parse_custom_field` already rejects
    // these) and primary defense for the API new path, which previously
    // let bad keys / whitespace-only values reach frontmatter rendering.
    for (key, value) in &args.custom_fields {
        super::validate_custom_field_key(key).map_err(DoNewError::Validation)?;
        super::validate_custom_field_value(key, value).map_err(DoNewError::Validation)?;
    }

    // Scheduling-DAG fields, set at creation. Mirror the `update --lane`
    // / `--lane-seq` / `--add-collision` validation *exactly*: `lane` must
    // be non-empty (parity with `check_set_nonempty("lane", …)`), and no
    // `collision:` token may be empty (parity with the `add_collision`
    // empty-element gate). Both use `is_empty()`, not `trim().is_empty()`,
    // so this path neither rejects nor accepts a value the `update` core
    // path treats differently — the CLI's `parse_non_empty` is the layer
    // that rejects whitespace for both verbs. Belt-and-braces for the CLI,
    // primary defense for the API `new` path.
    if let Some(l) = &args.lane {
        if l.is_empty() {
            return Err(DoNewError::Validation(
                "lane: empty-string Set is not allowed (omit --lane to leave the issue unclassified)"
                    .into(),
            ));
        }
    }
    if args.collision.iter().any(|c| c.is_empty()) {
        return Err(DoNewError::Validation(
            "collision contains an empty string element".into(),
        ));
    }

    let related = refs::normalize_related_refs(&args.related)
        .map_err(|e| DoNewError::Validation(format!("{e:#}")))?;

    let new_args = write::NewIssueArgs {
        title: &args.title,
        issue_type: &args.issue_type,
        priority: &args.priority,
        reporter: args.reporter.as_deref(),
        assignee: args.assignee.as_deref(),
        owner: args.owner.as_deref(),
        epic: args.epic.as_deref(),
        labels: &args.labels,
        related: &related,
        source: args.source.as_deref(),
        description: args.description.as_deref(),
        status: args.status.as_deref(),
        custom_fields: &args.custom_fields,
    };
    // Build the frontmatter mapping and validate it BEFORE serializing.
    // Validating the in-memory Mapping avoids the round-trip through
    // string parsing that the previous version used (and that subtly
    // duplicated the fragile `find("\n---")` splitter logic).
    let mut frontmatter = write::build_new_frontmatter(&new_args);
    // Project the scheduling-DAG fields into the freshly-built mapping
    // BEFORE schema validation, reusing the exact `write::` setters the
    // `update --lane` / `--lane-seq` / `--add-collision` path uses:
    // `set_string` for the string `lane:`, `set_i64` for the unquoted
    // integer `lane_seq:`, and `add_to_string_list` (which dedupes) for
    // the `collision:` list. A `NewArgs` that sets none of the three
    // leaves `frontmatter` byte-identical to the pre-field shape, so an
    // issue born without a lane still hashes to the golden vector.
    if let Some(l) = &args.lane {
        write::set_string(&mut frontmatter, "lane", l);
    }
    if let Some(n) = args.lane_seq {
        write::set_i64(&mut frontmatter, "lane_seq", n);
    }
    for c in &args.collision {
        write::add_to_string_list(&mut frontmatter, "collision", c).map_err(DoNewError::Io)?;
    }
    // Read the committed `collision:` list back off the mapping so the
    // returned value reflects the on-disk truth (deduped, in first-seen
    // order by `add_to_string_list`) rather than the raw argv. Captured
    // under the creation lock for the caller's `--json` echo.
    let committed_collision: Vec<String> = frontmatter
        .get(serde_yaml::Value::String("collision".into()))
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let schema =
        crate::schema::load(root).map_err(|e| DoNewError::SchemaConfig(format!("{e:#}")))?;
    {
        let violations = schema::validate(&schema, &frontmatter);
        if !violations.is_empty() {
            let msg = violations
                .iter()
                .map(|v| v.message())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(DoNewError::SchemaViolation(msg));
        }
    }
    // Body-section stubs: per-type required H2 sections from
    // `issues/.schema.yaml`'s `body_sections` map. Empty when the
    // type isn't declared (today's lenient default — no behavioural
    // break). Filtered through `all_h2_sections` of the rendered
    // body so a `--description` that already includes a required
    // heading doesn't get a duplicate appended.
    let render = write::render_new_item_from_fm(&new_args, &frontmatter);
    let required_sections = schema::required_sections_for_type(&schema, &args.issue_type);
    let render = if required_sections.is_empty() {
        render
    } else {
        let body_only = render.split("---\n\n").nth(1).unwrap_or(&render);
        let present = crate::body_sections::all_h2_sections(body_only);
        let missing: Vec<String> = required_sections
            .iter()
            .filter(|name| !present.contains_key(name.as_str()))
            .cloned()
            .collect();
        let extra_sections = schema::stub_for_sections(&missing);
        if extra_sections.is_empty() {
            render
        } else {
            let mut combined = render;
            if !combined.ends_with('\n') {
                combined.push('\n');
            }
            if !combined.ends_with("\n\n") {
                combined.push('\n');
            }
            combined.push_str(&extra_sections);
            combined
        }
    };

    // Authoring-time advisory: warn when the supplied body carries a
    // reserved-legacy section heading (`## Notes`) so the collision
    // surfaces now, not at commit time via the doctor pre-commit hook.
    // Non-fatal — the create still proceeds (the author may be
    // migrating). Scanned on the *raw* user body (`--description` /
    // `--body-file`, unified into `description` upstream), NOT the
    // rendered document: a `## Notes` can only originate from user
    // input, never from the renderer (which only injects `# <title>`
    // and canonical section stubs). Scanning the raw body avoids
    // splitting frontmatter back off `render`, which a Markdown
    // horizontal rule (`---`) or CRLF in the body would break.
    let mut warnings =
        crate::body_sections::reserved_section_warnings(args.description.as_deref().unwrap_or(""));

    let issues_parent = if args.inbox {
        root.join("issues").join(crate::repo::INBOX_DIR)
    } else {
        root.join("issues")
    };
    fs::create_dir_all(&issues_parent)
        .with_context(|| format!("cannot create {}", issues_parent.display()))
        .map_err(DoNewError::Io)?;

    // Pick a slug atomically: try `fs::create_dir` (which fails on
    // EEXIST) so two concurrent `issuectl new` invocations cannot race.
    // Post-flat-layout, the canonical home is `issues/<slug>/`.
    let (slug, dir, derived_base) = match &args.slug {
        Some(s) => {
            let normalized = write::slugify(s, 10);
            if !slug::is_valid(&normalized) {
                return Err(DoNewError::Validation(format!(
                    "--slug {:?} normalized to {:?}, which is not a valid slug \
                     (need ≥2 lowercase ASCII kebab segments, optional digits)",
                    s, normalized
                )));
            }
            // Detect a pre-existing legacy copy of the slug so the
            // error message points at the migration command.
            let (_flat, legacy_open, legacy_closed) = repo::paths_for(root, &normalized);
            if legacy_open.exists() || legacy_closed.exists() {
                return Err(DoNewError::Conflict(format!(
                    "slug {normalized} already used at legacy path; run `issuectl doctor --fix` first"
                )));
            }
            // Cross-bucket conflict: a slug must be unique across the
            // active flat root and the inbox drafts zone so `triage`
            // can move one into the other later without colliding.
            let flat_path = root.join("issues").join(&normalized);
            let inbox_path = root
                .join("issues")
                .join(crate::repo::INBOX_DIR)
                .join(&normalized);
            let other = if args.inbox { &flat_path } else { &inbox_path };
            if other.exists() {
                return Err(DoNewError::Conflict(format!(
                    "slug {normalized} already exists at {}; pick a different slug",
                    other.display()
                )));
            }
            let dir = issues_parent.join(&normalized);
            match fs::create_dir(&dir) {
                Ok(()) => (normalized, dir, None),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(DoNewError::Conflict(format!(
                        "slug {:?} already exists at {}; retry with a different --slug \
                         or omit --slug to get a random auto-generated one",
                        normalized,
                        dir.display()
                    )));
                }
                Err(e) => {
                    return Err(DoNewError::Io(
                        anyhow::Error::from(e).context(format!("cannot create {}", dir.display())),
                    ));
                }
            }
        }
        None => {
            // Default: derive a descriptive kebab slug from the title
            // (with its own numeric-suffix dedupe). The random
            // `intensifier-adjective-noun` form is reachable explicitly
            // via `--slug-random`, and is the automatic fallback when the
            // title yields no sensible slug (empty/all-stop-words/
            // non-ASCII) or the derived namespace is saturated. This path
            // does NOT route through the explicit-`--slug` conflict arm
            // above — a derived collision disambiguates silently rather
            // than erroring at the caller.
            let derived = if args.slug_random {
                None
            } else {
                slug::derive_from_title(&args.title)
            };
            match derived {
                Some(base) => {
                    match claim_derived_slug(root, &issues_parent, &base, args.inbox)
                        .map_err(DoNewError::Io)?
                    {
                        Some((slug, dir)) => (slug, dir, Some(base)),
                        None => {
                            let (slug, dir) =
                                claim_random_slug(root, &issues_parent).map_err(DoNewError::Io)?;
                            (slug, dir, None)
                        }
                    }
                }
                None => {
                    let (slug, dir) =
                        claim_random_slug(root, &issues_parent).map_err(DoNewError::Io)?;
                    (slug, dir, None)
                }
            }
        }
    };

    if let Some(base) = derived_base {
        if let Some(straightforward) = slug::straightforward_from_title(&args.title) {
            if base != straightforward {
                warnings.push(format!(
                    "derived base `{base}` differs from title slug `{straightforward}`: derived slugs retain 2–3 significant words after dropping stop-words"
                ));
            }
        }
        if slug != base {
            warnings.push(format!(
                "derived slug `{slug}` adds a numeric suffix because `{base}` already exists"
            ));
        }
    }

    let item_path = dir.join("item.md");
    // `create_new(true)` is belt-and-braces here: the directory is
    // already exclusively ours, but if a caller somehow seeds an
    // `item.md` between the rename and write, we fail loudly.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&item_path)
            .with_context(|| format!("cannot create {}", item_path.display()))
            .map_err(DoNewError::Io)?;
        f.write_all(render.as_bytes())
            .with_context(|| format!("cannot write {}", item_path.display()))
            .map_err(DoNewError::Io)?;
    }

    Ok(WriteOutcome {
        slug,
        title: args.title,
        item_path,
        warnings,
        lane: args.lane,
        lane_seq: args.lane_seq,
        collision: committed_collision,
    })
}

/// Upper bound on the numeric-suffix dedupe for a title-derived slug.
/// A base plus `-2`..=`-<cap>` gives 99 distinct homes for one title;
/// beyond that the title is too generic to keep disambiguating, and the
/// caller falls back to a random slug.
const DERIVED_SLUG_SUFFIX_CAP: usize = 99;

/// Atomically claim a flat directory for a title-derived `base` slug,
/// disambiguating collisions with a numeric suffix (`base`, `base-2`,
/// `base-3`, …). Returns `Ok(None)` when every candidate up to
/// [`DERIVED_SLUG_SUFFIX_CAP`] is taken, so the caller can fall back to a
/// random slug rather than fail the create.
///
/// Mirrors the conflict checks of the explicit-`--slug` arm — legacy
/// (pre-flat) paths and the cross-bucket (flat ↔ inbox) namespace — but
/// treats every conflict as "try the next suffix" instead of an error.
fn claim_derived_slug(
    root: &Path,
    issues_parent: &Path,
    base: &str,
    inbox: bool,
) -> Result<Option<(String, PathBuf)>> {
    for n in 1..=DERIVED_SLUG_SUFFIX_CAP {
        let candidate = if n == 1 {
            base.to_string()
        } else {
            format!("{base}-{n}")
        };
        // Valid by construction: `base` came from `slug::derive_from_title`
        // (which gates on `is_valid`), and appending `-<digits>` keeps every
        // segment a lowercase-ascii/digit kebab segment. Assert it so a
        // future change to the base derivation or suffix shape can't silently
        // claim a directory whose name isn't a resolvable slug.
        debug_assert!(
            slug::is_valid(&candidate),
            "derived candidate {candidate:?} must be a valid slug"
        );
        // Skip a slug already present at a legacy (pre-flat) path or in the
        // other bucket — a later `triage` must be able to move an inbox
        // draft to the flat root (or vice versa) without colliding.
        let (_flat, legacy_open, legacy_closed) = repo::paths_for(root, &candidate);
        if legacy_open.exists() || legacy_closed.exists() {
            continue;
        }
        let flat_path = root.join("issues").join(&candidate);
        let inbox_path = root
            .join("issues")
            .join(crate::repo::INBOX_DIR)
            .join(&candidate);
        let other = if inbox { &flat_path } else { &inbox_path };
        if other.exists() {
            continue;
        }
        let dir = issues_parent.join(&candidate);
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(Some((candidate, dir))),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(
                    anyhow::Error::from(e).context(format!("cannot create {}", dir.display()))
                )
            }
        }
    }
    Ok(None)
}

/// Generate a random slug and atomically claim its flat directory.
/// Loops on `EEXIST` so that two concurrent processes that happen to
/// pick the same slug both retry rather than silently overwriting.
fn claim_random_slug(root: &Path, issues_parent: &Path) -> Result<(String, PathBuf)> {
    for _ in 0..16 {
        let candidate = slug::generate();
        // Cheap pre-check: skip slugs that already exist at any path
        // (flat or legacy) to avoid burning a random pick when the
        // answer is obvious.
        let (_flat, legacy_open, legacy_closed) = repo::paths_for(root, &candidate);
        if legacy_open.exists() || legacy_closed.exists() {
            continue;
        }
        let dir = issues_parent.join(&candidate);
        match fs::create_dir(&dir) {
            Ok(()) => return Ok((candidate, dir)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(
                    anyhow::Error::from(e).context(format!("cannot create {}", dir.display()))
                )
            }
        }
    }
    bail!("could not claim a unique slug after 16 attempts; wordlist exhausted?")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slug;
    use std::fs;
    use tempfile::TempDir;

    fn fresh_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        tmp
    }

    fn new_args(t: &str, title: &str) -> NewArgs {
        NewArgs {
            issue_type: t.to_string(),
            title: title.to_string(),
            slug: None,
            slug_random: false,
            reporter: None,
            assignee: None,
            owner: None,
            priority: "normal".to_string(),
            epic: None,
            labels: vec![],
            related: vec![],
            source: None,
            description: None,
            custom_fields: vec![],
            lane: None,
            lane_seq: None,
            collision: vec![],
            status: None,
            inbox: false,
        }
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn new_creates_random_slug_directory() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "First bug");
        // Force the random path so this test actually exercises it — the
        // default would derive `first-bug` from the title.
        args.slug_random = true;
        args.reporter = Some("alice".into());
        args.assignee = Some("bob".into());
        let out = do_new(tmp.path(), args).unwrap();
        assert!(
            slug::is_valid(&out.slug),
            "{} should be valid slug",
            out.slug
        );
        assert!(out.item_path.exists());
        let content = read(&out.item_path);
        assert!(content.contains("type: bug"));
        assert!(content.contains("reporter: alice"));
        assert!(content.contains("assignee: bob"));
        assert!(content.contains("# First bug"));
    }

    #[test]
    fn new_derives_slug_from_title_by_default() {
        let tmp = fresh_repo();
        // No --slug: the default is now a title-derived kebab slug, not a
        // random intensifier-adjective-noun.
        let out = do_new(
            tmp.path(),
            new_args("bug", "Login redirect loops on safari"),
        )
        .unwrap();
        assert_eq!(out.slug, "login-redirect-loops");
        assert!(out
            .item_path
            .to_string_lossy()
            .contains("/login-redirect-loops/"));
    }

    #[test]
    fn derived_slug_warns_when_title_slug_is_shortened() {
        let tmp = fresh_repo();
        let out = do_new(tmp.path(), new_args("task", "vat-rs-module-split")).unwrap();
        assert_eq!(out.slug, "vat-rs-module");
        assert_eq!(out.warnings.len(), 1, "warnings={:?}", out.warnings);
        assert!(out.warnings[0].contains("vat-rs-module-split"));
        assert!(out.warnings[0].contains("2–3 significant words"));
    }

    #[test]
    fn derived_slug_collision_warns_about_numeric_disambiguation() {
        let tmp = fresh_repo();
        do_new(tmp.path(), new_args("task", "login redirect loops")).unwrap();
        let out = do_new(tmp.path(), new_args("task", "login redirect loops")).unwrap();
        assert_eq!(out.slug, "login-redirect-loops-2");
        assert_eq!(out.warnings.len(), 1, "warnings={:?}", out.warnings);
        assert!(out.warnings[0].contains("numeric suffix"));
        assert!(out.warnings[0].contains("already exists"));
        assert!(!out.warnings[0].contains("stop-words"));
    }

    #[test]
    fn new_lane_fields_are_set_and_lift_into_typed_issue() {
        // `new --lane`/`--lane-seq`/`--add-collision` write the
        // scheduling-DAG fields at creation, mirroring `update`, so the
        // issue is born into the DAG in one call. `lane_seq` must render
        // as an *unquoted* integer (numeric field), `collision` as a
        // string list, and all three must lift back into the typed
        // `Issue` fields via the parser.
        let tmp = fresh_repo();
        let mut args = new_args("feature", "Scheduled feature");
        args.slug = Some("sched-feat".into());
        args.lane = Some("cli-fixes".into());
        args.lane_seq = Some(40);
        // A duplicate collision token exercises the `add_to_string_list`
        // dedupe (same mechanics as `update --add-collision`): the repeat
        // must collapse, first-seen order preserved.
        args.collision = vec![
            "crates/issuectl/src/main.rs".into(),
            "foo/bar.rs".into(),
            "crates/issuectl/src/main.rs".into(),
        ];
        let out = do_new(tmp.path(), args).unwrap();

        let content = read(&out.item_path);
        assert!(content.contains("lane: cli-fixes"), "got: {content}");
        assert!(
            content.contains("lane_seq: 40") && !content.contains("lane_seq: '40'"),
            "lane_seq must be an unquoted integer; got: {content}"
        );

        let expected_collision = vec![
            "crates/issuectl/src/main.rs".to_string(),
            "foo/bar.rs".to_string(),
        ];
        // The outcome carries the committed (deduped) values captured under
        // the lock — the source of the `--json` echo, so assert them here
        // rather than re-reading disk in the CLI.
        assert_eq!(out.lane.as_deref(), Some("cli-fixes"));
        assert_eq!(out.lane_seq, Some(40));
        assert_eq!(out.collision, expected_collision);

        let parsed = crate::parser::parse_item_md_with_warnings(&out.item_path, &out.slug, "open");
        assert_eq!(parsed.issue.lane.as_deref(), Some("cli-fixes"));
        assert_eq!(parsed.issue.lane_seq, Some(40));
        assert_eq!(parsed.issue.collision, Some(expected_collision));
    }

    #[test]
    fn new_without_lane_omits_the_keys_and_hashes_identically() {
        // Load-bearing regression: an issue created WITHOUT any lane
        // field must be byte-for-byte the pre-field shape — no `lane:`,
        // `lane_seq:`, or `collision:` key — so it hashes identically to
        // an equivalent issue and never regresses the golden vector
        // (`canonical::no_lane_collision_hashes_identically` +
        // `no_lane_seq_hashes_identically` pin the hash side).
        let tmp = fresh_repo();
        let mut args = new_args("feature", "Plain feature");
        args.slug = Some("plain-feat".into());
        let out = do_new(tmp.path(), args).unwrap();

        let content = read(&out.item_path);
        assert!(!content.contains("lane:"), "unexpected lane key: {content}");
        assert!(
            !content.contains("lane_seq:"),
            "unexpected lane_seq key: {content}"
        );
        assert!(
            !content.contains("collision:"),
            "unexpected collision key: {content}"
        );

        let plain = crate::parser::parse_item_md_with_warnings(&out.item_path, &out.slug, "open");
        assert_eq!(plain.issue.lane, None);
        assert_eq!(plain.issue.lane_seq, None);
        assert_eq!(plain.issue.collision, None);

        // A second no-lane issue with the same authored fields hashes
        // identically — the lane fields being absent (not `Some(..)`)
        // keeps them out of `canonical_hash` entirely.
        let mut args2 = new_args("feature", "Plain feature");
        args2.slug = Some("plain-feat-2".into());
        let out2 = do_new(tmp.path(), args2).unwrap();
        let plain2 =
            crate::parser::parse_item_md_with_warnings(&out2.item_path, &out2.slug, "open");
        assert_eq!(
            crate::canonical::canonical_hash(&plain.issue),
            crate::canonical::canonical_hash(&plain2.issue),
            "two no-lane issues with the same authored fields must hash identically"
        );

        // Conversely, setting a lane MUST perturb the canonical hash — proof
        // the typed field is projected into the hash when `Some` (the other
        // half of the `only when Some` guarantee).
        let mut laned = new_args("feature", "Plain feature");
        laned.slug = Some("plain-feat-3".into());
        laned.lane = Some("cli-fixes".into());
        let out3 = do_new(tmp.path(), laned).unwrap();
        let laned_issue =
            crate::parser::parse_item_md_with_warnings(&out3.item_path, &out3.slug, "open");
        assert_ne!(
            crate::canonical::canonical_hash(&plain.issue),
            crate::canonical::canonical_hash(&laned_issue.issue),
            "setting a lane must change the canonical hash"
        );
    }

    #[test]
    fn new_lane_matches_new_then_update_lane_byte_for_byte() {
        // The feature's premise is parity with `update`: an issue born with
        // `new --lane X` must be indistinguishable on disk from one created
        // plain and then `update --lane X`-ed. Both project the same fields
        // through the same `write::` setters onto a mapping with the same
        // base key order, so the rendered item.md must be byte-identical —
        // guarding against key-ordering drift (fmt churn) between the paths.
        use crate::mutate::{update_issue, Patch, UpdateIssueRequest};

        let tmp = fresh_repo();
        let mut born = new_args("feature", "Same shape");
        born.slug = Some("born-laned".into());
        born.lane = Some("cli-fixes".into());
        born.lane_seq = Some(7);
        born.collision = vec!["a/b.rs".into()];
        let born_out = do_new(tmp.path(), born).unwrap();
        let born_text = read(&born_out.item_path);

        let mut plain = new_args("feature", "Same shape");
        plain.slug = Some("updated-laned".into());
        let plain_out = do_new(tmp.path(), plain).unwrap();
        update_issue(
            tmp.path(),
            "updated-laned",
            UpdateIssueRequest {
                lane: Patch::Set("cli-fixes".into()),
                lane_seq: Patch::Set(7),
                add_collision: vec!["a/b.rs".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let updated_text = read(&plain_out.item_path);

        // Normalise only the slug-derived `# <title>` line is identical
        // already (same title); the frontmatter must line up key-for-key.
        assert_eq!(
            born_text, updated_text,
            "new --lane must render identically to new + update --lane"
        );
    }

    #[test]
    fn new_rejects_empty_lane_and_empty_collision_token() {
        // Parity with `update`'s `check_set_nonempty` / `add_collision`
        // gate, which reject only the truly-empty string (`is_empty()`),
        // not whitespace — the CLI's `parse_non_empty` is what rejects
        // whitespace for both verbs.
        let tmp = fresh_repo();
        let mut args = new_args("feature", "Bad lane");
        args.slug = Some("bad-lane".into());
        args.lane = Some(String::new());
        let err = do_new(tmp.path(), args).unwrap_err();
        assert!(
            err.to_string()
                .contains("lane: empty-string Set is not allowed"),
            "got: {err}"
        );

        let mut args = new_args("feature", "Bad collision");
        args.slug = Some("bad-collision".into());
        args.collision = vec!["ok".into(), String::new()];
        let err = do_new(tmp.path(), args).unwrap_err();
        assert!(
            err.to_string().contains("collision contains an empty"),
            "got: {err}"
        );
    }

    #[test]
    fn new_derived_slug_collision_gets_numeric_suffix() {
        let tmp = fresh_repo();
        let first = do_new(tmp.path(), new_args("bug", "Fix login bug")).unwrap();
        assert_eq!(first.slug, "fix-login-bug");
        // Same title again → deterministic base collides → `-2` suffix.
        let second = do_new(tmp.path(), new_args("bug", "Fix login bug")).unwrap();
        assert_eq!(second.slug, "fix-login-bug-2");
        let third = do_new(tmp.path(), new_args("bug", "Fix login bug")).unwrap();
        assert_eq!(third.slug, "fix-login-bug-3");
    }

    #[test]
    fn new_falls_back_to_random_for_unsluggable_title() {
        let tmp = fresh_repo();
        // A title that derives no valid slug (non-ASCII) must still create
        // an issue — via the random fallback.
        let out = do_new(tmp.path(), new_args("bug", "Käyttäjän virhe")).unwrap();
        assert!(
            slug::is_valid(&out.slug),
            "{} should be a valid slug",
            out.slug
        );
        // Random form is three lowercase-letter segments; the derived path
        // would have produced digits or the title's words (neither here).
        assert_eq!(out.slug.split('-').count(), 3, "expected random slug shape");
    }

    #[test]
    fn new_slug_random_flag_forces_random_slug() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "Login redirect loops");
        args.slug_random = true;
        let out = do_new(tmp.path(), args).unwrap();
        assert!(slug::is_valid(&out.slug));
        // Explicitly NOT the derived slug.
        assert_ne!(out.slug, "login-redirect-loops");
        assert_eq!(out.slug.split('-').count(), 3, "expected random slug shape");
    }

    #[test]
    fn new_honors_explicit_slug_override() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "Some Long Title");
        args.slug = Some("custom-thing".into());
        let out = do_new(tmp.path(), args).unwrap();
        assert_eq!(out.slug, "custom-thing");
        assert!(out.item_path.to_string_lossy().contains("/custom-thing/"));
    }

    #[test]
    fn new_rejects_unsluggable_explicit_slug() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "Title");
        args.slug = Some("!!!".into());
        assert!(do_new(tmp.path(), args).is_err());
    }

    #[test]
    fn new_rejects_existing_slug() {
        let tmp = fresh_repo();
        // Use a valid ≥2-segment slug so the create actually reaches the
        // collision arm — a single-segment slug like "taken" would be
        // rejected by `is_valid` first and never test the conflict path.
        fs::create_dir_all(tmp.path().join("issues/already-taken")).unwrap();
        fs::write(
            tmp.path().join("issues/already-taken/item.md"),
            "---\nstatus: open\n---\n",
        )
        .unwrap();
        let mut args = new_args("bug", "Title");
        args.slug = Some("already-taken".into());
        let err = do_new(tmp.path(), args).unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "expected an already-exists conflict, got {err}"
        );
    }

    #[test]
    fn new_explicit_slug_wins_over_slug_random() {
        // The `slug_random` flag is ignored when an explicit `--slug` is
        // given — explicit always wins. This precedence is load-bearing:
        // `mutate::intake::file` sets both (slug = the filer's optional slug,
        // slug_random = true) so a supplied slug is honoured while unslugged
        // filings go random.
        let tmp = fresh_repo();
        let mut args = new_args("bug", "Some Title");
        args.slug = Some("custom-thing".into());
        args.slug_random = true;
        let out = do_new(tmp.path(), args).unwrap();
        assert_eq!(out.slug, "custom-thing");
    }

    #[test]
    fn new_rejects_epic_with_reporter() {
        let tmp = fresh_repo();
        let mut args = new_args("epic", "API v2");
        args.reporter = Some("alice".into());
        assert!(do_new(tmp.path(), args).is_err());
    }

    #[test]
    fn new_rejects_owner_for_non_epic() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "B");
        args.owner = Some("alice".into());
        assert!(do_new(tmp.path(), args).is_err());
    }

    #[test]
    fn new_creates_epic_with_owner() {
        let tmp = fresh_repo();
        let mut args = new_args("epic", "API v2 migration");
        args.owner = Some("cara".into());
        let out = do_new(tmp.path(), args).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("type: epic"));
        assert!(content.contains("owner: cara"));
    }

    #[test]
    fn new_normalizes_related_to_at_form() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "B");
        args.related = vec!["@extremely-quiet-otter".into(), "amber-loud-fox".into()];
        let out = do_new(tmp.path(), args).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("@extremely-quiet-otter"));
        assert!(content.contains("@amber-loud-fox"));
    }

    #[test]
    fn new_preserves_legacy_numeric_related() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "B");
        args.related = vec!["#7".into()];
        let out = do_new(tmp.path(), args).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("'#7'") || content.contains("\"#7\""));
    }

    #[test]
    fn new_rejects_when_custom_required_field_missing() {
        let tmp = fresh_repo();
        // Pre-write a schema demanding a `team` field. Without `--field`
        // creation must fail loudly rather than silently producing an
        // invalid issue.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let err = do_new(tmp.path(), new_args("bug", "Will fail")).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("schema") && msg.contains("team"),
            "expected schema/team in error, got {msg:?}"
        );
    }

    #[test]
    fn new_with_field_satisfies_custom_required_field() {
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n    enum: [payments, infra]\n",
        )
        .unwrap();
        let mut args = new_args("bug", "With team");
        args.custom_fields = vec![("team".into(), "payments".into())];
        let out = do_new(tmp.path(), args).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("team: payments"));
    }

    #[test]
    fn new_rejects_field_outside_schema_enum() {
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n    enum: [payments, infra]\n",
        )
        .unwrap();
        let mut args = new_args("bug", "Bad team");
        args.custom_fields = vec![("team".into(), "marketing".into())];
        let err = do_new(tmp.path(), args).err().unwrap();
        let msg = err.to_string();
        assert!(
            msg.contains("schema") && msg.contains("team") && msg.contains("marketing"),
            "expected schema/team/marketing in error, got {msg:?}"
        );
    }

    #[test]
    fn new_rejects_duplicate_field_keys() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "Dup");
        args.custom_fields = vec![("team".into(), "a".into()), ("team".into(), "b".into())];
        let err = do_new(tmp.path(), args).err().unwrap();
        let msg = err.to_string();
        assert!(
            msg.contains("team") && msg.contains("more than once"),
            "expected duplicate-rejection, got {msg:?}"
        );
    }

    #[test]
    fn do_new_error_to_anyhow_text_matches_per_variant() {
        // Lock the byte-identical CLI text contract: the From<DoNewError>
        // for anyhow::Error impl is what `cmd_new` relies on to keep
        // human-readable error messages stable across the typed-error
        // refactor. If a future contributor edits the variants without
        // touching the conversion, this test fails before users do.
        let cases: &[(DoNewError, &str)] = &[
            (
                DoNewError::Validation("--owner is only valid with --type epic".into()),
                "--owner is only valid with --type epic",
            ),
            (
                DoNewError::Conflict(
                    "slug \"foo\" already exists at /x; retry with a different --slug \
                     or omit --slug to get a random auto-generated one"
                        .into(),
                ),
                "slug \"foo\" already exists at /x; retry with a different --slug \
                 or omit --slug to get a random auto-generated one",
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
                DoNewError::TransitionConfig(s) => DoNewError::TransitionConfig(s.clone()),
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

    #[test]
    fn new_writes_default_schema_on_first_use() {
        let tmp = fresh_repo();
        assert!(!tmp.path().join("issues/.schema.yaml").exists());
        let args = new_args("bug", "First bug");
        do_new(tmp.path(), args).unwrap();
        assert!(
            tmp.path().join("issues/.schema.yaml").is_file(),
            "schema file should be auto-written on first new"
        );
    }
}
