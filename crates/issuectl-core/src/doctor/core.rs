use super::*;

pub(crate) fn rel(repo_root: &Path, p: &Path) -> String {
    p.strip_prefix(repo_root)
        .unwrap_or(p)
        .to_string_lossy()
        .to_string()
}

pub fn run(repo_root: &Path, fix: bool, json: bool, verbose: bool) -> Result<()> {
    run_via(repo_root, fix, json, verbose, &crate::clock::SystemClock)
}

/// Clock-injected variant of [`run`].
pub fn run_via(
    repo_root: &Path,
    fix: bool,
    json: bool,
    verbose: bool,
    clock: &dyn crate::clock::Clock,
) -> Result<()> {
    let mut findings = scan_via(repo_root, clock)?;
    let outcome: Option<ApplyOutcome> = if fix {
        // D2: hold the repo write lock through the apply pass so doctor
        // doesn't race CLI/server mutations. Re-scan under the lock to
        // ensure the plan reflects the locked-state filesystem.
        let lock = crate::mutate::WriteLock::acquire(repo_root)?;
        findings = scan_via(repo_root, clock)?;
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply_via(repo_root, actions, &lock, clock)?;
        // ALWAYS re-scan after apply, regardless of fix_applied. The
        // call to `DoctorActions::from_findings` drained findings via
        // `mem::take` (legacy_dirs / flat_layout_plan / notes_to_rename
        // / orphan_tempfiles / status reconciliation lists) — without
        // this rescan, render_text and render_json would receive a
        // gutted `findings` on preflight-blocked or no-write runs, and
        // the user would see "doctor: cannot apply --fix" with NONE of
        // the actual to-do lists below. Caches are hot post-apply, so
        // the I/O is negligible.
        findings = scan_via(repo_root, clock)?;
        Some(outcome)
    } else {
        None
    };

    let exit_decision = classify_exit(&findings, outcome.as_ref(), fix);
    if json {
        // The envelope-on-stderr contract is scoped to `--fix --json`
        // per the success criteria: read-only `--json doctor` keeps
        // the historical behaviour of emitting the full result on
        // stdout regardless of exit code, so existing scripts doing
        // `issuectl --json doctor | jq …` on an unhealthy repo still
        // work. Issue: @doctor-fix-noop.
        if fix && exit_decision.code != 0 {
            let details = render_json(&findings, outcome.as_ref(), fix, repo_root);
            let envelope = crate::envelope::error(
                exit_decision.error_code,
                &exit_decision.message,
                serde_json::json!({"details": details}),
            );
            eprintln!("{}", serde_json::to_string_pretty(&envelope)?);
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&crate::envelope::success(&render_json(
                    &findings,
                    outcome.as_ref(),
                    fix,
                    repo_root
                ))?)?
            );
        }
    } else {
        render_text(&findings, outcome.as_ref(), fix, verbose);
    }
    if exit_decision.code != 0 {
        std::process::exit(exit_decision.code);
    }
    Ok(())
}

/// Decision returned by `classify_exit`. `code == 0` means a clean
/// run. Non-zero `code` carries a stable `error_code` + human
/// `message` for the `--json` error envelope (issue: @doctor-fix-noop).
#[derive(Debug, Clone)]
pub(crate) struct ExitDecision {
    pub(crate) code: i32,
    pub(crate) error_code: &'static str,
    pub(crate) message: String,
}

