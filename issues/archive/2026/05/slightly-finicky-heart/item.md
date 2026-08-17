---
created: 2026-05-06
updated: 2026-05-09
type: feature
status: done
priority: high
reporter: jari
assignee: jari
epic: exorbitantly-ill-apples
labels: [doctor, foundation, validation]
commits:
- hash: eccce15
  summary: 'feat(doctor): full validation suite + installable git hooks'
- hash: 3def11d
  summary: 'fix(doctor,hooks): apply review findings from llm-review'
- hash: 03ec84c
  summary: 'fix(doctor,hooks): apply round-2 review findings'
closed: 2026-05-09
---

# issuectl doctor: full validation suite + installable git hooks

_Source: src/cli/doctor.rs, src/schema.rs (new), .githooks/ (new)_

## Description

Extend doctor beyond today's checks into a full repo-integrity command: invalid YAML, missing required fields, unknown enums (status/type/priority), broken slug refs (epic/related/blocked_by), dependency cycles, status/closed consistency, duplicate slugs, slug sanity, timestamp sanity. Add 'issuectl hooks install' that wires a pre-commit hook running doctor on changed issue files. Subsumes the worktree spin-off @amazingly-scattered-month (startup reconciliation rules — status/folder mismatches, orphan tempfiles, merge markers, ambiguous slugs). Foundation: every later feature in the epic assumes parsable, consistent files.
