---
created: 2026-05-06
updated: 2026-05-06
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, reporting, discuss]
---

# Per-issue events.jsonl log (alternative to git history for metrics)

## Description

Optional append-only normalized event log: issues/<slug>/events.jsonl with entries like {time, actor, op, field, from, to}. Gives precise status/transition history immune to rebases and squashes. Risk: extra file churn, more git noise, two sources of truth. DO NOT enable by default. Build only if @considerably-wide-mass (git-derived activity/timeline) proves insufficient for real metrics. Discuss before building.
