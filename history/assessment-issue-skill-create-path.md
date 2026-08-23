# Assessment: shipped `/issue` create-path guidance

Source: [`history/review-issue-skill-create-path.md`](review-issue-skill-create-path.md)

| # | Finding | Conf | Like | Read | Arch | Confidence | Recommendation |
|---|---|---|---|---|---|---|---|
| F1 | Claude and Codex contract bodies were not pinned to each other[^f1] | CONFIRMED | OCCASIONAL | IMPROVES | MINOR | HIGH | FIX |
| F2 | Prose assertions were sensitive to Markdown wrapping[^f2] | CONFIRMED | OCCASIONAL | IMPROVES | NONE | HIGH | FIX |
| F3 | Absent-versus-null scheduling echo lacked a black-box clear case[^f3] | CONFIRMED | OCCASIONAL | IMPROVES | NONE | HIGH | FIX |
| F4 | DAG each-row blocker claim lacked empty-row assertions[^f4] | CONFIRMED | OCCASIONAL | IMPROVES | NONE | HIGH | FIX |
| F5 | Filesystem guidance encouraged layout coupling[^f5] | CONFIRMED | OCCASIONAL | IMPROVES | MINOR | HIGH | FIX |
| F6 | Add `blocked_by` to update echoes[^f6] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect) |
| F7 | Normalize show and DAG blocker sigils[^f7] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect) |
| F8 | Duplicate every scheduling echo case as black-box coverage[^f8] | INCORRECT | — | — | — | MED | DROP (Rule 1a: incorrect) |
| F9 | Document create all-three scheduling echoes in this patch[^f9] | CONFIRMED | RARE | WORSENS | MINOR | MED | DROP (Rule 1b: rare, no readability gain) |
| F10 | Add a concurrency caveat for update followed by show[^f10] | CONFIRMED | RARE | WORSENS | NONE | MED | DROP (Rule 1b: rare, no readability gain) |

**FIX: 5   FIX (with care): 0   SPIN-OFF: 0   DISCUSS: 0   DROP: 5**

All five FIX findings were applied before this assessment was persisted. No SPIN-OFF issue is warranted.

[^f1]: Review lines 13–18.
[^f2]: Review lines 20–25.
[^f3]: Review lines 27–32.
[^f4]: Review lines 34–39.
[^f5]: Review lines 41–46.
[^f6]: Review line 50.
[^f7]: Review line 51.
[^f8]: Review line 52.
[^f9]: Review line 53.
[^f10]: Review line 54.
