---
created: 2026-05-07
updated: 2026-08-04
type: chore
status: open
priority: high
labels: [deferred]
---

# Cache canonical_hash to avoid recomputing on every /api/issues call

_Source: src/repo.rs, src/server/api.rs_

## Description

From<Issue> for IssueSummary calls canonical_hash for every issue on every /api/issues request. Repos with hundreds of issues already exist; SSE-driven load() amplifies this. Fix: cache version per item.md path keyed by mtime+size in AppState, or compute version once during parsing and thread it through Issue. Spin-off from drag-and-drop write-back review (history/review-needlessly-fluffy-decision-dnd.md, finding #3).
