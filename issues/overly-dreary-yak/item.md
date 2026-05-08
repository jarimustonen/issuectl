---
created: 2026-05-06
updated: 2026-05-08
type: feature
status: in-progress
priority: normal
reporter: jari
assignee: jari
epic: exorbitantly-ill-apples
labels: [agent-friendly, v0.6.0-candidate, workflow]
commits:
- hash: 39cabc5
  summary: body section conventions + note CLI + reopen notes stub
- hash: 6f4485f
  summary: 'body-sections: fence-aware parsing, validate inputs, drop Notes alias'
- hash: 3526f73
  summary: create spin-off virtually-dull-regret
related: ['@virtually-dull-regret', '@totally-placid-push']
---

# Standardized markdown sections: comments, decisions, agent runs, reopen notes

## Description

Append-only markdown body conventions, parsed and writeable by the safe-mutation CLI. (a) '## Comments' / '## Notes' — 'issuectl note <slug> --as alice "..."' appends a timestamped block. (b) '## Decisions' — record architectural choices so agents avoid re-litigating them. (c) '## Agent Runs' — auto-appended audit trail of agent attempts (branch, result, tests, commits). (d) Reopen flow: when a closed issue is reopened, auto-append '## Reopen Notes — <date>' with rationale prompt. All zero-schema; markdown stays human-readable, but the tool gives it shape.
