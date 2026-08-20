---
created: 2026-05-06
updated: 2026-08-20
type: improvement
status: wontfix
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [discuss, reporting, v0.6.0-candidate]
closed: 2026-08-12
---

# Per-issue events.jsonl log (alternative to git history for metrics)

## Description

Optional append-only normalized event log: issues/<slug>/events.jsonl with entries like {time, actor, op, field, from, to}. Gives precise status/transition history immune to rebases and squashes. Risk: extra file churn, more git noise, two sources of truth. DO NOT enable by default. Build only if @considerably-wide-mass (git-derived activity/timeline) proves insufficient for real metrics. Discuss before building.
