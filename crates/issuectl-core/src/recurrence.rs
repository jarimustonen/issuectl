//! Recurring / scheduled issues.
//!
//! A recurrence definition lives at
//! `.issuectl/recurrences/<name>.yaml` and describes a cron schedule
//! plus the template fields (title, labels, assignee, …) for the
//! issues it materializes. `issuectl schedule run` walks every
//! definition, asks the cron schedule for fire times since the last
//! `last_fire` cursor recorded in
//! `.issuectl/recurrences/.manifest.yaml`, and creates a new issue
//! file per due occurrence with `recurrence_of:` and `occurrence:`
//! custom frontmatter.
//!
//! Design pinning (from the brainstorm): **materialize a NEW issue
//! file per occurrence; never overwrite an active instance.** That
//! preserves git history per occurrence and means closing the
//! latest instance has no effect on the next one — the manifest is
//! the dedup key, not the on-disk file.
//!
//! Bootstrap semantics: a brand-new recurrence does NOT
//! retro-materialize occurrences from the past. The first
//! `schedule run` after a definition appears records `last_fire =
//! now`; the first real materialization happens at the next cron
//! tick. This avoids the surprise of installing a `0 0 * * *`
//! definition and immediately getting 365 issues.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};

use crate::mutate::new_issue::{do_new, NewArgs, WriteOutcome};
use crate::repo_config::ConfigSource;

/// Directory holding recurrence definitions (one YAML per recurrence).
pub const RECURRENCES_DIR: &str = ".issuectl/recurrences";

/// Manifest file (inside [`RECURRENCES_DIR`]) that records which
/// occurrences have already been materialized and the per-recurrence
/// `last_fire` cursor.
pub const MANIFEST_FILE: &str = ".manifest.yaml";

/// Schema version stamped into the manifest. Bump if the on-disk
/// shape changes; readers should refuse versions they don't know.
pub const MANIFEST_VERSION: u32 = 1;

/// Maximum number of fire times to materialize in a single
/// `schedule run` per recurrence. Bounds the blast radius when a
/// definition has been dormant for months and somebody enables it.
pub const MAX_CATCHUP_PER_RUN: usize = 50;

/// Frontmatter key on a materialized issue pointing back at the
/// recurrence that produced it (filename stem of the YAML def).
pub const RECURRENCE_OF_KEY: &str = "recurrence_of";

/// Frontmatter key carrying the cron fire time the issue was
/// materialized for. ISO-8601 UTC, second precision, no fractional
/// component (matches [`format_fire_time`]).
pub const OCCURRENCE_KEY: &str = "occurrence";

/// On-disk shape of `.issuectl/recurrences/<name>.yaml`. The file's
/// stem becomes [`RecurrenceDef::name`].
#[derive(Debug, Clone, Deserialize)]
pub struct RecurrenceDefFile {
    pub title: String,
    pub schedule: String,
    /// Optional human label, written into the materialized issue's
    /// `recurrence_of:` frontmatter. Defaults to the file stem when
    /// absent so the round-trip from manifest → file is unambiguous.
    #[serde(default)]
    pub template: Option<String>,
    /// `task` by default. Validated by schema at issue-write time.
    #[serde(default)]
    #[serde(rename = "type")]
    pub issue_type: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub reporter: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// In-memory view of a loaded definition: file contents plus the
/// stem (the canonical "name" of the recurrence).
#[derive(Debug, Clone)]
pub struct RecurrenceDef {
    pub name: String,
    pub file: RecurrenceDefFile,
}

impl RecurrenceDef {
    /// The string written into `recurrence_of:` on materialized
    /// issues. Falls back to the name when `template:` is omitted.
    pub fn template_label(&self) -> &str {
        self.file.template.as_deref().unwrap_or(&self.name)
    }

