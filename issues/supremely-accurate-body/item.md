---
created: 2026-05-06
updated: 2026-05-06
type: improvement
status: open
priority: normal
---

# Field-level merge for commuting metadata PATCHes (build only if M2 shows real 409 friction)

## Description

Spin-off from web-edit-sync design (docs/design/web-edit-sync.md §11).

The M1 design uses whole-file canonical hash for optimistic concurrency. This means independent metadata changes (tab A changes assignee, tab B changes priority — same starting hash) cause a spurious 409 even though the changes commute.

Mitigation: server-side field-level merge. When a PATCH arrives, compare the touched fields' on-disk values to the client's expected base. If only untouched fields have changed, accept the PATCH and recompute the hash. If the touched fields themselves have changed, return 409 as today.

Two implementation shapes:
1. Per-field hashes in IssueSummary (more wire data, simpler check).
2. Client sends base values for the fields it's mutating (smaller server state, larger PATCH).

Edge cases to think through:
- add/remove array semantics under concurrent edits (already intent-preserving, but interaction with merge needs care).
- body PATCHes are excluded — body conflicts always 409 by design (M2 conflict UI handles them).
- Interaction with --expected-version on the CLI: probably stays whole-file there.

DO NOT BUILD ahead of demand. Reviewers were split on whether this is worth the complexity. Wait until M2 ships and real-world reports show 409 friction is more than an annoyance. localStorage-backed body editing already prevents data loss; metadata 409s just cost a click to retry.

If this is never built, the design degrades to 'last writer wins, surfaced as 409 to one of them' — acceptable for a single-user local tool.
