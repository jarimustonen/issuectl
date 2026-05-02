---
created: 2026-04-30
updated: 2026-04-30
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#26", "#57", "#56"]
labels: [security, access-control, multi-tenant, design]
---

# 67. Data visibility / access-control policy — who-can-see-what

_Source: surfaced by A4b review (see `history/review-A4b-ops-extraction-and-deploy-cutover.md` §3)_

## Description

Across the codebase, scoping decisions are made ad-hoc per ops function:

- `ops::receipts::*` — strictly `(tenant_id, user_id)` scoped.
- `ops::extractions::load_extraction_summaries` — tenant-scoped; the
  docstring is honest about it but it returns rows across users in
  the same tenant for one `message_id`.
- `ops::user::list_users` — tenant-scoped (admin-only).
- `ops::user::find_user_by_email` — global lookup (no tenant scope).
- `ops::ingest::process_message` — global sender resolution.
- (Future) `agent_runs` / `agent_steps` reads (#57 #58–#62) — TBD.
- (Future) Admin "review another user's expense" UI (#22) — TBD.
- (Future) Expert reviewer cross-user oversight (#57 Phase 4) — TBD.

There is no coherent policy on **who is allowed to see what data**.
Each new function reinvents its own scoping. As more surface lands
(D-wave, web admin, expert review), the lack of a shared model will
either ossify into "tighten everything to user_id" (which breaks the
admin and reviewer use cases) or "trust callers" (which leaks).

## Goal

Produce a written access-control policy that answers, for every
domain object (tenant, user, thread, message, attachment, extraction,
receipt, expense, agent_run, agent_step, audit_event):

1. **Owner.** Who owns this row? `(tenant, user)` for most, `tenant`
   for some (e.g. `tenants.settings`), global for a few (e.g.
   `user_emails`, `tenant_users`).
2. **Default scope.** What's the minimum scope a non-admin user can
   read/write? (Almost always `tenant_id = ctx.tenant_id AND user_id
   = ctx.actor_user_id`.)
3. **Admin scope.** What can a tenant admin see beyond their own
   data? (Cross-user within the tenant? Cross-tenant never.)
4. **Approver scope.** Same question for `UserRole::Approver` (which
   exists in the schema but has no semantics today).
5. **Expert reviewer scope.** What can the #57 expert reviewer see?
   Cross-user within their assigned tenants? Cross-tenant? With
   audit-trail emission?
6. **System/automation scope.** Can `OpContext::system()` read or
   write tenant-scoped data, or is it strictly bootstrap-only?
7. **Channel-aware policy.** When does `Channel::EmailAgent` need
   web-confirmation for a write that `Channel::Web` would do
   directly?

## Out of scope

- **Implementing** all the policies. This issue produces the design;
  separate issues file the implementations per-domain.
- **External-facing access control** (API tokens, OAuth, SSO) — those
  are authentication concerns; this issue is about authorization once
  identity is established.

## Why now

- A4b shipped `load_extraction_summaries` as tenant-scoped on the
  reviewer's "tenant scope is correct for the agent's reuse path"
  argument. Reviewers (4 LLMs) split on whether this is a leak. The
  docstring captures the intent but the broader pattern is undecided.
- D-wave (#58–#62) writes `agent_runs` / `agent_steps`. The reads
  for those tables (#57 Phase 3/4) need a policy *before* writers
  ship — otherwise the reads are bolted on later.
- `#26` Phase 2 (multi-tenant käyttäjähallinta) is about to land
  registration + invitation flows; admin-vs-user distinctions get
  more teeth.
- `#22` (käyttäjähallinta admin-näkymä) implicitly assumes admins
  can see other users' summaries — that needs to be policy, not
  vibes.

## Acceptance criteria

- `issues/open/67-data-visibility-access-control/policy.md` exists,
  matrix-shaped: rows = domain object, columns = role × channel.
- For each cell: scoped read / scoped write / cross-user read /
  cross-user write / global read / forbidden.
- The matrix is reviewed (LLM review or peer) and lockable as a
  reference for future ops authors.
- A short follow-up issue is filed for each ops function that needs
  to change to match the policy. (Most should already match.)
- `crates/ops/AGENTS.md` gets a "see #67" pointer in the OpContext
  section so future authors know where the policy lives.

## Notes

The reviewer disagreement on `load_extraction_summaries` was the
trigger but the symptom is broader. A4b explicitly punted on this:
the function stays tenant-scoped, the docstring will be made
explicit ("tenant-scoped on purpose"), and #67 owns the bigger
question.

Suggested process: write the matrix first as a strawman, run
`/llm-review` on it (this is the kind of policy-doc LLMs critique
well), then revise. Estimated effort: 1–2 days for the design,
unknown for follow-up implementations (depends on how many
mismatches the matrix exposes).
