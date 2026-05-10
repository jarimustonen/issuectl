//! Custom git merge driver for `issues/<slug>/item.md`.
//!
//! Wired via `.gitattributes` (`issues/**/item.md merge=issuectl-yaml`)
//! and `git config merge.issuectl-yaml.driver "issuectl merge-driver
//! --base %O --ours %A --theirs %B --output %A"`. Print the snippets
//! with `issuectl install-merge-driver`.
//!
//! Semantics:
//! - **frontmatter** is parsed as YAML on all three sides and merged
//!   field by field;
//!   - array fields `labels`/`related`/`blocked_by` use the standard
//!     "ours ∪ theirs minus base-side deletions" rule so adds from
//!     either branch survive and deletes from either branch take
//!     effect;
//!   - `commits` is unioned by `hash` (ours wins on summary collision),
//!     preserving first-appearance order. Deletions are NOT honoured
//!     (commits are append-only by current CLI contract — if you need
//!     to drop a commit, edit it on every branch);
//!   - `updated:` picks the lexicographically newer date *of ours and
//!     theirs* (base is ignored — both branches deliberately set it,
//!     so resurrecting a stale base value would be wrong);
//!   - other scalars: 3-way merge with `(base, ours, theirs)` triple
//!     so a one-sided add against an absent base is kept (not a
//!     conflict); only diverging changes against the same base produce
//!     a conflict;
//!   - on a frontmatter conflict, the driver emits **real** `<<<<<<<`
//!     conflict markers around the offending fields. The result is
//!     intentionally invalid YAML — that's the point: every other tool
//!     (parser, IDE merge editor, git mergetool) recognises the
//!     conflict and refuses to silently accept the file.
//! - **body** falls back to `git merge-file --stdout` on body-only
//!   temp files so frontmatter differences don't pollute the body
//!   merge. Conflict-marker labels are passed via `-L ours -L base -L
//!   theirs` (instead of leaking temp paths).
//! - On any unresolved conflict, exit 1; git surfaces the standard
//!   merge UI. The output file MUST contain real conflict markers (in
//!   the body, the frontmatter, or both) — the driver never produces
//!   a parseable "merged" file with hidden conflicts.
//!
//! Coordination with `issuectl serve`: the driver acquires the repo
//! `WriteLock` before writing the output and uses `write_item_atomic`,
//! so a concurrent web mutation cannot race the merge result. The
//! lock is held briefly per `serve` PATCH, so this never deadlocks.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use serde_yaml::{Mapping, Value};

use crate::fmt::format_text;
use crate::item_text;
use crate::mutate::WriteLock;
use crate::write::{self, ItemFile};

#[derive(Debug)]
pub struct MergeArgs {
    pub base: PathBuf,
    pub ours: PathBuf,
    pub theirs: PathBuf,
    pub output: PathBuf,
}

