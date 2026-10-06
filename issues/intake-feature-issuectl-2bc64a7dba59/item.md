---
created: 2026-10-06
updated: 2026-10-06
type: feature
reporter: jari
status: untriaged
priority: normal
provenance: agent:3dbear-monorepo
source_ref: agent:3dbear-monorepo/reporter:jari/id:3dbear-status-summary-preserve-updated-20261006
---

# Allow summary-only custom field updates without touching updated

## Description

Allow summary-only custom field updates without touching updated

In 3dbear-monorepo a human-maintained `status_summary` custom field describes the current state of every issue, but changing only this description with `issuectl set <slug> status_summary …` resets the reserved `updated:` date to today. A backfill across ~1100 issues consequently made nearly every ticket look freshly updated in the dashboard even though none of the work progressed. `issuectl set <slug> updated <old-date>` is refused (built-in reserved); `issuectl update --patch-file` also rejects `updated`, so there is no safe CLI route for a summary-only edit that preserves it. Request a first-class guarded way to update non-progress/descriptive custom fields without touching `updated` (e.g. opt-in `--preserve-updated` for `set`/`update`, or a declared per-field policy). Default behavior for lifecycle/content changes should continue to stamp today. Dry-run, optimistic version and lock behavior must remain intact; regression test should prove a `status_summary`-only edit preserves an earlier date and a status transition stamps the new date. For now the caller must restore dates from git history and use a repository-local locked summary-only updater as a workaround.
