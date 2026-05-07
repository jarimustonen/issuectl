//! Custom git merge driver for `issues/**/*.md`.
//!
//! Wired via `.gitattributes` (`issues/**/*.md merge=issuectl-yaml`) and
//! `git config merge.issuectl-yaml.driver "issuectl merge-driver --base
//! %O --ours %A --theirs %B --output %A"`. Print the snippets with
//! `issuectl install-merge-driver`.
//!
//! Semantics (kept tight on purpose — wider semantics belong in `fmt`):
//! - **frontmatter** is parsed as YAML on all three sides and merged
//!   field by field;
//!   - array fields `labels`, `related`, `blocked_by` use the standard
//!     "ours ∪ theirs minus base-deletions-respected" rule so adds from
//!     either branch survive and deletes from either branch take effect;
//!   - `commits` arrays are unioned by `hash` (ours wins on summary
//!     conflict), preserving first-appearance order — the field is a
//!     log, not a set;
//!   - `updated:` picks the lexicographically newer date (ISO-8601
//!     ensures lex ≡ chronological);
//!   - other scalars: if both sides agree, keep; if only one changed,
//!     take it; if both diverged, leave a YAML-comment conflict marker
//!     and exit 1 — never silently pick a side.
//! - **body** falls back to `git merge-file --stdout %A %O %B` so the
//!   driver does not try to be clever about markdown.
//! - On any unresolved conflict, exit 1; git surfaces the standard
//!   merge UI. The output file MUST NOT contain a "merged" frontmatter
//!   with an unresolved field collision while exiting 0.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde_yaml::{Mapping, Value};

use crate::fmt::format_text;

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

    let MergeFrontmatterOutcome {
        merged: merged_fm,
        had_conflict: fm_conflict,
    } = three_way_merge_frontmatter(&base_fm, &ours_fm, &theirs_fm);

    // Body: defer to `git merge-file --stdout`. It returns the merged
    // text on stdout and a non-zero exit on conflict; we propagate that
    // by leaving the conflict markers in place and exiting 1.
    let (merged_body, body_conflict) = three_way_merge_body(
        &base_body,
        &ours_body,
        &theirs_body,
        &args.base,
        &args.ours,
        &args.theirs,
    )?;

    // Stitch frontmatter + body and run through `fmt` so the output is
    // canonical (idempotent on a follow-up `issuectl fmt --check`).
    let stitched = stitch(&merged_fm, &merged_body);
    let final_text = if fm_conflict || body_conflict {
        // Don't run the formatter on conflict markers — `format_text`
        // would fail to parse a frontmatter block containing comment
        // markers we leave for the user to resolve. Write the raw
        // stitched text so the user sees the conflicts.
        stitched
    } else {
        format_text(&stitched).unwrap_or(stitched)
    };

    std::fs::write(&args.output, final_text)
        .with_context(|| format!("cannot write output {}", args.output.display()))?;

    if fm_conflict || body_conflict {
        Ok(1)
    } else {
        Ok(0)
    }
}

fn parse_sides(text: &str) -> Result<(Mapping, String)> {
    // Reuse the same split convention as fmt::format_text. We don't call
    // format_text here because we want raw frontmatter Mapping for
    // field-by-field merging.
    let trimmed = text.trim_start();
    if !trimmed.starts_with("---") {
        return Ok((Mapping::new(), text.to_string()));
    }
    let rest = &trimmed[3..];
    let Some(end) = rest.find("\n---") else {
        return Ok((Mapping::new(), text.to_string()));
    };
    let yaml = &rest[..end];
    let mut after = end + 4;
    if rest.as_bytes().get(after) == Some(&b'\n') {
        after += 1;
    }
    let body = rest[after..].to_string();
    let fm: Mapping = if yaml.trim().is_empty() {
        Mapping::new()
    } else {
        serde_yaml::from_str(yaml).context("cannot parse frontmatter")?
    };
    Ok((fm, body))
}

#[derive(Debug)]
struct MergeFrontmatterOutcome {
    merged: Mapping,
    had_conflict: bool,
}

const ARRAY_UNION_FIELDS: &[&str] = &["labels", "related", "blocked_by"];