/// Run the merge driver. Returns Ok(0) on clean merge, Ok(1) when a
/// conflict was emitted (frontmatter scalar diverge or body text
/// conflict). Caller (main.rs) wires the i32 to `process::exit`.
pub fn run(args: &MergeArgs) -> Result<i32> {
    // Read all three sides up front so output==ours overwrite is safe.
    let base_raw = std::fs::read_to_string(&args.base)
        .with_context(|| format!("cannot read base {}", args.base.display()))?;
    let ours_raw = std::fs::read_to_string(&args.ours)
        .with_context(|| format!("cannot read ours {}", args.ours.display()))?;
    let theirs_raw = std::fs::read_to_string(&args.theirs)
        .with_context(|| format!("cannot read theirs {}", args.theirs.display()))?;

    let (base_fm, base_body) = parse_sides(&base_raw)?;
    let (ours_fm, ours_body) = parse_sides(&ours_raw)?;
    let (theirs_fm, theirs_body) = parse_sides(&theirs_raw)?;

    let fm_outcome = three_way_merge_frontmatter(&base_fm, &ours_fm, &theirs_fm);
    let (merged_body, body_conflict) = three_way_merge_body(&base_body, &ours_body, &theirs_body)?;

    // Stitch frontmatter + body. On clean frontmatter we round-trip
    // through `format_text` so the output is canonical. On conflict
    // we keep the raw text (which contains real `<<<<<<<` markers) —
    // running format_text would either fail (invalid YAML) or strip
    // the markers if we ever made the parser tolerant.
    let final_text = match &fm_outcome {
        FrontmatterMerge::Clean(map) => {
            let stitched =
                stitch_clean(map, &merged_body).context("cannot serialise merged frontmatter")?;
            if body_conflict {
                // Body conflict: don't run fmt, the body contains
                // `<<<<<<<` markers that aren't markdown content.
                stitched
            } else {
                format_text(&stitched).context("cannot canonicalise merged item")?
            }
        }
        FrontmatterMerge::Conflicted(text) => stitch_conflicted(text, &merged_body),
    };

    let had_conflict = body_conflict || matches!(fm_outcome, FrontmatterMerge::Conflicted(_));

    // Write the output under the repo flock + atomically. Reuse
    // `write_item_atomic` only for clean merges (it goes through
    // serialize_item which would re-frame the YAML); for conflicted
    // outputs we have raw text with conflict markers so we use a
    // direct temp+rename to preserve byte-for-byte output. The lock
    // covers both paths.
    if let Some(root) = repo_root_for(&args.output) {
        let _lock =
            WriteLock::acquire(&root).context("cannot acquire repo write lock for merge output")?;
        write_atomic_text(&args.output, &final_text)?;
    } else {
        // Output path doesn't live under a recognisable repo (rare —
        // git always invokes us with paths inside the repo). Fall
        // back to a non-locked atomic write rather than failing the
        // merge entirely.
        write_atomic_text(&args.output, &final_text)?;
    }

    if had_conflict {
        Ok(1)
    } else {
        Ok(0)
    }
}

fn parse_sides(text: &str) -> Result<(Mapping, String)> {
    let split = item_text::split(text);
    let fm: Mapping = match split.frontmatter {
        Some(yaml) if !yaml.trim().is_empty() => {
            serde_yaml::from_str(yaml).context("cannot parse frontmatter")?
        }
        _ => Mapping::new(),
    };
    Ok((fm, split.body.to_string()))
}

#[derive(Debug)]
enum FrontmatterMerge {
    Clean(Mapping),
    /// Pre-formatted frontmatter text (without surrounding `---`
    /// delimiters) that contains real `<<<<<<<` conflict markers.
    Conflicted(String),
}

const ARRAY_UNION_FIELDS: &[&str] = &["labels", "related", "blocked_by"];

