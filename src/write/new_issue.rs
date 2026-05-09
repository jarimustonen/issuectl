//! New-issue creation. Owns the typed input (`NewArgs`), output
//! (`NewOutcome`), and error (`DoNewError`) shapes shared by the CLI
//! `cmd_new` handler and the server-side `mutate::new_issue` boundary.
//!
//! Lives under `write/` because the on-disk write of `item.md` —
//! including frontmatter rendering and slug claim — is the operation
//! these symbols exist to perform; sibling helpers
//! (`build_new_frontmatter`, `render_new_item_from_fm`, `slugify`)
//! already live in `write::`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::{mutate, repo, schema, slug, write};

pub(crate) struct NewArgs {
    pub issue_type: String,
    pub title: String,
    pub slug: Option<String>,
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
}

pub(crate) struct NewOutcome {
    pub slug: String,
    pub title: String,
    pub item_path: PathBuf,
}

/// Typed error surfaced by `do_new_locked`. The mutate boundary maps
/// each variant to a `MutateError` (`Conflict` is renamed to
/// `ConflictingIntent`; the rest are 1:1) so the API picks the right
/// HTTP status without string-matching the formatted `anyhow::Error`.
#[derive(Debug)]
pub(crate) enum DoNewError {
    Validation(String),
    Conflict(String),
    SchemaViolation(String),
    SchemaConfig(String),
    Io(anyhow::Error),
}

impl From<DoNewError> for anyhow::Error {
    fn from(e: DoNewError) -> Self {
        match e {
            DoNewError::Io(e) => e,
            DoNewError::Validation(s)
            | DoNewError::Conflict(s)
            | DoNewError::SchemaConfig(s) => anyhow::Error::msg(s),
            DoNewError::SchemaViolation(s) => anyhow::Error::msg(format!("schema: {s}")),
        }
    }
}

pub(crate) fn do_new(root: &Path, args: NewArgs) -> Result<NewOutcome> {
    // M1 contract: every issuectl-mediated writer holds the repo
    // `flock`. Without this acquire, concurrent `issuectl new` from
    // the terminal would race against server-side mutations and
    // bypass the protocol's serialization guarantee.
    let lock = mutate::WriteLock::acquire(root)?;
    Ok(do_new_locked(&lock, root, args)?)
}

/// Body of `do_new` that assumes the caller holds the repo `WriteLock`.
/// Server-side `mutate::new_issue` uses this so it can hold the same
/// lock through the post-write parse + publish — without splitting the
/// sequence the synthetic `IssueUpserted` lands AFTER the lock is
/// released, inverting seq order against concurrent writers (C3).
pub(crate) fn do_new_locked(
    _lock: &mutate::WriteLock,
    root: &Path,
    args: NewArgs,
) -> std::result::Result<NewOutcome, DoNewError> {
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

    let related = crate::normalize_related_refs_pub(&args.related)
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
        custom_fields: &args.custom_fields,
    };
    // Build the frontmatter mapping and validate it BEFORE serializing.
    // Validating the in-memory Mapping avoids the round-trip through
    // string parsing that the previous version used (and that subtly
    // duplicated the fragile `find("\n---")` splitter logic).
    let frontmatter = write::build_new_frontmatter(&new_args);
    {
        let schema =
            schema::load(root).map_err(|e| DoNewError::SchemaConfig(format!("{e:#}")))?;
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
    let render = write::render_new_item_from_fm(&new_args, &frontmatter);

    let issues_parent = root.join("issues");
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
            let dir = issues_parent.join(&normalized);
            match fs::create_dir(&dir) {
                Ok(()) => (normalized, dir),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(DoNewError::Conflict(format!(
                        "target directory already exists: {}",
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
        None => claim_random_slug(root, &issues_parent).map_err(DoNewError::Io)?,
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

    Ok(NewOutcome {
        slug,
        title: args.title,
        item_path,
    })
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
        fs::create_dir_all(tmp.path().join("issues/taken")).unwrap();
        fs::write(
            tmp.path().join("issues/taken/item.md"),
            "---\nstatus: open\n---\n",
        )
        .unwrap();
        let mut args = new_args("bug", "Title");
        args.slug = Some("taken".into());
        assert!(do_new(tmp.path(), args).is_err());
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
        let res = do_new(tmp.path(), new_args("bug", "Will fail"));
        let err = res.err().expect("schema-required field missing should fail");
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
        args.custom_fields = vec![
            ("team".into(), "a".into()),
            ("team".into(), "b".into()),
        ];
        let err = do_new(tmp.path(), args).err().unwrap();
        let msg = err.to_string();
        assert!(
            msg.contains("team") && msg.contains("more than once"),
            "expected duplicate-rejection, got {msg:?}"
        );
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
