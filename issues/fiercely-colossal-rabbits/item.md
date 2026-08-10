---
created: 2026-05-07
updated: 2026-08-10
type: chore
status: obsolete
priority: high
labels: [deferred]
closed: 2026-08-10
---

# Cache canonical_hash to avoid recomputing on every /api/issues call

_Source: src/repo.rs, src/server/api.rs_

## Description

From<Issue> for IssueSummary calls canonical_hash for every issue on every /api/issues request. Repos with hundreds of issues already exist; SSE-driven load() amplifies this. Fix: cache version per item.md path keyed by mtime+size in AppState, or compute version once during parsing and thread it through Issue. Spin-off from drag-and-drop write-back review (history/review-needlessly-fluffy-decision-dnd.md, finding #3).

## Resolution

### 2026-08-10T10:03:40Z · @issuectl

Web/browser UI is being removed from issuectl (product decision 2026-08-10). This is a web-board enhancement, so it is obsolete. See @remove-web-ui.
