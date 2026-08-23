## Review: DoD delivery-status gate

**Reviewed:** `354223e..ed29f3a` across schema, transition evaluation, mutation tests, config reporting, README, changelog, and bundled `/issue` skill
**Reviewers:** gemini-3.1-pro-preview, gpt-5.6-sol, claude-opus-5, deepseek-v4-pro
**Rounds:** 2 cross-review rounds after the independent review

### Critical Issues (Consensus)

1. **Non-delivery close → delivery close bypasses strict DoD**
   - **What:** `evaluate_dod` suppresses the gate whenever the previous status is any closing status. After splitting delivery from non-delivery closes, `duplicate → fixed` and `wontfix → done` are fresh delivery transitions and must be gated.
   - **Where:** `crates/issuectl-core/src/transitions.rs`, `evaluate_dod`
   - **Why it matters:** permissive/custom transition matrices can move through a non-delivery disposition and then mark work delivered without satisfying strict acceptance criteria.
   - **Suggested fix:** compare the previous status with `is_delivery_status`, replace the now-vacuous `done → wontfix` test with a delivery→delivery test, and add non-delivery→delivery coverage.
   - **Raised by:** all four reviewers

2. **The README custom-status example breaks initialized transition matrices**
   - **What:** the example narrows the status enum to `[open, shipped, duplicate]`, while the default `.issuectl/transitions.yaml` references the other built-in statuses and rejects the mismatch on every write.
   - **Where:** `README.md`, DoD configuration example
   - **Why it matters:** copying the documented example into a normal initialized repo produces `TransitionConfig` errors before the custom status can be used.
   - **Suggested fix:** show the complete built-in status enum plus `shipped`, retain `done`/`fixed` in the delivery list, and state replacement semantics.
   - **Raised by:** all four reviewers after cross-review

3. **Explicit delivery-status policy fails open on typos**
   - **What:** `dod.delivery_statuses` accepts empty, duplicate, unknown, and lifecycle-active values. Exact matching then silently disables part or all of strict DoD.
   - **Where:** `crates/issuectl-core/src/schema.rs`, schema loadability validation
   - **Why it matters:** a typo such as `fxied` silently weakens an enforcement policy.
   - **Suggested fix:** distinguish an explicitly authored list from inherited defaults; validate explicit entries for shape, uniqueness, status-enum membership, and closing classification without rejecting inherited `done`/`fixed` in existing narrowed schemas.
   - **Raised by:** all four reviewers; they disagreed on whether it blocks this patch, not on the defect

### Partial Consensus / Minor Findings

4. **Strict-only backward compatibility lacks a positive load-path regression**
   - A mutation test writes only `dod.strict: true`, but checks only that `duplicate` is ungated. That would also pass if the default delivery list were accidentally empty. Add a loaded-schema `done`/`fixed` strict assertion.

5. **`evaluate_dod` rustdoc contradicts behavior**
   - It says an empty Acceptance Criteria section passes, although missing/zero-item sections warn or error. It also describes only active→built-in delivery despite custom statuses and the required non-delivery→delivery behavior.

6. **Built-in delivery names have no central constant/drift guard**
   - `done` and `fixed` are copied into a new helper independently from the existing issue-field status constants. A central `DELIVERY_STATUSES` constant or a focused drift test would make the taxonomy explicit.

### Disputed Issues

1. **Should an explicit empty delivery list be rejected?**
   - **For rejection:** empty strict policy is fail-open.
   - **Against:** it is a clear opt-out from the zero-config gate.
   - **Moderator's take:** allow and document it. Empty is unambiguous; typoed non-empty values are the dangerous case.

2. **Should configured active-class statuses be rejected or merely inert?**
   - **For rejection:** an explicit contradiction is almost certainly an operator mistake.
   - **Against:** lifecycle overrides are deliberately authoritative and historically allowed to disable closing behavior.
   - **Moderator's take:** inherited values may become inert without breaking load; explicitly listed active values should be rejected because the user asserted both sides of a contradiction in the same effective policy.

3. **Should `config show` add an effective derived gate set?**
   - **For:** operators otherwise must intersect `delivery_statuses` and `status_classes` themselves.
   - **Against:** validated explicit config plus the two existing inspectable values is sufficient; another derived key adds API surface.
   - **Moderator's take:** defer. Proper load validation removes the main silent-failure mode.

### Dropped Concerns

- **Use `BTreeSet` instead of `Vec`:** the list is tiny; validation can reject duplicates while preserving declared order.
- **Duplicate DoD and explicit transition-rule messages:** real but pre-existing and narrowed, not introduced by this patch.
- **`ready` command mismatch:** the README text describes transition behavior adjacent to the read-only report; no separate delivery classification exists in `ready`, so no mismatch was verified.
- **Doctor should audit historical DoD:** pre-existing scope expansion, not evidence against this transition fix.

### What's Solid

- The direct active→non-delivery warning/strict regressions cover all built-in dispositions.
- Custom delivery statuses flow through schema lifecycle classification in both warning and strict modes.
- `config show`, README, changelog, and bundled skill surfaces were updated together.

### Moderator's assessment

OpenAI gave the strongest first-pass implementation review; Claude gave the strongest cross-review by proving the existing closing test was vacuous and tracing the README example into transition validation. The single most important fix is changing the source-side predicate from closing to delivery and pinning both directions with tests. Presence-aware validation is also warranted because strict configuration must not silently fail open, but it must preserve inherited defaults for narrowed existing schemas.

**Review workflow note:** OpenAI's original continuation thread expired in cross-review round 1. The required reviewer was retried successfully as a fresh OpenAI run in round 2; its final response independently confirmed the consensus blockers.