fn three_way_merge_frontmatter(
    base: &Mapping,
    ours: &Mapping,
    theirs: &Mapping,
) -> MergeFrontmatterOutcome {
    let mut merged = Mapping::new();
    let mut had_conflict = false;

    let all_keys: BTreeSet<String> = base
        .keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .filter_map(|k| k.as_str().map(|s| s.to_string()))
        .collect();

    for key in &all_keys {
        let kv = Value::String(key.clone());
        let bv = base.get(&kv);
        let ov = ours.get(&kv);
        let tv = theirs.get(&kv);

        if ARRAY_UNION_FIELDS.contains(&key.as_str()) {
            if let Some(seq) = merge_array_union(bv, ov, tv) {
                merged.insert(kv, seq);
            }
            continue;
        }

        if key == "commits" {
            if let Some(seq) = merge_commits(bv, ov, tv) {
                merged.insert(kv, seq);
            }
            continue;
        }

        if key == "updated" {
            // Pick the lexicographically newer ISO date (works as
            // chronological for YYYY-MM-DD).
            let candidates = [ov, tv, bv];
            let newest = candidates
                .iter()
                .filter_map(|c| c.and_then(|v| v.as_str()))
                .max();
            if let Some(s) = newest {
                merged.insert(kv, Value::String(s.to_string()));
            }
            continue;
        }

        // Generic 3-way scalar merge.
        match scalar_three_way(bv, ov, tv) {
            ScalarMerge::Drop => {}
            ScalarMerge::Keep(v) => {
                merged.insert(kv, v);
            }
            ScalarMerge::Conflict {
                ours_val,
                theirs_val,
            } => {
                had_conflict = true;
                merged.insert(
                    Value::String(format!("__conflict_{key}__")),
                    Value::String(format!(
                        "ours={} theirs={} — resolve manually",
                        yaml_repr(&ours_val),
                        yaml_repr(&theirs_val),
                    )),
                );
                // Also write ours as the primary value so the file at
                // least parses; the __conflict__ key signals the human.
                if let Some(o) = ours_val {
                    merged.insert(Value::String(key.clone()), o);
                }
            }
        }
    }

    MergeFrontmatterOutcome {
        merged,
        had_conflict,
    }
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

fn scalar_three_way(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
) -> ScalarMerge {
    match (ours, theirs) {
        (None, None) => ScalarMerge::Drop,
        (Some(o), None) => {
            // theirs deleted. If ours == base, accept the delete; else
            // ours changed AND theirs deleted → conflict.
            if values_equal(Some(o), base) {
                ScalarMerge::Drop
            } else {
                ScalarMerge::Conflict {
                    ours_val: Some(o.clone()),
                    theirs_val: None,
                }
            }
        }
        (None, Some(t)) => {
            if values_equal(Some(t), base) {
                ScalarMerge::Drop
            } else {
                ScalarMerge::Conflict {
                    ours_val: None,
                    theirs_val: Some(t.clone()),
                }
            }
        }
        (Some(o), Some(t)) => {
            if values_equal(Some(o), Some(t)) {
                ScalarMerge::Keep(o.clone())
            } else if values_equal(Some(o), base) {
                ScalarMerge::Keep(t.clone())
            } else if values_equal(Some(t), base) {
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

fn values_equal(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

fn yaml_repr(v: &Option<Value>) -> String {
    match v {
        None => "(absent)".to_string(),
        Some(val) => serde_yaml::to_string(val)
            .unwrap_or_else(|_| "?".into())
            .trim()
            .to_string(),
    }
}

/// Standard 3-way array union: result = (ours ∪ theirs) − items deleted
/// from base by either side. An item deleted on one side and present on
/// the other (with no change in base) honours the delete.
fn merge_array_union(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
) -> Option<Value> {
    let b = as_string_set(base);
    let o = as_string_set(ours);
    let t = as_string_set(theirs);

    // Items present in both ours and theirs — keep.
    // Items present in only one side: keep iff also in base (no
    // explicit add) AND the other side did not delete them, OR they
    // were added by exactly that side.
    let mut keep: BTreeSet<String> = BTreeSet::new();
    let union: BTreeSet<&String> = o.union(&t).collect();
    for item in union {
        let in_ours = o.contains(item);
        let in_theirs = t.contains(item);
        let in_base = b.contains(item);
        let deleted_by_ours = in_base && !in_ours;
        let deleted_by_theirs = in_base && !in_theirs;
        if deleted_by_ours || deleted_by_theirs {
            // explicit delete — skip
            continue;
        }
        keep.insert(item.clone());
    }
    if keep.is_empty() && b.is_empty() && o.is_empty() && t.is_empty() {
        return None;
    }
    if keep.is_empty() {
        // Field was explicitly cleared on both sides — drop the key.
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

/// Union commits by hash; ours wins on summary conflict. Order is
/// first-appearance: ours' commits in their original order, then
/// theirs' new ones in theirs' order. Base is consulted only for
/// "is this entry an unchanged inheritance vs. a new addition" — but
/// because commits are append-only in practice we never need to drop
/// a base commit.
fn merge_commits(
    _base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
) -> Option<Value> {
    let mut order: Vec<String> = Vec::new();
    let mut by_hash: BTreeMap<String, Mapping> = BTreeMap::new();

    // Iterate ours first so first-appearance order respects ours, then
    // theirs adds new commits at the tail. The `is_ours` flag drives
    // the "ours wins on summary collision" rule — a same-hash commit
    // from theirs only overwrites if ours hasn't claimed it yet.
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
    _base_path: &Path,
    _ours_path: &Path,
    _theirs_path: &Path,
) -> Result<(String, bool)> {
    // Write the body slices to temp files so `git merge-file` operates
    // on body-only inputs — frontmatter differences must not register
    // as body conflicts (we own frontmatter merging).
    let dir = tempfile::tempdir().context("cannot create temp dir for body merge")?;
    let bp = dir.path().join("base.body");
    let op = dir.path().join("ours.body");
    let tp = dir.path().join("theirs.body");
    std::fs::write(&bp, base_body).context("write base body")?;
    std::fs::write(&op, ours_body).context("write ours body")?;
    std::fs::write(&tp, theirs_body).context("write theirs body")?;

    let out = Command::new("git")
        .args(["merge-file", "--stdout"])
        .arg(&op)
        .arg(&bp)
        .arg(&tp)
        .output()
        .with_context(|| "cannot invoke `git merge-file`")?;
    // git merge-file exits with the number of conflicts (>0) on
    // conflicting merges, or 0 for clean. <0 (255) means error.
    let status_code = out.status.code().unwrap_or(-1);
    if status_code < 0 {
        return Err(anyhow!(
            "git merge-file failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let merged_text = String::from_utf8_lossy(&out.stdout).to_string();
    let had_conflict = status_code > 0 || merged_text.contains("<<<<<<<");
    Ok((merged_text, had_conflict))
}

fn stitch(fm: &Mapping, body: &str) -> String {
    // Reuse write::serialize_item for the standard frontmatter framing.
    let item = crate::write::ItemFile {
        frontmatter: fm.clone(),
        body: if body.starts_with('\n') {
            body.to_string()
        } else {
            format!("\n{body}")
        },
    };
    crate::write::serialize_item(&item).unwrap_or_else(|_| String::new())
}

/// Print the snippet a downstream user pastes into their repo to
/// activate the driver. `apply` runs `git config` for them — we never
/// silently mutate user git config without their explicit opt-in.
pub fn install(apply: bool) -> Result<()> {
    let attr_line = "issues/**/*.md merge=issuectl-yaml";
    let config_cmd = "git config merge.issuectl-yaml.driver \
        \"issuectl merge-driver --base %O --ours %A --theirs %B --output %A\"";
    println!("Add to .gitattributes:");
    println!("  {attr_line}");
    println!();
    println!("Then run (per-repo, in your local config):");
    println!("  {config_cmd}");
    if apply {
        let status = Command::new("git")
            .args([
                "config",
                "merge.issuectl-yaml.driver",
                "issuectl merge-driver --base %O --ours %A --theirs %B --output %A",
            ])
            .status()
            .context("cannot invoke `git config`")?;
        if !status.success() {
            return Err(anyhow!("git config failed with status {status}"));
        }
        println!("\nApplied: merge.issuectl-yaml.driver is now configured for this repo.");
        println!("Note: .gitattributes is NOT modified — add the line yourself and commit.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_files(
        base: &str,
        ours: &str,
        theirs: &str,
    ) -> (tempfile::TempDir, MergeArgs) {
        let tmp = tempfile::tempdir().unwrap();
        let b = tmp.path().join("base");
        let o = tmp.path().join("ours");
        let t = tmp.path().join("theirs");
        let out = tmp.path().join("out");
        std::fs::write(&b, base).unwrap();
        std::fs::write(&o, ours).unwrap();
        std::fs::write(&t, theirs).unwrap();
        std::fs::write(&out, ours).unwrap(); // pre-seed like git would
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
        let ours = "---\nlabels: [b]\n---\n# T\n"; // ours removed `a`
        let theirs = "---\nlabels: [a, b]\n---\n# T\n"; // theirs unchanged
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 0);
        let merged = std::fs::read_to_string(&args.output).unwrap();
        assert!(merged.contains("labels: [b]"), "got: {merged}");
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
    fn scalar_both_diverge_conflict() {
        let base = "---\nstatus: open\npriority: normal\ntype: bug\n---\n# T\n";
        let ours = "---\nstatus: in-progress\npriority: normal\ntype: bug\n---\n# T\n";
        let theirs = "---\nstatus: testing\npriority: normal\ntype: bug\n---\n# T\n";
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 1, "diverged scalar must produce conflict");
        let merged = std::fs::read_to_string(&args.output).unwrap();
        assert!(merged.contains("__conflict_status__"), "got: {merged}");
    }

    #[test]
    fn commits_union_by_hash() {
        let base = "---\ncommits:\n- hash: a\n  summary: one\n---\n# T\n";
        let ours = "---\ncommits:\n- hash: a\n  summary: one\n- hash: b\n  summary: ours-add\n---\n# T\n";
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
    fn updated_picks_newest_date() {
        let base = "---\nupdated: 2026-01-01\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n";
        let ours = "---\nupdated: 2026-02-01\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n";
        let theirs = "---\nupdated: 2026-03-01\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n";
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 0);
        let merged = std::fs::read_to_string(&args.output).unwrap();
        assert!(merged.contains("updated: 2026-03-01"));
    }

    #[test]
    fn body_conflict_propagates() {
        let base = "---\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n\nshared\n";
        let ours = "---\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n\nours-line\n";
        let theirs = "---\nstatus: open\ntype: bug\npriority: normal\n---\n# T\n\ntheirs-line\n";
        let (_t, args) = write_files(base, ours, theirs);
        let code = run(&args).unwrap();
        assert_eq!(code, 1, "diverging body must produce conflict");
    }

    #[test]
    fn merge_idempotent_with_fmt() {
        // After a clean merge, the output should already pass `fmt --check`.
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