    /// Parse the cron schedule. Accepts standard 5-field cron
    /// (`min hour DoM mon DoW`) or the 6/7-field form that the
    /// `cron` crate uses natively (`sec` prepended).
    pub fn parsed_schedule(&self) -> Result<Schedule> {
        parse_cron(&self.file.schedule)
            .with_context(|| format!("recurrence {:?}: invalid schedule", self.name))
    }
}

/// Accept standard 5-field cron by prepending `0 ` for the seconds
/// field — the `cron` crate parses 6/7-field expressions. 6/7-field
/// inputs pass through unchanged.
pub fn parse_cron(expr: &str) -> Result<Schedule> {
    let trimmed = expr.trim();
    let field_count = trimmed.split_whitespace().count();
    let normalized = match field_count {
        5 => format!("0 {trimmed}"),
        6 | 7 => trimmed.to_string(),
        n => bail!("cron expression must have 5, 6, or 7 fields (got {n}): {trimmed:?}"),
    };
    Schedule::from_str(&normalized).map_err(|e| anyhow!("{e}"))
}

/// Manifest entry for a single materialized occurrence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestOccurrence {
    pub occurrence: String,
    pub slug: String,
    pub materialized: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestRecurrence {
    /// Last fire time the run loop has *evaluated* — not necessarily
    /// the time of the most recent occurrence in `occurrences` (we
    /// advance this even on dry runs to avoid re-evaluation churn).
    #[serde(default)]
    pub last_fire: Option<String>,
    #[serde(default)]
    pub occurrences: Vec<ManifestOccurrence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    #[serde(default)]
    pub recurrences: BTreeMap<String, ManifestRecurrence>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            recurrences: BTreeMap::new(),
        }
    }
}

/// Absolute path to the recurrences directory under `root`.
pub fn recurrences_dir(root: &Path) -> PathBuf {
    root.join(RECURRENCES_DIR)
}

/// Absolute path to the manifest file.
pub fn manifest_path(root: &Path) -> PathBuf {
    recurrences_dir(root).join(MANIFEST_FILE)
}