fn three_way_merge_frontmatter(
    base: &Mapping,
    ours: &Mapping,
    theirs: &Mapping,
) -> FrontmatterMerge {
    let mut clean = Mapping::new();
    let mut conflicts: Vec<FieldConflict> = Vec::new();

    // Walk every key that appears anywhere. Non-string keys are
    // preserved verbatim by tracking them separately.
    let mut all_string_keys: BTreeSet<String> = BTreeSet::new();
    for k in base.keys().chain(ours.keys()).chain(theirs.keys()) {
        if let Some(s) = k.as_str() {
            all_string_keys.insert(s.to_string());
        }
    }
    // Non-string keys: take ours first, then any new from theirs.
    // (Reviewer flag C5: silent drop is data loss. Preservation policy:
    // ours-wins on value collision for these uncommon keys.)
    let mut nonstring_emit: Vec<(Value, Value)> = Vec::new();
    let mut seen_nonstring: BTreeSet<String> = BTreeSet::new();
    for src in [ours, theirs] {
        for (k, v) in src.iter() {
            if k.as_str().is_some() {
                continue;
            }
            let fp = format!("{:?}", k);
            if seen_nonstring.insert(fp) {
                nonstring_emit.push((k.clone(), v.clone()));
            }
        }
    }

    for key in &all_string_keys {
        let kv = Value::String(key.clone());
        let bv = base.get(&kv);
        let ov = ours.get(&kv);
        let tv = theirs.get(&kv);

        if ARRAY_UNION_FIELDS.contains(&key.as_str()) {
            if let Some(seq) = merge_array_union(bv, ov, tv) {
                clean.insert(kv, seq);
            }
            continue;
        }

        if key == "commits" {
            if let Some(seq) = merge_commits(ov, tv) {
                clean.insert(kv, seq);
            }
            continue;
        }

        if key == "updated" {
            // Pick the newer of (ours, theirs). Ignoring base prevents
            // a stale base value from beating a deliberate downward
            // edit on both branches (P7).
            let candidates: Vec<&str> = [ov, tv]
                .iter()
                .filter_map(|c| c.and_then(|v| v.as_str()))
                .collect();
            if let Some(s) = candidates.into_iter().max() {
                clean.insert(kv, Value::String(s.to_string()));
            } else if let Some(b) = bv.and_then(|v| v.as_str()) {
                // Both sides cleared — keep base (rare; matches the
                // "no explicit overwrite on either side" intuition).
                clean.insert(kv, Value::String(b.to_string()));
            }
            continue;
        }

        match scalar_three_way(bv, ov, tv) {
            ScalarMerge::Drop => {}
            ScalarMerge::Keep(v) => {
                clean.insert(kv, v);
            }
            ScalarMerge::Conflict {
                ours_val,
                theirs_val,
            } => {
                conflicts.push(FieldConflict {
                    key: key.clone(),
                    ours: ours_val,
                    theirs: theirs_val,
                });
            }
        }
    }

    // Re-attach non-string keys at the end in deterministic order.
    for (k, v) in nonstring_emit {
        clean.insert(k, v);
    }

    if conflicts.is_empty() {
        FrontmatterMerge::Clean(clean)
    } else {
        FrontmatterMerge::Conflicted(render_conflicted_frontmatter(&clean, &conflicts))
    }
}

#[derive(Debug)]
struct FieldConflict {
    key: String,
    ours: Option<Value>,
    theirs: Option<Value>,
}

#[derive(Debug)]
enum ScalarMerge {
    Drop,
    Keep(Value),
    Conflict {
        ours_val: Option<Value>,
        theirs_val: Option<Value>,
    },
}

/// 3-way scalar merge using the full (base, ours, theirs) triple. The
/// key insight relative to a 2-way merge: "absent on one side" can mean
/// either "this side deleted it" (when present in base) OR "this side
/// didn't touch it" (when also absent in base). Conflating those two
/// produced spurious conflicts on every one-sided field add (C3).
fn scalar_three_way(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
) -> ScalarMerge {
    match (base, ours, theirs) {
        (None, None, None) => ScalarMerge::Drop,

        // Absent in base + only one side has a value → that side added
        // it; the other side simply didn't touch the field.
        (None, Some(o), None) => ScalarMerge::Keep(o.clone()),
        (None, None, Some(t)) => ScalarMerge::Keep(t.clone()),

        // Absent in base + both sides added.
        (None, Some(o), Some(t)) => {
            if o == t {
                ScalarMerge::Keep(o.clone())
            } else {
                ScalarMerge::Conflict {
                    ours_val: Some(o.clone()),
                    theirs_val: Some(t.clone()),
                }
            }
        }

        // Present in base; both deleted.
        (Some(_), None, None) => ScalarMerge::Drop,

        // Present in base; one side deleted, other side present.
        (Some(b), Some(o), None) => {
            if o == b {
                // Ours unchanged, theirs deleted → take the delete.
                ScalarMerge::Drop
            } else {
                // Ours edited AND theirs deleted → conflict.
                ScalarMerge::Conflict {
                    ours_val: Some(o.clone()),
                    theirs_val: None,
                }
            }
        }
        (Some(b), None, Some(t)) => {
            if t == b {
                ScalarMerge::Drop
            } else {
                ScalarMerge::Conflict {
                    ours_val: None,
                    theirs_val: Some(t.clone()),
                }
            }
        }

        // Present in base; both sides have values.
        (Some(b), Some(o), Some(t)) => {
            if o == t {
                ScalarMerge::Keep(o.clone())
            } else if o == b {
                ScalarMerge::Keep(t.clone())
            } else if t == b {
                ScalarMerge::Keep(o.clone())
            } else {
                ScalarMerge::Conflict {
                    ours_val: Some(o.clone()),
                    theirs_val: Some(t.clone()),
                }
            }
        }
    }
}

