---
created: 2026-09-08
updated: 2026-09-08
type: bug
status: in-progress
priority: normal
provenance: ai-review
source_ref: taskfleet:01m1zhvjmbr2tp5xkcekwd2jar/review-finding:generated-skill-contract-drift
review_source: ai-review
originating_run: 01m1zhvjmbr2tp5xkcekwd2jar
originating_run_kind: spinoff
assessment_classification: CONFIRMED
assessment_outcome: FIX_WITH_CARE
review_confidence: high
review_severity: high
review_target: issuectl 0.18.3 generated /issue family in Homebase
labels:
- ai-review-model:gemini-3.1-pro-preview
- ai-review-model:gpt-5.6-sol
- ai-review-model:claude-fable-5
- ai-review-model:deepseek-v4-pro
lane: skills
commits:
- hash: 62dc3ce5b7a644aa85507e7b9280cfcad1346f11
  summary: align generated skills with Taskfleet 0.7.1 contracts
- hash: 2e4a006f09e1e86ef3b86c29fe42aca6bb5ecbe1
  summary: harden generated intake settlement and layout guidance
- hash: 706cfaf063a338c8ecac3d651d7969ee82609ece
  summary: cover aggregate wait failures in intake contract
- hash: 752d00f8132ceaaf639945fb7c346f0f49449375
  summary: record multi-model review and assessed follow-up
---

# Generated skill contracts drift from current Taskfleet and issue layout

## Description

## Observed occurrence

Homebase regenerated all nine repository-local `/issue`, `/issue-new`, and `/issue-intake` artifacts byte-for-byte from the released issuectl 0.18.3 binary, then reviewed the generated family against Taskfleet 0.7.1's current callee contracts. The generated prose contains several concrete stale contracts:

1. `/issue-intake` tells callers to verify bug-analysis completion from `git log` because “run-status is unreliable”. Current Taskfleet requires callers to retain each run id, wait for settlement, and inspect the canonical `landed` and structured report data. Git ancestry/log checks can false-negative after rebases and the current generated flow also discards useful `success:false` reports.
2. `/issue-intake` processes bugs and features but sends every unclear item to bug-only `/worktree-bug-analysis`. The text later says feature requests rarely need analysis, but provides no compatible enrichment path when an unclear feature does need codebase analysis.
3. `/issue` still says the raw active path is `issues/open/<slug>/item.md`; issuectl's canonical active layout is flat `issues/<slug>/item.md` (closed items are separately archived/closed according to current discovery rules).
4. `/issue-intake` emits an `<!-- intake-return -->` block and claims `/stint` consumes it. The current Taskfleet conductor is `/stint-start`; it does not consume that block and instead schedules from `issuectl dag --json` after explicit disposition.

A related cross-repository contract should be coordinated with Taskfleet: `/worktree-bug-analysis` currently permits `## Suspected Root Cause` as an alternative to the exact `## Triage analysis` heading. issuectl derives `needs_analysis` only from the latter, so the alternative can cause repeat analysis. The issuectl caller should not claim the exact heading until the callee guarantees it.

## Real-world impact

Agents can prematurely brief before workers settle, lose structured failure diagnostics, repeatedly analyse an item whose heading is invisible to issuectl, invoke a bug-only workflow for a feature, or use a stale filesystem path. These are silent workflow errors in the generated contract distributed to every supported harness.

## Recommended fix

Update the source templates under `crates/issuectl-core/templates/`, not installed copies. Make `/issue-intake` use Taskfleet's current run wait/show and `landed`/report contract, gate bug-analysis invocation to compatible issue types, remove or align the unconsumed stint return block, and correct the flat-layout note. Add template/dogfood regression assertions for the stale phrases. Coordinate the exact analysis heading with Taskfleet.

## Review assessment

Classification: CONFIRMED. Outcome: FIX_WITH_CARE because the Taskfleet synchronization and output contracts should be checked against the exact released Taskfleet version while editing the upstream templates. Severity: high. Confidence: high.
