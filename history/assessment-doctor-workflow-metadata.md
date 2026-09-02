# Assessment: doctor workflow metadata registration

Source: [`history/review-doctor-workflow-metadata.md`](review-doctor-workflow-metadata.md) · HEAD `71eecca46dfefb69608148812e14ba23cec81b4f`

| # | Finding | Conf | Like | Read | Arch | Confidence | Recommendation |
|---|---|---|---|---|---|---|---|
| F1 | Workflow metadata docs misstate enum-free strings as open-valued[^f1] | CONFIRMED | OCCASIONAL | IMPROVES | NONE | HIGH | FIX |
| F2 | Schema installation test does not inspect installed bytes[^f2] | CONFIRMED | OCCASIONAL | IMPROVES | NONE | HIGH | FIX |
| F3 | Workflow metadata extra-field projection lacks a named regression[^f3] | CONFIRMED | RARE | IMPROVES | NONE | HIGH | FIX |

FIX: 3   FIX (with care): 0   SPIN-OFF: 0   DISCUSS: 0   DROP: 0

All three fixes were clean, localized changes and were applied during this run. No follow-up issue is warranted.

[^f1]: Review lines 13–18; confirmed against scalar validation in `schema.rs` and the authoritative string-valued `--field` writer contract.
[^f2]: Review lines 20–25; confirmed because `schema::load` layers built-in defaults before returning the effective schema.
[^f3]: Review lines 27–32; confirmed by parser flattening into `Issue.extra` and the absence of a workflow-specific parser assertion before review fixes.