/// Compute the exit code + envelope code/message from a post-apply
/// findings scan and optional outcome. Pure: extracted from `run` so
/// the mapping is unit-testable (issue: @doctor-fix-noop, success
/// criterion D).
///
/// Mapping:
///   - `outcome.apply_error.is_some()` → `doctor-apply-error` (exit 1)
///   - `stop_phase == Preflight`       → `doctor-blocked`     (exit 1)
///   - `stop_phase == PostApply`       → `doctor-partial`     (exit 1)
///   - critical findings remain        → `doctor-partial`     (exit 1)
///   - `notes_conflicts_at_apply` non-empty after a clean `Ok` apply
///     → `doctor-partial` (exit 1; the apply ran but some manual-only
///     work is left for the user)
///   - else → exit 0
pub(crate) fn classify_exit(
    findings: &DoctorFindings,
    outcome: Option<&ApplyOutcome>,
    fix: bool,
) -> ExitDecision {
    let ok = ExitDecision {
        code: 0,
        error_code: "",
        message: String::new(),
    };
    let crit = critical_blockers(findings);
    if let Some(oc) = outcome {
        if let Some(err) = &oc.apply_error {
            return ExitDecision {
                code: 1,
                error_code: "doctor-apply-error",
                message: format!("doctor --fix aborted mid-pipeline: {err}"),
            };
        }
        match oc.stop_phase {
            StopPhase::Preflight => {
                return ExitDecision {
                    code: 1,
                    error_code: "doctor-blocked",
                    message: format!(
                        "doctor --fix refused: {} preflight blocker(s)",
                        oc.blockers.len()
                    ),
                };
            }
            StopPhase::PostApply => {
                return ExitDecision {
                    code: 1,
                    error_code: "doctor-partial",
                    message: format!(
                        "doctor --fix partial: {} post-apply blocker(s) remain after partial writes",
                        oc.blockers.len()
                    ),
                };
            }
            StopPhase::Ok => {
                // Manual-merge notes/comments findings produce a
                // specific message — checked BEFORE the generic
                // `crit` branch because notes_conflicts persists in
                // `findings.notes_conflicts` (and therefore in
                // `crit` via `critical_blockers`) after the apply
                // pass that recorded them; the generic branch would
                // otherwise mask the specific guidance the user
                // needs.
                if !oc.notes_conflicts_at_apply.is_empty() {
                    return ExitDecision {
                        code: 1,
                        error_code: "doctor-partial",
                        message: format!(
                            "doctor --fix partial: {} issue(s) need manual `## Notes`/`## Comments` merge",
                            oc.notes_conflicts_at_apply.len()
                        ),
                    };
                }
                if !crit.is_empty() {
                    return ExitDecision {
                        code: 1,
                        error_code: "doctor-partial",
                        message: format!(
                            "doctor --fix partial: {} unfixable finding(s) remain",
                            remaining_finding_count(findings)
                        ),
                    };
                }
                return ok;
            }
        }
    }
    // Read-only path: any critical finding drives exit 1 too. No
    // envelope code distinction for `--fix=false` (no `apply_outcome`
    // to attach), but if `--json doctor` is run on an unhealthy repo
    // we still emit the structured envelope so scripts parse one
    // shape.
    if !crit.is_empty() {
        return ExitDecision {
            code: 1,
            error_code: if fix {
                "doctor-partial"
            } else {
                "doctor-unhealthy"
            },
            message: format!("doctor: {} unfixable finding(s)", crit.len()),
        };
    }
    ok
}

/// Scope for `blockers_for` — disambiguates "is the repo unhealthy
/// enough to exit 1?" (the broad set) from "is the repo unsafe to
/// run the apply pipeline against?" (the narrow, layout-fatal
/// subset). Schema-shape findings (schema violations, non-legacy
/// broken refs, dependency cycles, status/timestamp inconsistencies)
/// drive the exit code but are NOT layout-fatal: the safest, most
/// mechanical phase (`--fix`'s flat-layout migration) just renames
/// directories and is independent of frontmatter contents. Treating
/// schema findings as preflight blockers forced users with hundreds
/// of pre-existing schema violations to hand-fix every one of them
/// before doctor would lift a finger — the largest single adoption
/// blocker reported in downstream project 0.5.1 feedback (@intensely-ill-garden,
/// @staggeringly-important-zoo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockerScope {
    /// Drives the run-time exit code. The full set of "user must
    /// intervene" findings.
    ExitCode,
    /// Drives the `--fix` preflight refusal AND the post-flat-layout
    /// safety re-check. Narrower than `ExitCode`: schema-shape
    /// findings are filtered out so `--fix` can still migrate the
    /// layout while schema violations are pending. The user fixes
    /// the schema violations afterward against the post-migration
    /// state — the same scan output ranks them under exit-1 so they
    /// remain visible.
    ApplyPreflight,
}

/// Single-source-of-truth predicate for "the repo is in a state the
/// user must intervene on". Drives both the exit code (`run` checks
/// `!critical_blockers(&findings).is_empty()`) AND the `--fix`
/// preflight check via the narrower `apply_blockers` view.
/// Previously these two callers held drifting copies of the rule
/// set, which produced two failure modes the spin-off flagged: (a)
/// `--fix` would mutate a repo `has_critical_findings` rated
/// critical (partial mutations on a critically-unhealthy repo), and
/// (b) parse-error classification used a fragile substring matcher
/// whose Hard/Soft split was easy to flip by re-wording the parser's
/// message.
pub(crate) fn critical_blockers(findings: &DoctorFindings) -> Vec<String> {
    blockers_for(findings, BlockerScope::ExitCode)
}

/// Layout-fatal subset of `critical_blockers`. Used by the `--fix`
/// preflight check and the post-flat-layout safety re-check.
pub(crate) fn apply_blockers(findings: &DoctorFindings) -> Vec<String> {
    blockers_for(findings, BlockerScope::ApplyPreflight)
}

