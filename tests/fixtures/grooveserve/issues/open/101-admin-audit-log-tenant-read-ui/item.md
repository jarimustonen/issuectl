---
created: 2026-05-01
updated: 2026-05-01
type: feature
reporter: jari
assignee: jari
status: in-progress
priority: normal
related: ["#67", "#57", "#22"]
labels: [admin, audit, ui]
epic: 26
---

# 101. Admin audit-log read API + UI (`audit_events` tenant-scoped)

_Source: #67 §3 follow-up — locked matrix grants admin Web `tenant`
read on `audit_events` but no read API/UI exists yet._

## Description

The locked #67 access-control policy grants tenant admins
`tenant`-scoped read access to `audit_events` (forensic visibility:
"who changed Matti's role last week?"). Today there is no read API
or UI for this — `audit_events` is write-only from ops::* and only
queryable via direct SQL.

This issue tracks adding:
- `ops::audit::list_events(ctx, filters) -> Vec<AuditEventRow>` —
  tenant-scoped list with filter support (action, target_type,
  date range, actor).
- `/admin/audit` page — paginated table.
- Possibly tie into #57 §6 expert-reviewer view.

## Out of scope

- Cross-tenant audit (expert reviewer): tracked in #57 Phase 4.
- Audit-event mutation: events are append-only; no update/delete UI.
- API tokens / OAuth audit (deferred until external auth lands).

## Acceptance criteria

- `ops::audit::list_events` exists with `(tenant_id)` scope.
- `/admin/audit` lists last N rows with filters.
- Tests: tenant isolation, admin-only gate, filter-by-action.
- AGENTS.md updated.
