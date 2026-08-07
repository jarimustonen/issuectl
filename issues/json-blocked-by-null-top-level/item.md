---
created: 2026-08-06
updated: 2026-08-07
type: bug
status: fixed
priority: normal
commits:
- hash: 857d8df
  summary: surface blocked_by top-level on ls/search via shared project_blocked_by helper; regression test
- hash: 438c703
  summary: apply /llm-review — search --json test, ls canonicalization/scalar/extra-survival tests, panic-msg fix, docstring hash note
closed: 2026-08-07
---

# JSON show/ls buries blocked_by under extra; top-level .blocked_by is always null

## Description

Observed with issuectl 0.7.1. blocked_by is documented as the canonical frontmatter list (see 'issuectl depend' help) and is stored top-level in item.md frontmatter (e.g. blocked_by: ['@sv-0-repo-datakoti']). But JSON serialization exposes it inconsistently:

  issuectl --json show <slug> | jq '{top: .blocked_by, extra: .extra}'
  => {top: null, extra: {blocked_by: ['@sv-0-repo-datakoti']}}

Same for --json ls. EXPECTED: top-level .blocked_by carries the list (mirroring the frontmatter and the other list fields like .related/.labels). ACTUAL: top-level .blocked_by is always null; the real value is buried in .extra.blocked_by. This silently fools programmatic consumers reading .blocked_by (a jq check of a depend-add chain reads null and looks like the link failed, when it actually succeeded). Fix: serialize blocked_by as a top-level JSON field. Repro: create two issues, 'issuectl depend add B --blocked-by A', then 'issuectl --json show B'.