/// 3-way array merge. Items present in only one side that aren't in
/// base are adds; items in base + ours-or-theirs but not both are
/// deletes. The result is the union minus deletions.
fn merge_array_union(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
) -> Option<Value> {
    let b = as_string_set(base);
    let o = as_string_set(ours);
    let t = as_string_set(theirs);

    let mut keep: BTreeSet<String> = BTreeSet::new();
    let union: BTreeSet<&String> = o.union(&t).collect();
    for item in union {
        let in_ours = o.contains(item);
        let in_theirs = t.contains(item);
        let in_base = b.contains(item);
        let deleted_by_ours = in_base && !in_ours;
        let deleted_by_theirs = in_base && !in_theirs;
        if deleted_by_ours || deleted_by_theirs {
            continue;
        }
        keep.insert(item.clone());
    }
    if keep.is_empty() {
        return None;
    }
    let seq: Vec<Value> = keep.into_iter().map(Value::String).collect();
    Some(Value::Sequence(seq))
}

fn as_string_set(v: Option<&Value>) -> BTreeSet<String> {
    match v {
        Some(Value::Sequence(s)) => s
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => BTreeSet::new(),
    }
}

/// Union commits by hash; ours wins on summary collision. Order is
/// first-appearance: ours' commits in their original order, then
/// theirs' new ones. Deletions are not honoured — commits are
/// append-only by current CLI contract (no `--remove-commit`); a
/// commit removed manually on one branch will reappear from the
/// other branch's copy. Documented here so callers don't expect
/// delete-respect semantics symmetric with `merge_array_union`.
fn merge_commits(ours: Option<&Value>, theirs: Option<&Value>) -> Option<Value> {
    let mut order: Vec<String> = Vec::new();
    let mut by_hash: BTreeMap<String, Mapping> = BTreeMap::new();

    for (side, is_ours) in [(ours, true), (theirs, false)] {
        let Some(Value::Sequence(seq)) = side else {
            continue;
        };
        for entry in seq {
            let Value::Mapping(m) = entry else { continue };
            let Some(hash) = m
                .get(Value::String("hash".into()))
                .and_then(|v| v.as_str())
                .map(str::to_string)
            else {
                continue;
            };
            if !by_hash.contains_key(&hash) {
                order.push(hash.clone());
                by_hash.insert(hash, m.clone());
            } else if is_ours {
                by_hash.insert(hash, m.clone());
            }
        }
    }
    if order.is_empty() {
        return None;
    }
    let seq: Vec<Value> = order
        .into_iter()
        .map(|h| Value::Mapping(by_hash.remove(&h).unwrap()))
        .collect();
    Some(Value::Sequence(seq))
}

