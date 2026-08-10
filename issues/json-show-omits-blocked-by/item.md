---
created: 2026-08-10
updated: 2026-08-10
type: bug
status: cannot-reproduce
priority: normal
labels: [from-homebase]
closed: 2026-08-10
---

# issuectl --json show omits the blocked_by field from top-level output

## Description

## Observed
`issuectl depend add <a> --blocked-by <b>` correctly writes `blocked_by: ['@b']` to the
issue's frontmatter, BUT `issuectl --json show <a>` does not surface `blocked_by` as a
top-level key. Its key set was:
`assignee, body, closed, commits, created, epic, extra, folder, labels, owner, priority,
related, reporter, slug, status, title, type, updated, version` — no `blocked_by`.
(`{blocked_by, blockedBy, blocked}` all came back null; it appears to be buried in `extra`,
if present at all.)

## Expected
`blocked_by` is a first-class, schema-declared scheduling field (the canonical list; `blocks`
is its derived reverse view). It should appear as a top-level key in `--json show` (and
ideally the derived `blocks` too), so an agent can read the dependency graph without parsing
frontmatter. Consumers building a scheduling DAG (e.g. the homebase `/stint-*` execution-DAG,
and the new `dag-scheduling-view` feature) rely on this.

## Repro
`issuectl new "X" --slug x; issuectl new "Y" --slug y; issuectl depend add y --blocked-by x;
issuectl --json show y | jq 'has("blocked_by")'` → false (expected true).

## Resolution

### 2026-08-10T09:51:40Z · @issuectl

Already fixed in 0.7.2 via project_blocked_by projection; verified has("blocked_by")=true on current code. Report predates the 0.7.2 fix.
