---
created: 2026-05-01
updated: 2026-05-01
type: bug
reporter: jari
assignee: jari
status: fixed
priority: high
closed: 2026-05-01
related: ["#67"]
labels: [tools, schema, multi-tenant, agent]
epic: 26
commits:
  - hash: 16a4d22
    summary: "fix(server): rewrite get_user_context + profile-snapshot helpers for A3 schema (#100)"
  - hash: a3ef8a4
    summary: "fix(server): extract audit helper, add status filter, add language to get_user_context"
  - hash: 9fc81f2
    summary: "fix(server): lock tu before profile, reject empty prefs, fix error handling"
---

# 100. Rewrite `get_user_context` + profile-snapshot helpers for A3 schema

_Source: discovered during #67 policy review (post-A3 schema audit)._

## Description

`crates/server/src/tools/user/get_user_context.rs` and the
profile-snapshot helpers in `crates/server/src/tools/user/util.rs`
(`load_profile_snapshot_tx`, `ensure_profile_exists_tx`) reference
columns that **do not exist** in the post-A3 schema:

```rust
"SELECT u.email, u.name, u.role, ...
 FROM users u
 LEFT JOIN user_profiles p ON p.user_id = u.id
 WHERE u.id = $1 AND u.tenant_id = $2"   // u.tenant_id doesn't exist
```

After the A3 normalisation:

- `users` has `id, name, password_hash, locale, created_at, updated_at` —
  no `email`, no `role`, no `tenant_id`.
- Membership lives in `tenant_users`.
- Email lives in `user_emails`.

The tool is broken for any caller; sqlx will fail at first invocation
with `column "email" does not exist`. Same for `load_profile_snapshot_tx`
and `ensure_profile_exists_tx`.

## Required change

Rewrite each helper to JOIN through `tenant_users` (scoped on
`(ctx.tenant_id, ctx.user_id)`), pull `email` from
`user_emails WHERE is_primary`, and pull `role` from `tenant_users`.

Per the locked #67 matrix:
- `get_user_context` — `own (read)` for the actor's own profile only.
- `load_profile_snapshot_tx` — `own (read)`; same scope as above.
- `ensure_profile_exists_tx` — `own (write)`; only inserts when the
  membership exists.

## Acceptance criteria

- All three functions compile against the current schema.
- Tests cover: success path, cross-tenant rejection (returns None),
  cross-user same-tenant rejection (returns None for the wrong user).
- AGENTS.md note in `crates/server/AGENTS.md` if the tool's behavior
  changes user-visibly.