/// Load every `*.yaml` (and `*.yml`) file under
/// `.issuectl/recurrences/` (excluding the manifest, which is dot-
/// prefixed). Returns an empty list when the directory is missing —
/// `schedule run` on a repo without recurrences is a no-op, not an
/// error.
pub fn load_definitions(root: &Path) -> Result<Vec<RecurrenceDef>> {
    let dir = recurrences_dir(root);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries = fs::read_dir(&dir).with_context(|| format!("cannot read {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("cannot read entry under {}", dir.display()))?;
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };
        // Skip the manifest and any other dotfiles to keep
        // operational state separate from user-authored defs.
        if name.starts_with('.') {
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str());
        if !matches!(ext, Some("yaml") | Some("yml")) {
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("recurrence file has no usable stem: {}", path.display()))?
            .to_string();
        let bytes = fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
        let file: RecurrenceDefFile = serde_yaml::from_slice(&bytes)
            .with_context(|| format!("cannot parse {}", path.display()))?;
        out.push(RecurrenceDef { name: stem, file });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Read the manifest from disk. Missing file returns the default
/// empty manifest so first-run is indistinguishable from "never
/// materialized anything".
pub fn load_manifest(root: &Path) -> Result<Manifest> {
    let path = manifest_path(root);
    if !path.exists() {
        return Ok(Manifest::default());
    }
    let bytes = fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let m: Manifest = serde_yaml::from_slice(&bytes)
        .with_context(|| format!("cannot parse {}", path.display()))?;
    if m.version != MANIFEST_VERSION {
        bail!(
            "{}: unsupported manifest version {} (this build supports {MANIFEST_VERSION})",
            path.display(),
            m.version
        );
    }
    Ok(m)
}

/// Write the manifest atomically (tmp file + rename). The directory
/// is created if missing so a brand-new repo doesn't need to scaffold
/// it manually.
pub fn save_manifest(root: &Path, manifest: &Manifest) -> Result<()> {
    let dir = recurrences_dir(root);
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let path = manifest_path(root);
    let tmp = path.with_extension("yaml.tmp");
    let yaml = serde_yaml::to_string(manifest).context("cannot serialize manifest")?;
    fs::write(&tmp, yaml).with_context(|| format!("cannot write {}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .with_context(|| format!("cannot rename {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Canonical string form of a fire time. Second-precision ISO-8601
/// UTC, e.g. `2026-05-25T00:00:00Z`. Used as both the `occurrence:`
/// frontmatter value and the manifest dedup key — keep these in
/// lockstep so the manifest lookup matches what's written to disk.
pub fn format_fire_time(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Result of one materialization (what got written for a single fire
/// time). Aggregated into [`RunReport`].
#[derive(Debug, Clone, Serialize)]
pub struct Materialized {
    pub recurrence: String,
    pub occurrence: String,
    pub slug: String,
    pub title: String,
    pub path: PathBuf,
}

/// Aggregate result of one `schedule run`. `dry_run` lets callers
/// preview what *would* be materialized without touching the
/// manifest or creating issues.
#[derive(Debug, Clone, Serialize, Default)]
pub struct RunReport {
    pub materialized: Vec<Materialized>,
    pub skipped_already_materialized: usize,
    pub recurrences_evaluated: usize,
    pub dry_run: bool,
    /// Errors per recurrence that did not abort the whole run (e.g.
    /// an unparseable cron expression in one definition). Format:
    /// `(recurrence_name, message)`.
    pub errors: Vec<(String, String)>,
}

/// Compute the list of fire times in `(cursor, now]` for `schedule`.
/// Caps at [`MAX_CATCHUP_PER_RUN`]. The `cron` crate's `after`
/// iterator is exclusive on the lower bound, which matches the
/// "advance past last_fire" semantics we want.
pub fn fires_between(
    schedule: &Schedule,
    cursor: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Vec<DateTime<Utc>> {
    let mut out = Vec::new();
    for t in schedule.after(&cursor) {
        if t > now {
            break;
        }
        out.push(t);
        if out.len() >= MAX_CATCHUP_PER_RUN {
            break;
        }
    }
    out
}

/// Run the schedule: walk every definition, materialize due
/// occurrences, persist the updated manifest. Designed to be safe to
/// invoke repeatedly — the manifest's `(recurrence, occurrence)`
/// dedup makes it idempotent at the materialization level.
///
/// `now` is injected so tests can pin the wall clock; production
/// callers pass `Utc::now()`.
pub fn run(
    root: &Path,
    config: &dyn ConfigSource,
    now: DateTime<Utc>,
    dry_run: bool,
) -> Result<RunReport> {
    let defs = load_definitions(root)?;
    let mut manifest = load_manifest(root).unwrap_or_default();
    let mut report = RunReport {
        dry_run,
        ..Default::default()
    };
    report.recurrences_evaluated = defs.len();

    for def in &defs {
        if let Err(e) = run_one(root, config, &def, now, dry_run, &mut manifest, &mut report) {
            // One bad definition shouldn't black-hole the whole
            // schedule. Record and continue.
            report.errors.push((def.name.clone(), format!("{e:#}")));
        }
    }

    if !dry_run {
        save_manifest(root, &manifest)?;
    }
    Ok(report)
}

/// Wall-clock variant of [`run`] — convenience for the CLI so it
/// doesn't have to depend on `chrono` directly.
pub fn run_now(root: &Path, config: &dyn ConfigSource, dry_run: bool) -> Result<RunReport> {
    run(root, config, Utc::now(), dry_run)
}

fn run_one(
    root: &Path,
    config: &dyn ConfigSource,
    def: &RecurrenceDef,
    now: DateTime<Utc>,
    dry_run: bool,
    manifest: &mut Manifest,
    report: &mut RunReport,
) -> Result<()> {
    let schedule = def.parsed_schedule()?;
    // Pull the per-recurrence state once; we mutate the entry by
    // value and write it back at the end so a mid-loop error
    // doesn't leave a half-updated entry behind.
    let mut state = manifest.recurrences.remove(&def.name).unwrap_or_default();

    let cursor = match state
        .last_fire
        .as_deref()
        .map(parse_fire_time)
        .transpose()?
    {
        Some(t) => t,
        None => {
            // First sight of this definition: subscribe at `now`,
            // do NOT retro-materialize. See the module docs.
            state.last_fire = Some(format_fire_time(now));
            manifest.recurrences.insert(def.name.clone(), state);
            return Ok(());
        }
    };

    let fires = fires_between(&schedule, cursor, now);
    let mut latest_seen = cursor;
    for fire in fires {
        latest_seen = fire;
        let occ_key = format_fire_time(fire);
        if state.occurrences.iter().any(|o| o.occurrence == occ_key) {
            // Already materialized — `last_fire` will still advance
            // past it via `latest_seen` so we don't keep finding it.
            report.skipped_already_materialized += 1;
            continue;
        }

        if dry_run {
            report.materialized.push(Materialized {
                recurrence: def.name.clone(),
                occurrence: occ_key,
                slug: String::new(),
                title: def.file.title.clone(),
                path: PathBuf::new(),
            });
            continue;
        }

        let outcome = materialize(root, config, def, &occ_key)
            .with_context(|| format!("materializing {} @ {occ_key}", def.name))?;
        state.occurrences.push(ManifestOccurrence {
            occurrence: occ_key.clone(),
            slug: outcome.slug.clone(),
            materialized: format_fire_time(now),
        });
        report.materialized.push(Materialized {
            recurrence: def.name.clone(),
            occurrence: occ_key,
            slug: outcome.slug,
            title: outcome.title,
            path: outcome.item_path,
        });
    }

    state.last_fire = Some(format_fire_time(latest_seen));
    manifest.recurrences.insert(def.name.clone(), state);
    Ok(())
}

fn materialize(
    root: &Path,
    config: &dyn ConfigSource,
    def: &RecurrenceDef,
    occurrence_key: &str,
) -> Result<WriteOutcome> {
    let args = NewArgs {
        issue_type: def.file.issue_type.clone().unwrap_or_else(|| "task".into()),
        title: def.file.title.clone(),
        slug: None,
        reporter: def.file.reporter.clone(),
        assignee: def.file.assignee.clone(),
        owner: None,
        priority: def.file.priority.clone().unwrap_or_else(|| "normal".into()),
        epic: None,
        labels: def.file.labels.clone(),
        related: vec![],
        source: None,
        description: def.file.description.clone(),
        custom_fields: vec![
            (
                RECURRENCE_OF_KEY.to_string(),
                def.template_label().to_string(),
            ),
            (OCCURRENCE_KEY.to_string(), occurrence_key.to_string()),
        ],
        inbox: false,
    };
    do_new(root, args, config)
}

fn parse_fire_time(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .with_context(|| format!("invalid ISO-8601 timestamp in manifest: {s:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_config::UncachedConfig;
    use chrono::TimeZone;
    use std::fs;
    use tempfile::TempDir;

    fn fresh_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        fs::create_dir_all(tmp.path().join(RECURRENCES_DIR)).unwrap();
        tmp
    }

    fn write_def(root: &Path, name: &str, body: &str) {
        let path = recurrences_dir(root).join(format!("{name}.yaml"));
        fs::write(path, body).unwrap();
    }

    #[test]
    fn parse_cron_accepts_5_and_6_field() {
        parse_cron("0 0 * * 1").unwrap();
        parse_cron("0 0 0 * * 1").unwrap();
    }

    #[test]
    fn parse_cron_rejects_garbage() {
        assert!(parse_cron("not a cron expr").is_err());
        assert!(parse_cron("0 0 0").is_err());
    }

    #[test]
    fn format_fire_time_is_seconds_zulu() {
        let t = Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap();
        assert_eq!(format_fire_time(t), "2026-05-25T00:00:00Z");
    }

    #[test]
    fn load_definitions_returns_empty_on_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let defs = load_definitions(tmp.path()).unwrap();
        assert!(defs.is_empty());
    }

    #[test]
    fn load_definitions_skips_manifest_and_non_yaml() {
        let tmp = fresh_repo();
        write_def(
            tmp.path(),
            "weekly",
            "title: Weekly review\nschedule: 0 0 * * 1\n",
        );
        // Manifest dotfile and a stray README should both be ignored.
        fs::write(
            recurrences_dir(tmp.path()).join(".manifest.yaml"),
            "version: 1\nrecurrences: {}\n",
        )
        .unwrap();
        fs::write(recurrences_dir(tmp.path()).join("README.md"), "hi").unwrap();
        let defs = load_definitions(tmp.path()).unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "weekly");
        assert_eq!(defs[0].file.title, "Weekly review");
    }

    #[test]
    fn first_run_subscribes_without_materializing() {
        let tmp = fresh_repo();
        write_def(
            tmp.path(),
            "daily",
            "title: Daily standup\nschedule: 0 0 * * *\n",
        );
        let now = Utc.with_ymd_and_hms(2026, 5, 25, 12, 0, 0).unwrap();
        let report = run(tmp.path(), &UncachedConfig, now, false).unwrap();
        assert!(
            report.materialized.is_empty(),
            "first run should not materialize"
        );
        assert_eq!(report.recurrences_evaluated, 1);
        let manifest = load_manifest(tmp.path()).unwrap();
        let state = manifest.recurrences.get("daily").unwrap();
        assert_eq!(state.last_fire.as_deref(), Some("2026-05-25T12:00:00Z"));
        assert!(state.occurrences.is_empty());
    }

    #[test]
    fn second_run_materializes_one_per_due_fire() {
        let tmp = fresh_repo();
        write_def(
            tmp.path(),
            "daily",
            "title: Daily standup\nschedule: 0 0 * * *\n",
        );
        // First run subscribes at 2026-05-25T00:00:00Z (just before
        // the first fire of the day to keep arithmetic simple).
        let t0 = Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap();
        run(tmp.path(), &UncachedConfig, t0, false).unwrap();
        // Three days later → expect three fires (26th, 27th, 28th).
        let t1 = Utc.with_ymd_and_hms(2026, 5, 28, 12, 0, 0).unwrap();
        let report = run(tmp.path(), &UncachedConfig, t1, false).unwrap();
        assert_eq!(report.materialized.len(), 3);
        assert_eq!(report.materialized[0].occurrence, "2026-05-26T00:00:00Z");
        // Each materialized issue is a real file with the
        // recurrence_of / occurrence frontmatter and a fresh slug.
        for m in &report.materialized {
            assert!(m.path.exists());
            let content = fs::read_to_string(&m.path).unwrap();
            assert!(content.contains("recurrence_of: daily"));
            assert!(
                content.contains(&format!("occurrence: '{}'", m.occurrence))
                    || content.contains(&format!("occurrence: \"{}\"", m.occurrence))
                    || content.contains(&format!("occurrence: {}", m.occurrence))
            );
        }
    }

    #[test]
    fn run_is_idempotent_via_manifest() {
        let tmp = fresh_repo();
        write_def(tmp.path(), "daily", "title: Daily\nschedule: 0 0 * * *\n");
        let t0 = Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap();
        run(tmp.path(), &UncachedConfig, t0, false).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 5, 27, 12, 0, 0).unwrap();
        let first = run(tmp.path(), &UncachedConfig, t1, false).unwrap();
        let again = run(tmp.path(), &UncachedConfig, t1, false).unwrap();
        assert_eq!(first.materialized.len(), 2);
        assert_eq!(again.materialized.len(), 0);
    }

    #[test]
    fn dry_run_does_not_write_manifest_or_files() {
        let tmp = fresh_repo();
        write_def(tmp.path(), "daily", "title: D\nschedule: 0 0 * * *\n");
        let t0 = Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap();
        // Even the first-run subscribe step must be a no-op under
        // dry-run, so a subsequent real run still has cursor=None
        // and behaves like a true first run.
        run(tmp.path(), &UncachedConfig, t0, true).unwrap();
        assert!(!manifest_path(tmp.path()).exists());
        let issues_dir = tmp.path().join("issues");
        let count_after_dry = fs::read_dir(&issues_dir).unwrap().count();
        assert_eq!(count_after_dry, 0, "dry run must not create issue dirs");
    }

    #[test]
    fn manifest_persists_slug_and_dedups_after_restart() {
        let tmp = fresh_repo();
        write_def(tmp.path(), "daily", "title: D\nschedule: 0 0 * * *\n");
        let t0 = Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap();
        run(tmp.path(), &UncachedConfig, t0, false).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 5, 26, 12, 0, 0).unwrap();
        run(tmp.path(), &UncachedConfig, t1, false).unwrap();
        // Reload manifest from disk — simulates process restart.
        let manifest = load_manifest(tmp.path()).unwrap();
        let state = manifest.recurrences.get("daily").unwrap();
        assert_eq!(state.occurrences.len(), 1);
        assert!(!state.occurrences[0].slug.is_empty());
    }

    #[test]
    fn catchup_caps_at_max_per_run() {
        let tmp = fresh_repo();
        // Hourly cron: 60+ fires over many days will exceed the cap.
        write_def(tmp.path(), "hourly", "title: H\nschedule: 0 * * * *\n");
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        run(tmp.path(), &UncachedConfig, t0, false).unwrap();
        // ~5 months later — would be thousands of fires uncapped.
        let t1 = Utc.with_ymd_and_hms(2026, 5, 31, 0, 0, 0).unwrap();
        let report = run(tmp.path(), &UncachedConfig, t1, false).unwrap();
        assert_eq!(report.materialized.len(), MAX_CATCHUP_PER_RUN);
    }

    #[test]
    fn template_label_falls_back_to_name() {
        let def = RecurrenceDef {
            name: "weekly".into(),
            file: RecurrenceDefFile {
                title: "x".into(),
                schedule: "0 0 * * 1".into(),
                template: None,
                issue_type: None,
                priority: None,
                labels: vec![],
                assignee: None,
                reporter: None,
                description: None,
            },
        };
        assert_eq!(def.template_label(), "weekly");
    }

    #[test]
    fn errors_in_one_def_dont_abort_others() {
        let tmp = fresh_repo();
        write_def(tmp.path(), "good", "title: G\nschedule: 0 0 * * *\n");
        write_def(tmp.path(), "bad", "title: B\nschedule: not-a-cron\n");
        let t0 = Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap();
        let report = run(tmp.path(), &UncachedConfig, t0, false).unwrap();
        assert_eq!(report.recurrences_evaluated, 2);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].0, "bad");
        // The good def's first-sight subscription still happened.
        let m = load_manifest(tmp.path()).unwrap();
        assert!(m.recurrences.contains_key("good"));
        assert!(!m.recurrences.contains_key("bad"));
    }

    #[test]
    fn fires_between_excludes_cursor_includes_now() {
        let schedule = parse_cron("0 0 * * *").unwrap();
        let cursor = Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap();
        // Exact `now` matches a fire — include it (`<=` semantics).
        let now = Utc.with_ymd_and_hms(2026, 5, 27, 0, 0, 0).unwrap();
        let fires = fires_between(&schedule, cursor, now);
        assert_eq!(fires.len(), 2);
        assert_eq!(
            fires[0],
            Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 0).unwrap()
        );
        assert_eq!(fires[1], now);
    }

    #[test]
    fn manifest_rejects_unknown_version() {
        let tmp = fresh_repo();
        fs::write(manifest_path(tmp.path()), "version: 99\nrecurrences: {}\n").unwrap();
        assert!(load_manifest(tmp.path()).is_err());
    }
}
