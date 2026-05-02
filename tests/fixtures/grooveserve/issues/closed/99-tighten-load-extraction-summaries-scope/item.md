---
created: 2026-05-01
updated: 2026-05-01
closed: 2026-05-01
type: task
reporter: jari
assignee: jari
status: done
priority: high
related: ["#67", "#63"]
labels: [security, ops, multi-tenant, access-control]
epic: 56
commits:
  - hash: 196788d
    summary: "fix(ops): tighten load_extraction_summaries to own-scope (#99)"
  - hash: 48e14ab
    summary: "docs(issues): close #99 extraction-scope tightening, update epic #56"
---

# 99. Tighten `load_extraction_summaries` scope to `(tenant_id, user_id, message_id)`

_Continues: #67 §3 follow-up_

## Description

`grooveserve_ops::extractions::load_extraction_summaries` is currently
tenant-scoped: it returns every extraction row matching `(tenant_id,
message_id)` regardless of which user owns the row. The MVP single-tenant
invariant made the docstring-honest behaviour appear safe, and #67 v1
strawman reproduced the same reasoning.

**The v1 LLM review (gpt-5.5 P0) overrode that.** Even under MVP
single-tenant, two users in the same tenant can collide on `message_id` —
RFC 5322 `Message-ID` is *not* an authorization boundary, and the current
function would hand User B's vision-OCR rows to User A on the wrong
collision. The collision is unlikely in practice but the access-control
guarantee shouldn't lean on a probabilistic header field.

The locked #67 policy (v1.1) lists this row as `own`-scoped: caller must
pass `(tenant_id, user_id)` and the SQL must filter on both.

## Required change

Add `user_id: i64` parameter to `load_extraction_summaries` and append
`AND user_id = $X` to the WHERE clause. Existing call sites
(`crates/server/src/ingest/pipeline.rs` retry path) already have a
resolved `user_id` — wire it in.

```rust
WHERE tenant_id = $1
  AND user_id = $2
  AND message_id = $3
```

Update tests to cover the cross-user-same-message-id case (insert two
rows with the same `message_id` but different `user_id`, assert each
caller sees only their own).

## Why now

- Locked #67 policy mandates this scope as `own`. Drift from the
  matrix is filed as follow-up issues; this is the first such follow-up.
- The risk window grows once any new caller touches this fn (Phase 4
  expert UI under #57, web-side "did this email get processed?" pages
  under #11).
- Multi-tenant membership (#63) makes the bug worse but the bug exists
  today.

## Acceptance criteria

- `load_extraction_summaries` takes `(tenant_id, user_id, message_id)`.
- Existing call site updated to pass `user_id`.
- New sqlx test: two users, same `message_id`, each caller sees only
  their row.
- AGENTS.md note in `crates/ops/AGENTS.md` updated.
- #67 §3 follow-up bullet checked off (or replaced with "done" pointer).

## Quick Test

```rust
// Insert extraction rows for two users with same message_id
// Caller A: load_extraction_summaries(tenant, user_a, msg) → 1 row, owner=A
// Caller B: load_extraction_summaries(tenant, user_b, msg) → 1 row, owner=B
```
