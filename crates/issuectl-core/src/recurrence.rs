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

use crate::mutate::new_issue::{do_new_locked, NewArgs, WriteOutcome};
use crate::mutate::WriteLock;

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
    /// Cursor for the next `schedule run`: the last fire time
    /// `run` *successfully advanced past* on a non-dry run. The
    /// cron iterator is started with `schedule.after(last_fire)`
    /// so a failed materialization leaves its own fire time as
    /// the next-run target (the cursor only advances past fires
    /// that materialized cleanly).
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
        let ext_lower = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        if !matches!(ext_lower.as_deref(), Some("yaml") | Some("yml")) {
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
    /// Names of recurrences that hit the [`MAX_CATCHUP_PER_RUN`] cap
    /// on this invocation — surfaces silent truncation so a dormant
    /// definition catching up after months isn't a black box.
    pub capped: Vec<String>,
    /// Names of recurrences this run subscribed for the first time
    /// (recorded `last_fire = now` without materializing anything).
    /// Exposed so the CLI can explain why a brand-new definition
    /// produced "no occurrences due".
    pub subscribed: Vec<String>,
    /// Errors per recurrence that did not abort the whole run (e.g.
    /// an unparseable cron expression in one definition). Format:
    /// `(recurrence_name, message)`.
    pub errors: Vec<(String, String)>,
}

/// Compute the list of fire times in `(cursor, now]` for `schedule`.
/// Caps at [`MAX_CATCHUP_PER_RUN`]. The `cron` crate's `after`
/// iterator is exclusive on the lower bound, which matches the
/// "advance past last_fire" semantics we want. The boolean return
/// is `true` when the cap kicked in *and* there is at least one
/// more in-window fire pending — callers surface it to the user
/// rather than silently truncating.
pub fn fires_between(
    schedule: &Schedule,
    cursor: DateTime<Utc>,
    now: DateTime<Utc>,
) -> (Vec<DateTime<Utc>>, bool) {
    let mut out = Vec::new();
    let mut capped = false;
    for t in schedule.after(&cursor) {
        if t > now {
            break;
        }
        if out.len() >= MAX_CATCHUP_PER_RUN {
            // There is at least one more fire ≤ now beyond the cap.
            capped = true;
            break;
        }
        out.push(t);
    }
    (out, capped)
}

/// Run the schedule: walk every definition, materialize due
/// occurrences, persist the updated manifest. Designed to be safe to
/// invoke repeatedly — the manifest's `(recurrence, occurrence)`
/// dedup makes it idempotent at the materialization level.
///
/// `now` is injected so tests can pin the wall clock; production
/// callers pass `Utc::now()`.
pub fn run(root: &Path, now: DateTime<Utc>, dry_run: bool) -> Result<RunReport> {
    // Hold the repo-wide flock around the manifest read/process/write
    // window. Without this, two parallel `schedule run` invocations
    // (e.g. system cron + a user-triggered run) would both read the
    // same cursor, both materialize the same occurrences, and the
    // last manifest writer would overwrite the other's record.
    // `do_new_locked` (below) shares this same WriteLock so we don't
    // re-enter and deadlock per fire.
    let lock = WriteLock::acquire(root)?;
    let defs = load_definitions(root)?;
    // Propagate manifest parse errors. Silently treating a
    // corrupted manifest as empty would reset every cursor and
    // re-materialize history — a far worse failure mode than asking
    // the operator to look at the broken YAML.
    let mut manifest = load_manifest(root)?;
    let before = manifest.clone();
    let mut report = RunReport {
        dry_run,
        ..Default::default()
    };
    report.recurrences_evaluated = defs.len();

    for def in &defs {
        if let Err(e) = run_one(&lock, root, def, now, dry_run, &mut manifest, &mut report) {
            // One bad definition shouldn't black-hole the whole
            // schedule. Record and continue.
            report.errors.push((def.name.clone(), format!("{e:#}")));
        }
    }

    if !dry_run && manifest_changed(&before, &manifest) {
        save_manifest(root, &manifest)?;
    }
    Ok(report)
}

/// Cheap equality on the manifest's logical content. Used to skip
/// the atomic rename when nothing changed, so an idle `schedule run`
/// doesn't churn `git status` or the disk.
fn manifest_changed(a: &Manifest, b: &Manifest) -> bool {
    serde_yaml::to_string(a).ok() != serde_yaml::to_string(b).ok()
}

/// Wall-clock variant of [`run`] — convenience for the CLI so it
/// doesn't have to depend on `chrono` directly.
pub fn run_now(root: &Path, dry_run: bool) -> Result<RunReport> {
    run(root, Utc::now(), dry_run)
}

