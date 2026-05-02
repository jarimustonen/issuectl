---
created: 2026-05-01
updated: 2026-05-01
type: task
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#67"]
labels: [api, auth, design, future]
---

# 102. API-token channel + access-control policy (placeholder)

_Source: #67 §4.3 follow-up — placeholder for when external API
tokens land._

## Description

When external API token authentication is added (Phase 4+), it
introduces a fifth channel `Channel::ApiKey` with its own row in the
#67 matrix. Typical scoping:
- `tenant (read + write)` for an admin-issued integration token,
- scoped further by the token's permitted op list.

This issue is a **design placeholder**, not an active task. File no
work here until external API tokens are on the roadmap. When they
are:
1. Add `Channel::ApiKey` to `crates/ops/src/context.rs`.
2. Update #67 matrix with a new column.
3. Mint/revoke ops + management UI.
4. Per-token scope/permission model.

## Acceptance criteria

When this is taken up, success criteria are defined at that time —
this issue exists so #67's §4.3 has a concrete pointer.
