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
            status: None,
            inbox: false,
        }
    }
}

pub struct WriteOutcome {
    pub slug: String,
    pub title: String,
    pub item_path: PathBuf,
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

pub fn do_new(
    root: &Path,
    args: NewArgs,
    config: &dyn crate::repo_config::ConfigSource,
) -> Result<WriteOutcome> {
    // M1 contract: every issuectl-mediated writer holds the repo
    // `flock`. Without this acquire, concurrent `issuectl new` from
    // the terminal would race against server-side mutations and
    // bypass the protocol's serialization guarantee.
    let lock = WriteLock::acquire(root)?;
    Ok(do_new_locked(&lock, root, args, config)?)
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
    config: &dyn crate::repo_config::ConfigSource,
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
    let frontmatter = write::build_new_frontmatter(&new_args);
    let schema = config
        .schema(root)
        .map_err(|e| DoNewError::SchemaConfig(format!("{e:#}")))?;
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
    let (slug, dir) = match &args.slug {
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
                Ok(()) => (normalized, dir),
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
                        Some(claimed) => claimed,
                        None => claim_random_slug(root, &issues_parent).map_err(DoNewError::Io)?,
                    }
                }
                None => claim_random_slug(root, &issues_parent).map_err(DoNewError::Io)?,
            }
        }
    };

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
    use crate::repo_config::UncachedConfig;
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
        args.reporter = Some("alice".into());
        args.assignee = Some("bob".into());
        let out = do_new(tmp.path(), args, &UncachedConfig).unwrap();
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
            &UncachedConfig,
        )
        .unwrap();
        assert_eq!(out.slug, "login-redirect-loops");
        assert!(out
            .item_path
            .to_string_lossy()
            .contains("/login-redirect-loops/"));
    }

    #[test]
    fn new_derived_slug_collision_gets_numeric_suffix() {
        let tmp = fresh_repo();
        let first = do_new(
            tmp.path(),
            new_args("bug", "Fix login bug"),
            &UncachedConfig,
        )
        .unwrap();
        assert_eq!(first.slug, "fix-login-bug");
        // Same title again → deterministic base collides → `-2` suffix.
        let second = do_new(
            tmp.path(),
            new_args("bug", "Fix login bug"),
            &UncachedConfig,
        )
        .unwrap();
        assert_eq!(second.slug, "fix-login-bug-2");
        let third = do_new(
            tmp.path(),
            new_args("bug", "Fix login bug"),
            &UncachedConfig,
        )
        .unwrap();
        assert_eq!(third.slug, "fix-login-bug-3");
    }

    #[test]
    fn new_falls_back_to_random_for_unsluggable_title() {
        let tmp = fresh_repo();
        // A title that derives no valid slug (non-ASCII) must still create
        // an issue — via the random fallback.
        let out = do_new(
            tmp.path(),
            new_args("bug", "Käyttäjän virhe"),
            &UncachedConfig,
        )
        .unwrap();
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
        let out = do_new(tmp.path(), args, &UncachedConfig).unwrap();
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
        let out = do_new(tmp.path(), args, &UncachedConfig).unwrap();
        assert_eq!(out.slug, "custom-thing");
        assert!(out.item_path.to_string_lossy().contains("/custom-thing/"));
    }

    #[test]
    fn new_rejects_unsluggable_explicit_slug() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "Title");
        args.slug = Some("!!!".into());
        assert!(do_new(tmp.path(), args, &UncachedConfig).is_err());
    }

    #[test]
    fn new_rejects_existing_slug() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/taken")).unwrap();
        fs::write(
            tmp.path().join("issues/taken/item.md"),
            "---\nstatus: open\n---\n",
        )
        .unwrap();
        let mut args = new_args("bug", "Title");
        args.slug = Some("taken".into());
        assert!(do_new(tmp.path(), args, &UncachedConfig).is_err());
    }

    #[test]
    fn new_rejects_epic_with_reporter() {
        let tmp = fresh_repo();
        let mut args = new_args("epic", "API v2");
        args.reporter = Some("alice".into());
        assert!(do_new(tmp.path(), args, &UncachedConfig).is_err());
    }

    #[test]
    fn new_rejects_owner_for_non_epic() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "B");
        args.owner = Some("alice".into());
        assert!(do_new(tmp.path(), args, &UncachedConfig).is_err());
    }

    #[test]
    fn new_creates_epic_with_owner() {
        let tmp = fresh_repo();
        let mut args = new_args("epic", "API v2 migration");
        args.owner = Some("cara".into());
        let out = do_new(tmp.path(), args, &UncachedConfig).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("type: epic"));
        assert!(content.contains("owner: cara"));
    }

    #[test]
    fn new_normalizes_related_to_at_form() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "B");
        args.related = vec!["@extremely-quiet-otter".into(), "amber-loud-fox".into()];
        let out = do_new(tmp.path(), args, &UncachedConfig).unwrap();
        let content = read(&out.item_path);
        assert!(content.contains("@extremely-quiet-otter"));
        assert!(content.contains("@amber-loud-fox"));
    }

    #[test]
    fn new_preserves_legacy_numeric_related() {
        let tmp = fresh_repo();
        let mut args = new_args("bug", "B");
        args.related = vec!["#7".into()];
        let out = do_new(tmp.path(), args, &UncachedConfig).unwrap();
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
        let res = do_new(tmp.path(), new_args("bug", "Will fail"), &UncachedConfig);
        let err = res
            .err()
            .expect("schema-required field missing should fail");
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
        let out = do_new(tmp.path(), args, &UncachedConfig).unwrap();
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
        let err = do_new(tmp.path(), args, &UncachedConfig).err().unwrap();
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
        let err = do_new(tmp.path(), args, &UncachedConfig).err().unwrap();
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
        do_new(tmp.path(), args, &UncachedConfig).unwrap();
        assert!(
            tmp.path().join("issues/.schema.yaml").is_file(),
            "schema file should be auto-written on first new"
        );
    }
}
