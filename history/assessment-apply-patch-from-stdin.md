# Assessment: apply-patch-from-stdin

Source: `history/review-apply-patch-from-stdin.md`  
HEAD assessed: `9e75f694208b6e3c7e0dde0b8afc35db1dfa479d`

| # | Finding | Conf | Like | Read | Arch | Confidence | Recommendation |
|---|---|---|---|---|---|---|---|
| F1 | Path-shape inference misclassifies inputs[^1] | CONFIRMED | REGULAR | IMPROVES | MINOR | HIGH | FIX |
| F2 | Shared errors are surface-specific and duplicative[^2] | CONFIRMED | OCCASIONAL | IMPROVES | NONE | HIGH | FIX |
| F3 | Classifier and source regression tests are too narrow[^3] | CONFIRMED | REGULAR | NEUTRAL | MINOR | HIGH | FIX |
| F4 | `json_output` boolean obscures expected-version policy[^4] | CONFIRMED | OCCASIONAL | IMPROVES | MINOR | HIGH | FIX |
| F5 | TTY blocking and unbounded stdin[^5] | CONFIRMED | RARE | WORSENS | MINOR | HIGH | DROP (Rule 1d: worsens readability and is not regular) |
| F6 | Patch failures lack specific JSON error codes[^6] | CONFIRMED | RARE | NEUTRAL | MODERATE | MED | DROP (Rule 1c: broader complexity without demonstrated impact) |
| F7 | Test-only parser forwarding shim[^7] | CONFIRMED | RARE | NEUTRAL | NONE | HIGH | DROP (Rule 1b: rare, no readability gain) |
| F8 | Module and skill artifacts were allegedly missing[^8] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect) |
| F9 | Literal dash path allegedly fails on Windows[^9] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect) |

FIX: 4   FIX (with care): 0   SPIN-OFF: 0   DISCUSS: 0   DROP: 5

[^1]: `history/review-apply-patch-from-stdin.md:9`
[^2]: `history/review-apply-patch-from-stdin.md:18`
[^3]: `history/review-apply-patch-from-stdin.md:21`
[^4]: `history/review-apply-patch-from-stdin.md:24`
[^5]: `history/review-apply-patch-from-stdin.md:29`
[^6]: `history/review-apply-patch-from-stdin.md:34`
[^7]: `history/review-apply-patch-from-stdin.md:39`
[^8]: `history/review-apply-patch-from-stdin.md:46`
[^9]: `history/review-apply-patch-from-stdin.md:47`