fn run_one(
    lock: &WriteLock,
    root: &Path,
    def: &RecurrenceDef,
    now: DateTime<Utc>,
    dry_run: bool,
    manifest: &mut Manifest,
    report: &mut RunReport,
) -> Result<()> {
    let schedule = def.parsed_schedule()?;
    // Mutate state in-place via the `entry()` API. The prior
    // remove-then-insert pattern lost the entire entry whenever this
    // function returned `Err` via `?` between the two operations —
    // meaning a single materialization error reset the cursor and
    // re-subscribed the definition on the next run.
    let state = manifest.recurrences.entry(def.name.clone()).or_default();

    let cursor = match state
        .last_fire
        .as_deref()
        .map(parse_fire_time)
        .transpose()?
    {
        Some(t) => t,
        None => {
            // First sight of this definition: subscribe at `now`, do
            // NOT retro-materialize. Skip the manifest mutation under
            // dry-run so a preview doesn't covertly persist the
            // subscription. See the module docs for the rationale.
            if !dry_run {
                state.last_fire = Some(format_fire_time(now));
            }
            report.subscribed.push(def.name.clone());
            return Ok(());
        }
    };

    let (fires, hit_cap) = fires_between(&schedule, cursor, now);
    if hit_cap {
        report.capped.push(def.name.clone());
    }
    let mut latest_success = cursor;
    for fire in fires {
        let occ_key = format_fire_time(fire);
        if state.occurrences.iter().any(|o| o.occurrence == occ_key) {
            // Already materialized in a prior run — safe to skip and
            // advance past it. Defensive: shouldn't normally happen
            // because the cron iterator starts strictly after the
            // cursor, but a hand-edited manifest or a mid-flight
            // schedule change can produce overlap.
            report.skipped_already_materialized += 1;
            latest_success = fire;
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
            latest_success = fire;
            continue;
        }

        match materialize(lock, root, def, &occ_key) {
            Ok(outcome) => {
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
                latest_success = fire;
            }
            Err(e) => {
                // Advance the cursor only past *successful* fires —
                // leaving the failed one as the next-run target.
                // Stop processing this def so we don't blast through
                // a hundred broken occurrences in one go.
                report
                    .errors
                    .push((def.name.clone(), format!("@{occ_key}: {e:#}")));
                break;
            }
        }
    }

    // Persist the new cursor, but only when we're not in dry-run
    // mode. Persisting a cursor advance from a preview run would
    // mask exactly the thing the user is trying to preview.
    if !dry_run {
        state.last_fire = Some(format_fire_time(latest_success));
    }
    Ok(())
}

fn materialize(
    lock: &WriteLock,
    root: &Path,
    def: &RecurrenceDef,
    occurrence_key: &str,
) -> Result<WriteOutcome> {
    let args = NewArgs {
        issue_type: def.file.issue_type.clone().unwrap_or_else(|| "task".into()),
        title: def.file.title.clone(),
        slug: None,
        // Recurring occurrences all share the template's title, so a
        // title-derived slug would collide every period and burn through
        // the numeric-suffix namespace. Keep the random slug per occurrence.
        slug_random: true,
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
        lane: None,
        lane_seq: None,
        collision: vec![],
        status: None,
        inbox: false,
    };
    // Use the under-lock entry point so we don't double-acquire the
    // repo flock (fs2 advisory flock is per-fd; re-acquire would
    // either succeed silently and break the invariant, or deadlock,
    // depending on the platform). The outer `run()` already holds
    // it for the full read-process-write window.
    do_new_locked(lock, root, args).map_err(Into::into)
}

