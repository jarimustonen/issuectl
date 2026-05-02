---
created: 2026-05-01
updated: 2026-05-01
closed: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: done
priority: normal
related: ["#26", "#67"]
labels: [auth, session, defense-in-depth]
epic: 26
---

# 104. Filter `session::resolve` on `tenant_users.status='active' AND tenants.status='active'`

_Source: #67 v1.1 LLM review (OpenAI P0, downgraded to defense-in-depth
after verifying `disable_user` clears sessions)._

## Description

`crates/ops/src/session.rs::resolve` joins `tenant_users` to fetch the
current role for the session, but does **not** filter on
`tu.status='active' AND t.status='active'`. The resolved session
inherits whatever `role` happens to be in the row; if the membership
or tenant has been flipped to a non-active state via a path that
didn't transactionally clear the sessions, the session continues to
work.

Today this is bounded:
- `ops::user::disable_user` deletes the target user's sessions in the
  same tx as the membership flip.
- `ops::user::enable_user` doesn't write to `sessions` (no need;
  disabled users had no session to keep).
- `ops::pending_admin::apply_disable_user_inline` mirrors the
  session-clearing.
- `ops::user::update_role` does NOT clear sessions on demote — but
  the role is read fresh per request so a demoted admin's session
  loses admin powers on the next request.

So today, the only way a session outlives a `disabled` flip is if the
flip happens by direct SQL (gsadmin escape hatch, manual cleanup) or
a future flow that forgets to clear sessions.

## Required change

Add the membership-and-tenant-active filter to the canonical
resolution path:

```sql
SELECT s.user_id, s.tenant_id, tu.role, ...
FROM sessions s
JOIN users u ON u.id = s.user_id
JOIN tenants t ON t.id = s.tenant_id AND t.status = 'active'
JOIN tenant_users tu ON tu.user_id = s.user_id
                     AND tu.tenant_id = s.tenant_id
                     AND tu.status = 'active'
LEFT JOIN user_emails ue ON ...
WHERE s.id_hash = $1
```

Add an integration test:
- Create a session for a user.
- Flip `tenant_users.status` to `disabled` directly.
- `session::resolve` returns `Ok(None)`.
- Same with `tenants.status='suspended'` or `'deleted'`.

## Acceptance criteria

- `session::resolve` rejects sessions whose membership / tenant is
  no longer active.
- New test in `crates/ops/src/session.rs` (or
  `crates/server/tests/`) proves it.
- AGENTS.md note in `crates/ops/AGENTS.md` mentions the filter.
