---
created: 2026-05-06
updated: 2026-05-06
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, workflow]
---

# Recurring / scheduled issues (cron-driven, materialize new file per occurrence)

## Description

Recurrence definitions in .issuectl/recurrences/<name>.yaml (title, schedule cron expression, template, labels, assignee). 'issuectl schedule run' materializes a new issue per due occurrence with 'recurrence_of: <template>' and 'occurrence: <key>'. Manifest prevents duplicates. Architectural decision (unanimous in brainstorm): materialize new file per occurrence, never overwrite an 'active instance' — preserves git history. Use case: weekly dependency updates, monthly chores, periodic reviews.