fn parse_fire_time(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .with_context(|| format!("invalid ISO-8601 timestamp in manifest: {s:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let report = run(tmp.path(), now, false).unwrap();
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
        run(tmp.path(), t0, false).unwrap();
        // Three days later → expect three fires (26th, 27th, 28th).
        let t1 = Utc.with_ymd_and_hms(2026, 5, 28, 12, 0, 0).unwrap();
        let report = run(tmp.path(), t1, false).unwrap();
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
        run(tmp.path(), t0, false).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 5, 27, 12, 0, 0).unwrap();
        let first = run(tmp.path(), t1, false).unwrap();
        let again = run(tmp.path(), t1, false).unwrap();
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
        run(tmp.path(), t0, true).unwrap();
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
        run(tmp.path(), t0, false).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 5, 26, 12, 0, 0).unwrap();
        run(tmp.path(), t1, false).unwrap();
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
        run(tmp.path(), t0, false).unwrap();
        // ~5 months later — would be thousands of fires uncapped.
        let t1 = Utc.with_ymd_and_hms(2026, 5, 31, 0, 0, 0).unwrap();
        let report = run(tmp.path(), t1, false).unwrap();
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
        let report = run(tmp.path(), t0, false).unwrap();
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
        let (fires, capped) = fires_between(&schedule, cursor, now);
        assert!(!capped);
        assert_eq!(fires.len(), 2);
        assert_eq!(
            fires[0],
            Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 0).unwrap()
        );
        assert_eq!(fires[1], now);
    }

    #[test]
    fn catchup_cap_is_surfaced_via_capped_field() {
        let tmp = fresh_repo();
        write_def(tmp.path(), "hourly", "title: H\nschedule: 0 * * * *\n");
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        run(tmp.path(), t0, false).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 5, 31, 0, 0, 0).unwrap();
        let report = run(tmp.path(), t1, false).unwrap();
        // Cap was hit because thousands of hours fit in the window.
        assert_eq!(report.capped, vec!["hourly".to_string()]);
    }

    #[test]
    fn first_sight_surfaces_subscribed_in_report() {
        let tmp = fresh_repo();
        write_def(tmp.path(), "daily", "title: D\nschedule: 0 0 * * *\n");
        let t0 = Utc.with_ymd_and_hms(2026, 5, 25, 12, 0, 0).unwrap();
        let report = run(tmp.path(), t0, false).unwrap();
        assert_eq!(report.subscribed, vec!["daily".to_string()]);
        assert!(report.materialized.is_empty());
    }

    #[test]
    fn dry_run_preserves_subscription_state() {
        // Repro for the gemini review: a brand-new def under
        // `--dry-run` must NOT covertly persist `last_fire=now`,
        // because the very next non-dry run is supposed to subscribe.
        let tmp = fresh_repo();
        write_def(tmp.path(), "daily", "title: D\nschedule: 0 0 * * *\n");
        let t0 = Utc.with_ymd_and_hms(2026, 5, 25, 12, 0, 0).unwrap();
        let report = run(tmp.path(), t0, true).unwrap();
        assert_eq!(report.subscribed, vec!["daily".to_string()]);
        assert!(!manifest_path(tmp.path()).exists());
        // A subsequent non-dry run still treats it as first-sight.
        let report = run(tmp.path(), t0, false).unwrap();
        assert_eq!(report.subscribed, vec!["daily".to_string()]);
    }

    #[test]
    fn idle_run_does_not_rewrite_manifest() {
        // Skip the atomic rename when nothing about the manifest
        // changed: otherwise every cron tick churns `git status` and
        // the file's mtime.
        let tmp = fresh_repo();
        write_def(tmp.path(), "daily", "title: D\nschedule: 0 0 * * *\n");
        let t0 = Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap();
        run(tmp.path(), t0, false).unwrap();
        let mtime_before = fs::metadata(manifest_path(tmp.path()))
            .unwrap()
            .modified()
            .unwrap();
        // Sleep just enough that a *different* mtime would be
        // observable if the file were rewritten.
        std::thread::sleep(std::time::Duration::from_millis(50));
        // Now == cursor (no due fires) → no write.
        run(tmp.path(), t0, false).unwrap();
        let mtime_after = fs::metadata(manifest_path(tmp.path()))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "idle run should not rewrite manifest"
        );
    }

    #[test]
    fn corrupt_manifest_is_an_error_not_silent_reset() {
        // Repro for the deepseek review: `load_manifest` errors must
        // propagate so a corrupt manifest doesn't silently reset
        // every cursor and re-materialize history.
        let tmp = fresh_repo();
        write_def(tmp.path(), "daily", "title: D\nschedule: 0 0 * * *\n");
        fs::write(
            manifest_path(tmp.path()),
            "this: is: not valid: yaml: at all",
        )
        .unwrap();
        let t0 = Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap();
        assert!(run(tmp.path(), t0, false).is_err());
    }

    #[test]
    fn load_definitions_accepts_uppercase_yaml_extension() {
        let tmp = fresh_repo();
        let path = recurrences_dir(tmp.path()).join("Weekly.YAML");
        fs::write(path, "title: W\nschedule: 0 0 * * 1\n").unwrap();
        let defs = load_definitions(tmp.path()).unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "Weekly");
    }

    #[test]
    fn manifest_rejects_unknown_version() {
        let tmp = fresh_repo();
        fs::write(manifest_path(tmp.path()), "version: 99\nrecurrences: {}\n").unwrap();
        assert!(load_manifest(tmp.path()).is_err());
    }
}
