# Assessment: Broken pipe handling review

Source: [`history/review-broken-pipe.md`](review-broken-pipe.md)

| # | Finding | Conf | Like | Read | Arch | Confidence | Recommendation |
|---|---|---|---|---|---|---|---|
| F1 | The first regression test was racy[^1] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: no longer present) |
| F2 | Immediate process exit could abort work after output[^2] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: no longer present) |
| F3 | Non-BrokenPipe stdout failures lacked coverage[^3] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: coverage added) |
| F4 | Text output lacked BrokenPipe coverage[^4] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: coverage added) |
| F5 | Prefer SIGPIPE-style exit 141[^5] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: contradicts accepted contract) |
| F6 | Suppression can continue read-only computation after EPIPE[^6] | CONFIRMED | RARE | WORSENS | MODERATE | MED | DROP (Rule 1b: rare, no readability gain) |

**FIX: 0   FIX (with care): 0   SPIN-OFF: 0   DISCUSS: 0   DROP: 6**

[^1]: [`history/review-broken-pipe.md:9`](review-broken-pipe.md#L9), all four reviewers.
[^2]: [`history/review-broken-pipe.md:16`](review-broken-pipe.md#L16), all four reviewers.
[^3]: [`history/review-broken-pipe.md:25`](review-broken-pipe.md#L25), OpenAI and DeepSeek.
[^4]: [`history/review-broken-pipe.md:28`](review-broken-pipe.md#L28), OpenAI, Claude, and DeepSeek.
[^5]: [`history/review-broken-pipe.md:33`](review-broken-pipe.md#L33), Gemini.
[^6]: [`history/review-broken-pipe.md:38`](review-broken-pipe.md#L38), Gemini.

No findings remain to apply or spin off. Every review claim was rechecked against the current tree; the only surviving trade-off lacks observed real-world impact and would require a broader, less readable command-class policy.
