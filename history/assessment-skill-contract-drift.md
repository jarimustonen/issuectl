# Assessment: generated issue skill contract alignment

Source: [`history/review-skill-contract-drift.md`](review-skill-contract-drift.md)

| # | Finding | Conf | Like | Read | Arch | Confidence | Recommendation |
|---|---|---|---|---|---|---|---|
| F1 | Alternative-heading analysis can respawn across invocations [^1] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: corrected before assessment) |
| F2 | Settlement failure states are under-specified [^2] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: corrected before assessment) |
| F3 | Landing semantics treat every false value as unknown [^3] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: corrected before assessment) |
| F4 | Close layout still implies legacy paths [^4] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: corrected before assessment) |
| F5 | Issuectl should recognize both analysis headings [^5] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: outside the focused caller contract) |
| F6 | Issuectl intake JSON paths are unwrapped [^6] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: disproved empirically) |
| F7 | Generated skill copies are absent [^7] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: disproved by diff and tests) |
| F8 | Branch reverts broken-pipe work [^8] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: two-dot diff artifact) |
| F9 | Taskfleet version-specific prose creates maintenance pressure [^9] | CONFIRMED | RARE | NEUTRAL | MINOR | HIGH | DROP (Rule 1b: future-only drift, no clarity gain now) |

FIX: 0   FIX (with care): 0   SPIN-OFF: 0   DISCUSS: 0   DROP: 9

No assessment row requires an in-branch change or a staged spin-off command. The separately observed Taskfleet heading-contract mismatch is tracked through intake as required by this run's cross-repository follow-up policy.

[^1]: [`history/review-skill-contract-drift.md:12`](review-skill-contract-drift.md#critical-issues-consensus)
[^2]: [`history/review-skill-contract-drift.md:19`](review-skill-contract-drift.md#critical-issues-consensus)
[^3]: [`history/review-skill-contract-drift.md:26`](review-skill-contract-drift.md#critical-issues-consensus)
[^4]: [`history/review-skill-contract-drift.md:33`](review-skill-contract-drift.md#critical-issues-consensus)
[^5]: [`history/review-skill-contract-drift.md:42`](review-skill-contract-drift.md#disputed-or-incorrect-findings)
[^6]: [`history/review-skill-contract-drift.md:47`](review-skill-contract-drift.md#disputed-or-incorrect-findings)
[^7]: [`history/review-skill-contract-drift.md:51`](review-skill-contract-drift.md#disputed-or-incorrect-findings)
[^8]: [`history/review-skill-contract-drift.md:55`](review-skill-contract-drift.md#disputed-or-incorrect-findings)
[^9]: [`history/review-skill-contract-drift.md:62`](review-skill-contract-drift.md#minor-findings)
