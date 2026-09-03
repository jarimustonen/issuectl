## Assessment: release bump dogfood regeneration

| # | Finding | Conf | Like | Read | Arch | Confidence | Recommendation |
|---|---|---|---|---|---|---|---|
| F1 | Production binary override could render stale skills [^1] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: fixed before assessment) |
| F2 | Hard-coded Cargo artifact path breaks configured targets [^2] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: fixed before assessment) |
| F3 | Hook test skipped the production Cargo boundary [^3] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: fixed before assessment) |
| F4 | Generated skill versions were not checked [^4] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: fixed before assessment) |
| F5 | Cargo invocation relied on caller CWD [^5] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: fixed before assessment) |
| F6 | Broad installer could alter issues/AGENTS.md [^6] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: fixed before assessment) |
| F7 | Operator-home assertions missed managed paths [^7] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: fixed before assessment) |
| F8 | Signal trap did not explicitly terminate [^8] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: fixed before assessment) |
| F9 | Release contract assertion was textual [^9] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: fixed before assessment) |
| F10 | Workspace-version extraction is unscoped [^10] | CONFIRMED | OCCASIONAL | IMPROVES | NONE | HIGH | FIX |
| F11 | Toolchain-home preservation is not asserted [^11] | CONFIRMED | OCCASIONAL | IMPROVES | NONE | HIGH | FIX |
| F12 | Real Cargo execution is not automated [^12] | CONFIRMED | RARE | WORSENS | MINOR | HIGH | DROP (Rule 1b: rare, no readability gain) |
| F13 | Dogfood path inventory is duplicated [^13] | CONFIRMED | RARE | WORSENS | MODERATE | HIGH | DROP (Rule 1b: rare, no readability gain) |
| F14 | Missing-scaffold refusal lacks direct coverage [^14] | CONFIRMED | OCCASIONAL | IMPROVES | NONE | HIGH | FIX |
| F15 | Immediate byte equality contradicts development-time drift tolerance [^15] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: release-specific invariant) |

**FIX: 3   FIX (with care): 0   SPIN-OFF: 0   DISCUSS: 0   DROP: 12**

[^1]: `history/review-release-bump-dogfood.md:9`
[^2]: `history/review-release-bump-dogfood.md:15`
[^3]: `history/review-release-bump-dogfood.md:20`
[^4]: `history/review-release-bump-dogfood.md:25`
[^5]: `history/review-release-bump-dogfood.md:32`
[^6]: `history/review-release-bump-dogfood.md:33`
[^7]: `history/review-release-bump-dogfood.md:34`
[^8]: `history/review-release-bump-dogfood.md:35`
[^9]: `history/review-release-bump-dogfood.md:36`
[^10]: `history/review-release-bump-dogfood.md:40`
[^11]: `history/review-release-bump-dogfood.md:45`
[^12]: `history/review-release-bump-dogfood.md:49`
[^13]: `history/review-release-bump-dogfood.md:53`
[^14]: `history/review-release-bump-dogfood.md:57`
[^15]: `history/review-release-bump-dogfood.md:61`
