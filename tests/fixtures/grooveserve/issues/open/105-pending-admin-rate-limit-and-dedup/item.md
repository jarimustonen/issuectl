---
created: 2026-05-01
updated: 2026-05-01
type: feature
reporter: jari
assignee: jari
status: open
priority: high
related: ["#26", "#67"]
labels: [security, rate-limit, abuse-control, multi-tenant]
epic: 26
---

# 105. Rate limit + idempotency on `pending_admin_actions` creation

_Source: #67 v1.1 LLM review consensus (4/4 reviewers): no rate
limit on agent-driven pending-row creation is a DoS / abuse vector._

## Description

The four EmailAgent admin tools (`invite_user`, `enable_user`,
`disable_user`, `update_user_role`) call
`ops::pending_admin::create_pending` with no per-tenant or per-actor
throttle. A prompt-injection loop or a compromised admin mailbox can
flood `pending_admin_actions` with thousands of rows and the tenant
admin's mailbox with thousands of "please confirm" prompts.

The 24-hour TTL alone doesn't mitigate burst behaviour. There is also
no expiry sweeper today (#106), so rows accumulate as `pending` past
`expires_at`.

## Required change — three layers

### 1. Per-actor / per-tenant rate limit

A token-bucket or sliding-window counter, keyed on
`(tenant_id, requested_by_user_id, action_type)`. Soft cap, e.g.
10 pending rows per actor per hour, 100 per tenant per hour.

Returns a new `OpError::RateLimited { retry_after_secs }` (already
exists for onboarding-resend; reuse). Tool wrappers map this to a
clear "olet pyytänyt liian monta hallintotoimea, yritä myöhemmin
uudelleen" message.

Implementation choice: in-memory via a `dashmap` of buckets per
process is simplest but doesn't survive restart; DB-backed counters
are persistent but cost a write per check. Pick based on prod
deployment shape (single binary today → in-memory is fine).

### 2. Partial unique index for idempotency

Suppress duplicate identical pending rows at the schema level:

```sql
CREATE UNIQUE INDEX uniq_pending_admin_action
    ON pending_admin_actions (tenant_id, action, target_id)
    WHERE status = 'pending' AND target_id IS NOT NULL;
```

`invite_user` (target_id NULL) needs a different dedup approach —
unique on `(tenant_id, action, payload->>'email')` partial.

Tool wrappers translate the conflict into a helpful "Toimenpide on
jo odottamassa vahvistusta" response.

### 3. Per-tenant cap on outstanding pendings

Hard cap (e.g. 50 outstanding pending rows per tenant) so a
sustained flood doesn't bloat the table even if the rate limit
fails. Returns `OpError::RateLimited`.

## Acceptance criteria

- New `ops::pending_admin` rate-limit check, configurable thresholds.
- Migration adds the partial unique index.
- Tool wrappers translate `OpError::RateLimited` to a Finnish
  user-facing message.
- Tests: burst of 11 pendings → 11th rejected; duplicate
  `disable_user` for same target → second rejected with `Conflict`
  or `RateLimited`; per-tenant cap reached → reject.
- AGENTS.md note describing the limits.

## Why high priority

- The DoS vector is reachable via a single inbound email if the
  agent is in a confused / hallucinating state.
- Mitigation prevents accidental admin mailbox burnout.
