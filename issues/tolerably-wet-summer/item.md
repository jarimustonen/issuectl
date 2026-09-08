---
created: 2026-09-08
updated: 2026-09-08
type: improvement
status: untriaged
priority: normal
provenance: ai-review
source_ref: taskfleet:01m2053akan8f49ac04s3vm67j/review-finding:taskfleet-triage-heading-contract
review_source: ai-review
originating_run: 01m2053akan8f49ac04s3vm67j
originating_run_kind: spinoff
assessment_classification: CONFIRMED
assessment_outcome: FIX
review_confidence: high
review_severity: medium
review_target: Taskfleet 0.7.1 bundled worktree-bug-analysis contract
labels:
- ai-review-model:gemini-3.1-pro-preview
- ai-review-model:gpt-5.6-sol
- ai-review-model:claude-fable-5
- ai-review-model:deepseek-v4-pro
---

# Taskfleet bug analysis should emit canonical triage heading

## Description

## Observed occurrence

Taskfleet 0.7.1's bundled `/worktree-bug-analysis` skill permits an analysis worker to append either `## Triage analysis` or `## Suspected Root Cause`. issuectl 0.18.3 intentionally derives `needs_analysis` and its `analysis` projection only from the exact `## Triage analysis` heading.

The issuectl `/issue-intake` caller now handles the released mismatch conservatively by checking the alternative parsed H2 before spawning and by not claiming that Taskfleet guarantees issuectl's canonical heading. The cross-repository contract itself remains divergent.

## Impact

A Taskfleet worker that chooses the alternative heading leaves issuectl's canonical `needs_analysis` signal true. Every consumer must carry compatibility logic or risk repeated analysis. Standardizing the Taskfleet worker contract on `## Triage analysis` would make the producer and consumer agree without broadening issuectl's exact-heading API.

## Scope

Update Taskfleet's bundled `/worktree-bug-analysis` contract and its generated copies/tests so completed issue enrichment uses the exact `## Triage analysis` heading. Coordinate the released contract before removing issuectl's caller-side compatibility handling. Do not change issuectl as part of this follow-up.