fn three_way_merge_body(
    base_body: &str,
    ours_body: &str,
    theirs_body: &str,
) -> Result<(String, bool)> {
    // Body-only temp files so frontmatter differences don't show up
    // as body conflicts.
    let dir = tempfile::tempdir().context("cannot create temp dir for body merge")?;
    let bp = dir.path().join("base.body");
    let op = dir.path().join("ours.body");
    let tp = dir.path().join("theirs.body");
    std::fs::write(&bp, base_body).context("write base body")?;
    std::fs::write(&op, ours_body).context("write ours body")?;
    std::fs::write(&tp, theirs_body).context("write theirs body")?;

    let out = Command::new("git")
        .args([
            "merge-file",
            "--stdout",
            "-L",
            "ours",
            "-L",
            "base",
            "-L",
            "theirs",
        ])
        .arg(&op)
        .arg(&bp)
        .arg(&tp)
        .output()
        .with_context(|| "cannot invoke `git merge-file`")?;

    let code = out.status.code();
    let merged_text = String::from_utf8_lossy(&out.stdout).to_string();
    match code {
        Some(0) => Ok((merged_text, false)),
        // git merge-file uses 1..=127 for "n conflicts". Anything else
        // (including signal-killed -1) is a real failure.
        Some(n) if (1..=127).contains(&n) => Ok((merged_text, true)),
        Some(other) => Err(anyhow!(
            "git merge-file failed with status {other}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        None => Err(anyhow!(
            "git merge-file terminated by signal: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

fn stitch_clean(fm: &Mapping, body: &str) -> Result<String> {
    let item = ItemFile {
        frontmatter: fm.clone(),
        body: if body.starts_with('\n') {
            body.to_string()
        } else {
            format!("\n{body}")
        },
    };
    write::serialize_item(&item)
}

/// Stitch a frontmatter that already contains `<<<<<<<` markers (raw
/// text, no leading/trailing `---`) plus the body. Output is invalid
/// YAML by design — that's how we make sure no parser quietly accepts
/// a "merged" file.
fn stitch_conflicted(fm_text: &str, body: &str) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(fm_text);
    if !fm_text.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n");
    if body.starts_with('\n') {
        out.push_str(body);
    } else {
        out.push('\n');
        out.push_str(body);
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Render the conflicted-frontmatter form: clean fields first, then a
/// real `<<<<<<<` / `=======` / `>>>>>>>` block listing the
/// conflicting fields on each side. The `<<<<<<<` form is intentional:
/// invalid YAML is exactly the signal we want — every parser stops,
/// every IDE merge editor renders the conflict, and `git mergetool`
/// works as usual. C4 fix.
fn render_conflicted_frontmatter(clean: &Mapping, conflicts: &[FieldConflict]) -> String {
    let mut out = String::new();
    if !clean.is_empty() {
        let yaml = serde_yaml::to_string(clean).unwrap_or_default();
        out.push_str(&yaml);
    }
    out.push_str("<<<<<<< ours\n");
    for c in conflicts {
        if let Some(v) = &c.ours {
            out.push_str(&render_field_line(&c.key, v));
        }
    }
    out.push_str("=======\n");
    for c in conflicts {
        if let Some(v) = &c.theirs {
            out.push_str(&render_field_line(&c.key, v));
        }
    }
    out.push_str(">>>>>>> theirs\n");
    out
}

fn render_field_line(key: &str, val: &Value) -> String {
    let mut single = Mapping::new();
    single.insert(Value::String(key.to_string()), val.clone());
    // serde_yaml emits e.g. `status: in-progress\n`. Strip leading
    // `---` in case to_string ever adds it (it doesn't for Mapping,
    // but guard anyway).
    serde_yaml::to_string(&single).unwrap_or_else(|_| format!("{key}: ?\n"))
}

fn write_atomic_text(target: &Path, text: &str) -> Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| anyhow!("target has no parent: {}", target.display()))?;
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let mut tf = tempfile::Builder::new()
        .prefix(".issuectl-tmp-")
        .tempfile_in(dir)
        .with_context(|| format!("cannot create tempfile in {}", dir.display()))?;
    use std::io::Write;
    tf.as_file_mut()
        .write_all(text.as_bytes())
        .with_context(|| format!("cannot write {}", target.display()))?;
    tf.as_file()
        .sync_all()
        .with_context(|| format!("cannot fsync {}", target.display()))?;
    tf.persist(target)
        .map_err(|e| anyhow!("cannot persist tempfile: {e}"))?;
    Ok(())
}

/// Try to find the issuectl repo root above `output` so we can take
/// the same flock as the rest of the mutation surface. Walks upward
/// looking for an `issues/` directory or a `.git`. Returns `None` if
/// no recognisable root exists (e.g. driver invoked from a test fixture
/// outside any repo).
fn repo_root_for(output: &Path) -> Option<PathBuf> {
    let mut cur = output.parent();
    while let Some(p) = cur {
        if p.join("issues").is_dir() || p.join(".git").exists() {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}

/// Print the snippet a downstream user pastes into their repo to
/// activate the driver. `apply` runs `git config` for them — never
/// silently mutates `.gitattributes` (committed, shared) or git
/// global config (cross-repo blast radius).
pub fn install(root: &Path, apply: bool) -> Result<()> {
    let attr_line = "issues/**/item.md merge=issuectl-yaml";
    let driver_value = driver_value();
    let config_cmd = format!("git config merge.issuectl-yaml.driver \"{driver_value}\"");
    println!("Add to .gitattributes (commit this):");
    println!("  {attr_line}");
    println!();
    println!("Then run (per-repo, in your local config):");
    println!("  {config_cmd}");
    if apply {
        // `--apply` is the explicit user request, so existing differing
        // values are overwritten — matches the behavior in place
        // before init grew its own consent gate.
        let outcome = apply_driver_config(root, &driver_value, true)?;
        match outcome {
            InstallOutcome::Configured => println!(
                "\nApplied: merge.issuectl-yaml.driver is now configured for {}.",
                root.display()
            ),
            InstallOutcome::AlreadyConfigured => println!(
                "\nAlready configured: merge.issuectl-yaml.driver is set for {}.",
                root.display()
            ),
        }
        println!("Note: .gitattributes is NOT modified — add the line yourself and commit.");
    }
    Ok(())
}

/// Outcome reporter for `install_quiet`. `Configured` means we wrote
/// (or rewrote) the `merge.issuectl-yaml.driver` git-config value;
/// `AlreadyConfigured` means it was already set to the expected value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallOutcome {
    Configured,
    AlreadyConfigured,
}

/// Apply the merge-driver git-config without printing. Returns whether
/// the config value was already set to the expected merge-driver
/// invocation. With `force=false`, refuses to overwrite an existing
/// differing value (e.g. a user's wrapper script). Used by
/// `issuectl init`; `install` wraps this and adds human-readable
/// output + the `.gitattributes` reminder.
pub fn install_config(root: &Path, force: bool) -> Result<InstallOutcome> {
    apply_driver_config(root, &driver_value(), force)
}

fn driver_value() -> String {
    // Use the absolute path of the running binary so installs survive
    // PATH changes and cargo-installed binaries that get relocated.
    // Falls back to bare `issuectl` if current_exe fails (cross-arch
    // builds, exotic platforms). Quote the path so that binaries
    // installed under directories with spaces (e.g. macOS' "Application
    // Support" or iCloud-synced paths) still resolve correctly when
    // git's merge driver is invoked through a shell.
    let exe = std::env::current_exe()
        .map(|p| sh_quote(&p.to_string_lossy()))
        .unwrap_or_else(|_| "issuectl".to_string());
    format!("{exe} merge-driver --base %O --ours %A --theirs %B --output %A")
}

/// POSIX single-quote escape: wraps the value in `'…'`, replacing any
/// embedded `'` with the canonical `'\''` sequence. Sufficient for
/// every shell git invokes the merge driver under (sh, bash, zsh).
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn apply_driver_config(
    root: &Path,
    driver_value: &str,
    force: bool,
) -> Result<InstallOutcome> {
    let existing = Command::new("git")
        .current_dir(root)
        .args(["config", "--local", "--get", "merge.issuectl-yaml.driver"])
        .output()
        .context("cannot invoke `git config --get`")?;
    if existing.status.success() {
        let cur = String::from_utf8_lossy(&existing.stdout).trim().to_string();
        if cur == driver_value {
            return Ok(InstallOutcome::AlreadyConfigured);
        }
        if !force {
            bail!(
                "merge.issuectl-yaml.driver is already set to a different value \
                 ({cur:?}); pass --force to overwrite, or unset it manually first"
            );
        }
    }
    let out = Command::new("git")
        .current_dir(root)
        .args(["config", "merge.issuectl-yaml.driver", driver_value])
        .output()
        .context("cannot invoke `git config`")?;
    if !out.status.success() {
        bail!(
            "git config merge.issuectl-yaml.driver failed (exit {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(InstallOutcome::Configured)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_files(base: &str, ours: &str, theirs: &str) -> (tempfile::TempDir, MergeArgs) {
        let tmp = tempfile::tempdir().unwrap();
        let b = tmp.path().join("base");
        let o = tmp.path().join("ours");
        let t = tmp.path().join("theirs");
        let out = tmp.path().join("out");
        std::fs::write(&b, base).unwrap();
        std::fs::write(&o, ours).unwrap();
        std::fs::write(&t, theirs).unwrap();
        std::fs::write(&out, ours).unwrap();
        (
            tmp,
            MergeArgs {
                base: b,
                ours: o,
                theirs: t,
                output: out,
            },
        )
    }

    #[test]
    fn array_union_both_add() {
        let base = "---\nlabels: [a]\n---\n# T\n";
        let ours = "---\nlabels: [a, b]\n---\n# T\n";
        let theirs = "---\nlabels: [a, c]\n---\n# T\n";
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 0);
        let merged = std::fs::read_to_string(&args.output).unwrap();
        assert!(merged.contains("labels: [a, b, c]"), "got: {merged}");
    }

    #[test]
    fn array_one_side_deletes() {
        let base = "---\nlabels: [a, b]\n---\n# T\n";
        let ours = "---\nlabels: [b]\n---\n# T\n";
        let theirs = "---\nlabels: [a, b]\n---\n# T\n";
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 0);
        let merged = std::fs::read_to_string(&args.output).unwrap();
        assert!(merged.contains("labels: [b]"));
        assert!(!merged.contains("labels: [a"));
    }

    #[test]
    fn scalar_one_sided_change() {
        let base = "---\nstatus: open\npriority: normal\ntype: bug\n---\n# T\n";
        let ours = "---\nstatus: open\npriority: normal\ntype: bug\n---\n# T\n";
        let theirs = "---\nstatus: in-progress\npriority: normal\ntype: bug\n---\n# T\n";
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 0);
        let merged = std::fs::read_to_string(&args.output).unwrap();
        assert!(merged.contains("status: in-progress"));
    }

    #[test]
    fn scalar_one_sided_add_from_absent_base_kept() {
        // C3 regression test: ours adds `assignee: alice`, theirs doesn't
        // touch the field, base doesn't have it. Must NOT conflict.
        let base = "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n";
        let ours = "---\ntype: bug\nstatus: open\npriority: normal\nassignee: alice\n---\n# T\n";
        let theirs = base;
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 0, "one-sided add must not conflict");
        let merged = std::fs::read_to_string(&args.output).unwrap();
        assert!(merged.contains("assignee: alice"), "got: {merged}");
    }

    #[test]
    fn scalar_both_diverge_emits_real_conflict_markers() {
        // C4 regression test: diverging scalars must produce real
        // `<<<<<<<` markers, not a parseable __conflict_*__ key.
        let base = "---\nstatus: open\npriority: normal\ntype: bug\n---\n# T\n";
        let ours = "---\nstatus: in-progress\npriority: normal\ntype: bug\n---\n# T\n";
        let theirs = "---\nstatus: testing\npriority: normal\ntype: bug\n---\n# T\n";
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 1, "diverged scalar must produce conflict");
        let merged = std::fs::read_to_string(&args.output).unwrap();
        assert!(merged.contains("<<<<<<< ours"), "missing markers: {merged}");
        assert!(merged.contains("======="), "missing markers: {merged}");
        assert!(
            merged.contains(">>>>>>> theirs"),
            "missing markers: {merged}"
        );
        assert!(merged.contains("status: in-progress"));
        assert!(merged.contains("status: testing"));
        // Parsing the result as YAML must FAIL — that's the signal.
        let frontmatter = merged.split("---").nth(1).unwrap_or("");
        assert!(
            serde_yaml::from_str::<serde_yaml::Value>(frontmatter).is_err(),
            "conflicted frontmatter must NOT parse as valid YAML"
        );
    }

    #[test]
    fn commits_union_by_hash() {
        let base = "---\ncommits:\n- hash: a\n  summary: one\n---\n# T\n";
        let ours =
            "---\ncommits:\n- hash: a\n  summary: one\n- hash: b\n  summary: ours-add\n---\n# T\n";
        let theirs = "---\ncommits:\n- hash: a\n  summary: one\n- hash: c\n  summary: theirs-add\n---\n# T\n";
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 0);
        let merged = std::fs::read_to_string(&args.output).unwrap();
        assert!(merged.contains("ours-add"));
        assert!(merged.contains("theirs-add"));
        assert!(merged.contains("hash: a"));
    }

    #[test]
    fn updated_picks_newer_of_ours_and_theirs_ignoring_base() {
        // P7 regression: base must not beat ours/theirs.
        let base =
            "---\nupdated: 2026-12-31\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n";
        let ours =
            "---\nupdated: 2026-01-15\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n";
        let theirs =
            "---\nupdated: 2026-02-15\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n";
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 0);
        let merged = std::fs::read_to_string(&args.output).unwrap();
        assert!(merged.contains("updated: 2026-02-15"), "got: {merged}");
        assert!(!merged.contains("updated: 2026-12-31"));
    }

    #[test]
    fn body_conflict_propagates_with_real_markers() {
        let base = "---\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n\nshared\n";
        let ours = "---\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n\nours-line\n";
        let theirs = "---\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n\ntheirs-line\n";
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 1);
        let merged = std::fs::read_to_string(&args.output).unwrap();
        assert!(merged.contains("<<<<<<< ours"));
        assert!(merged.contains(">>>>>>> theirs"));
        // Labels must be `ours` / `theirs`, NOT temp paths.
        assert!(!merged.contains("/tmp/"));
        assert!(!merged.contains(".body"));
    }

    #[test]
    fn output_overwriting_ours_path_is_safe() {
        // Mirrors git's actual invocation: --output %A == --ours %A.
        let base = "---\nlabels: [a]\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n";
        let ours = "---\nlabels: [a, b]\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n";
        let theirs = "---\nlabels: [a, c]\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n";
        let tmp = tempfile::tempdir().unwrap();
        let bp = tmp.path().join("base");
        let op = tmp.path().join("ours");
        let tp = tmp.path().join("theirs");
        std::fs::write(&bp, base).unwrap();
        std::fs::write(&op, ours).unwrap();
        std::fs::write(&tp, theirs).unwrap();
        let args = MergeArgs {
            base: bp,
            ours: op.clone(),
            theirs: tp,
            output: op,
        };
        let code = run(&args).unwrap();
        assert_eq!(code, 0);
        let merged = std::fs::read_to_string(&args.output).unwrap();
        assert!(merged.contains("labels: [a, b, c]"));
    }

    #[test]
    fn merge_idempotent_with_fmt() {
        let base = "---\nstatus: open\ntype: bug\npriority: normal\nlabels: [a]\n---\n\n# T\n";
        let ours = "---\nstatus: open\ntype: bug\npriority: normal\nlabels: [a, b]\n---\n\n# T\n";
        let theirs = "---\nstatus: open\ntype: bug\npriority: normal\nlabels: [a, c]\n---\n\n# T\n";
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 0);
        let merged = std::fs::read_to_string(&args.output).unwrap();
        let formatted = format_text(&merged).unwrap();
        assert_eq!(merged, formatted, "merge output must already be canonical");
    }
}
