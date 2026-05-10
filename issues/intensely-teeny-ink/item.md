---
created: 2026-05-10
updated: 2026-05-10
type: feature
reporter: jari
assignee: jari
status: open
priority: high
epic: hugely-exciting-spiders
---

# Custom boards: user-defined views with configurable column grouping (epic, label, custom field) for triage and planning

## Description

Today the kanban has one fixed grouping (status). For release planning and triage we need user-defined boards where columns can group on a different axis — typically epic (drag to assign issue to v0.6, future, unscoped) or label, but generally any field that has a small bounded value set.

Concrete near-term need: drag open issues into 0.6 / future buckets without editing each item.md. Drop = update epic: frontmatter (or label, or other field).

Design questions to resolve:
- Where do board definitions live? Probably .issuectl/boards/<name>.yaml so they're committed and reviewable.
- What is the schema? Minimum: name, group_by (field name), columns (list of values + optional 'unassigned' bucket), optional filter (reuse the query-engine syntax we already have — that's the 'logic' without inventing a DSL).
- Drag semantics: dropping into column X writes the column's value to the group_by field via the existing PATCH path (flock + expected_version).
- URL: /board/<name>; default board (status) stays at /.
- Read-only fallback if a board references a missing field or invalid query.

Stretch:
- 'Saved views' menu in the header, surfaced from the same definition file.
- Per-board card display options (which fields to show on the card).
