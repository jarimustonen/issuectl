# Assessment: DoD delivery-status gate

Source: `history/review-dod-delivery-gate.md`  
HEAD: `ed29f3a7f9f7e49aad7e5c4c4dba150ebefda6d2`

| # | Finding | Conf | Like | Read | Arch | Confidence | Recommendation |
|---|---|---|---|---|---|---|---|
| F1 | Non-delivery close to delivery bypasses strict DoD[^F1] | CONFIRMED | REGULAR | IMPROVES | NONE | HIGH | FIX |
| F2 | README custom-status example breaks initialized transition rules[^F2] | CONFIRMED | REGULAR | IMPROVES | NONE | HIGH | FIX |
| F3 | Explicit delivery-status policy fails open on typos[^F3] | CONFIRMED | OCCASIONAL | IMPROVES | MINOR | HIGH | FIX |
| F4 | Strict-only compatibility lacks a positive load-path regression[^F4] | CONFIRMED | OCCASIONAL | IMPROVES | NONE | HIGH | FIX |
| F5 | evaluate_dod rustdoc contradicts behavior[^F5] | CONFIRMED | REGULAR | IMPROVES | NONE | HIGH | FIX |
| F6 | Built-in delivery names lack a central constant or drift guard[^F6] | CONFIRMED | RARE | IMPROVES | NONE | HIGH | FIX |
| F7 | Reject an explicit empty delivery list[^F7] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: INCORRECT / unable to verify) |
| F8 | Expose a derived effective delivery set in config show[^F8] | CONFIRMED | RARE | NEUTRAL | MINOR | MED | DROP (Rule 1b: RARE, no readability gain) |
| F9 | Use BTreeSet instead of Vec[^F9] | CONFIRMED | RARE | NEUTRAL | MINOR | HIGH | DROP (Rule 1b: RARE, no readability gain) |
| F10 | Deduplicate DoD and explicit transition-rule messages[^F10] | CONFIRMED | RARE | NEUTRAL | MINOR | MED | DROP (Rule 1b: RARE, no readability gain) |
| F11 | ready command mismatches delivery configuration[^F11] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: INCORRECT / unable to verify) |
| F12 | Doctor should audit historical DoD compliance[^F12] | CONFIRMED | RARE | NEUTRAL | MODERATE | MED | DROP (Rule 1c: complexity without real-world impact) |

FIX: 6   FIX_WITH_CARE: 0   SPIN_OFF: 0   DISCUSS: 0   DROP: 6

## Evidence

[^F1]: Review line 9. The predicate uses `is_closing(prev_status)`, so `duplicate → fixed` returns before acceptance criteria evaluation. The existing done→wontfix test is vacuous because the target itself is not a delivery status.
[^F2]: Review line 16. `load_validated_rules` checks every transition status against the configured status universe; the example removes statuses referenced by the scaffolded matrix.
[^F3]: Review line 23. The loader validates aliases and body sections but never checks `dod.delivery_statuses`; exact string membership makes an unknown value inert. Presence-aware validation is needed to avoid rejecting inherited defaults in narrowed schemas.
[^F4]: Review line 32. The test asserts only that duplicate is ungated, which would also pass with an accidentally empty delivery list.
[^F5]: Review line 35. `acceptance_criteria_message` returns a message for missing and zero-item sections, and custom delivery statuses are supported.
[^F6]: Review line 38. `done` and `fixed` are literal strings in `default_delivery_statuses` while related built-in sets are centralized in issue_fields. Centralization reduces taxonomy drift.
[^F7]: Review line 43. An empty list is an unambiguous policy opt-out, unlike a typoed non-empty list; the moderator resolution is to allow and document it.
[^F8]: Review line 53. The two effective inputs are already inspectable. Once explicit contradictory policy is validated, another derived API key has little real-world value and adds surface.
[^F9]: Review line 60. The list is tiny and order-preserving Vec plus duplicate validation is simpler; lookup performance has no practical impact.
[^F10]: Review line 61. Both evaluators can produce similar messages, but this behavior predates the patch and the new policy narrows its occurrence. Fixing it adds coupling without demonstrated impact.
[^F11]: Review line 62. `ready` reports body checklist completion independent of a transition target. The README transition sentence is adjacent context, not a claim that ready evaluates statuses.
[^F12]: Review line 63. Doctor has no zero-config historical DoD audit, but that is pre-existing scope expansion rather than a defect in transition gating and would add a new subsystem behavior without evidence.
