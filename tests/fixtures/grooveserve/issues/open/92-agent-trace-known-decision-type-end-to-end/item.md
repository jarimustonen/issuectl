---
created: 2026-05-01
updated: 2026-05-01
type: task
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#60", "#62"]
labels: [agent-trace, technical-debt]
---

# 92. agent_trace: enforce KnownDecisionType end-to-end

_Source: D4 (#60) /llm-review SPIN-OFF S7_

## Description

`#60` (D4) made `InlineDecisionInput.decision_type` typed as
`KnownDecisionType` (compile-time enforced). Two related fields
remain stringly-typed:

1. **`DecisionStep.decision_type: &'a str`** — used by
   `record_step(StepRecord::Decision)`, currently called from
   `agent::run_loop`'s MaxTokens branch via the helper
   `trace::record_decision_step` (which already takes
   `KnownDecisionType` and converts). The underlying ops type
   can be tightened to compile-time enforce all callers.

2. **`ManualRunInput.decision_type: &'a str`** — used by
   `record_manual_run` (#62). The doc-comment already promises:

   > Once #60 lands a `KnownDecisionType` enum for the LLM-seam
   > decision rows, this field should switch to that type so
   > manual runs share the typo-proof catalog.

   #60 landed; the migration didn't happen.

## Scope

Two options:
- **A (single shared enum):** Change both fields to
  `KnownDecisionType` (already includes manual variants:
  `Reverted`, `ManualCorrection`, `ReprocessRequested`).
  Migrate `record_manual_run` callers.
- **B (split enums):** Introduce `InlineDecisionType` (current
  inline variants) and `ManualDecisionType` (manual variants).
  Cleaner separation but two enums to maintain.

A is simpler and matches the existing catalog; recommend A.

## Out of scope

- Removing `KnownDecisionType::ManualCorrection` etc. from the
  inline-only catalog — manual runs share `KnownDecisionType` per
  current convention.

## Acceptance criteria

- `DecisionStep.decision_type: KnownDecisionType`
- `ManualRunInput.decision_type: KnownDecisionType`
- All callers updated; no `&str` decision_type remains in the public
  ops surface
- Existing tests adapted; `cargo test --workspace` clean

## Päätös

Not MVP-blocking — string-typing-on-the-helper pattern is good
enough until a future contributor introduces a typo. File for the
audit-discipline-tightening pass before Phase 4 ships.