pub(crate) fn blockers_for(findings: &DoctorFindings, scope: BlockerScope) -> Vec<String> {
    let layout_only = matches!(scope, BlockerScope::ApplyPreflight);
    let mut blockers: Vec<String> = Vec::new();

    if !findings.flat_layout_conflicts.is_empty() {
        let detail = findings
            .flat_layout_conflicts
            .iter()
            .map(|c| format!("    {}: {}", c.slug, c.detail))
            .collect::<Vec<_>>()
            .join("\n");
        blockers.push(format!("flat-layout migration conflicts:\n{detail}"));
    }
    if !findings.duplicate_slugs.is_empty() {
        blockers.push(format!("duplicate slugs: {:?}", findings.duplicate_slugs));
    }
    if !findings.both_open_and_closed.is_empty() {
        blockers.push(format!(
            "slugs present in BOTH issues/open/ and issues/closed/: {:?}",
            findings.both_open_and_closed
        ));
    }
    if !findings.conflict_markers.is_empty() {
        blockers.push(format!(
            "git merge-conflict markers in: {:?}",
            findings.conflict_markers
        ));
    }
    let hard_parse: Vec<&ParseError> = findings
        .parse_errors
        .iter()
        .filter(|e| e.severity == ParseSeverity::Hard)
        .collect();
    if !hard_parse.is_empty() {
        let detail = hard_parse
            .iter()
            .map(|e| format!("    {}: {}", e.location, e.message))
            .collect::<Vec<_>>()
            .join("\n");
        blockers.push(format!(
            "unparseable issue file(s) ({}):\n{detail}",
            hard_parse.len()
        ));
    }
    if let Some(err) = &findings.schema_parse_error {
        blockers.push(format!("schema file parse error: {err}"));
    }
    if !layout_only && !findings.schema_violations.is_empty() {
        blockers.push(format!(
            "schema violations: {} issue(s) fail validation",
            findings.schema_violations.len()
        ));
    }
    if !findings.invalid_slugs.is_empty() {
        blockers.push(format!("invalid slugs: {:?}", findings.invalid_slugs));
    }
    if !findings.missing_item_md.is_empty() {
        blockers.push(format!(
            "directories missing item.md: {:?}",
            findings.missing_item_md
        ));
    }
    // Exclude legacy numeric refs from the critical set: they are
    // exactly what `--fix`'s legacy migration translates (number →
    // slug via `rewrite_item_frontmatter`). Treating them as critical
    // would refuse the very migration designed to heal them, so a
    // partially-flat-layout repo with a few stale `epic: 7` refs
    // could not progress. The `(legacy numeric ref)` suffix is set
    // by `populate_extended_validation::check_ref` and is the typed
    // signal — not a substring matcher on user content.
    let non_legacy_broken: Vec<&(String, String, String)> = findings
        .broken_refs
        .iter()
        .filter(|(_, _, target)| !target.ends_with("(legacy numeric ref)"))
        .collect();
    if !layout_only && !non_legacy_broken.is_empty() {
        blockers.push(format!(
            "broken cross-references: {} entry/entries",
            non_legacy_broken.len()
        ));
    }
    if !layout_only && !findings.blocked_by_cycles.is_empty() {
        blockers.push(format!(
            "dependency cycles via blocked_by: {} cycle(s)",
            findings.blocked_by_cycles.len()
        ));
    }
    if !layout_only && !findings.blocked_by_self.is_empty() {
        blockers.push(format!(
            "self-dependencies in blocked_by: {:?}",
            findings.blocked_by_self
        ));
    }
    if !layout_only && !findings.status_consistency.is_empty() {
        blockers.push(format!(
            "status/closed-date inconsistencies: {} entry/entries",
            findings.status_consistency.len()
        ));
    }
    if !layout_only && !findings.timestamp_issues.is_empty() {
        blockers.push(format!(
            "timestamp sanity issues: {} entry/entries",
            findings.timestamp_issues.len()
        ));
    }
    if !findings.symlinked_dirs.is_empty() {
        // Symlinks under `issues/` could redirect a rewrite outside
        // the repo (a `--fix` body rewrite could land on arbitrary
        // disk). Kept as a preflight blocker for safety.
        blockers.push(format!(
            "symlinked issue directories: {:?}",
            findings.symlinked_dirs
        ));
    }
    // `notes_conflicts`, `agents_md_malformed`, `agents_md_check_skipped`
    // are localised, per-file manual-merge findings. They drive exit-1
    // (so the user keeps seeing them) but MUST NOT be preflight
    // blockers — they used to silently swallow orthogonal auto-fixable
    // work (alias coercion, AGENTS.md schema-block regen, NN-rename)
    // by aborting the whole apply pass. `rename_notes_to_comments`
    // already records skipped slugs via `outcome.notes_conflicts_at_apply`,
    // and `regenerate_agents_md` is already gated on these AGENTS.md
    // flags in `DoctorActions::from_findings`. See issue: @doctor-fix-noop.
    if !layout_only {
        if !findings.notes_conflicts.is_empty() {
            blockers.push(format!(
                "## Notes / ## Comments conflicts (manual merge): {:?}",
                findings.notes_conflicts
            ));
        }
        if let Some(reason) = &findings.agents_md_malformed {
            blockers.push(format!("AGENTS.md is malformed: {reason}"));
        }
        if let Some(err) = &findings.agents_md_check_skipped {
            blockers.push(format!("AGENTS.md drift check skipped: {err}"));
        }
    }
    blockers
}
