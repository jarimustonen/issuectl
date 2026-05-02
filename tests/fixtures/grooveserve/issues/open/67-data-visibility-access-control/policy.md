# Data visibility / access-control policy

_Issue: #67 | Status: locked v1.1 (post-LLM-review) | Created: 2026-05-01 | Updated: 2026-05-01 | Author: jari_

## Purpose

Canonical reference for "who can see what data" across `crates/ops`.
Every domain object gets a row; every row says, for each role × channel
combination, whether reads and writes are allowed and at what scope.

This is the policy `crates/ops/AGENTS.md` points at when an op author
asks "should this be tenant-scoped or user-scoped?" — the answer lives
here, not in the function-by-function commit history.

## Reading this document

The matrix in §2 is the policy. §1 defines the terms it uses. §3 tracks
follow-up issues for ops functions whose current behaviour does not
match the policy. §4 records the open questions the strawman has not
yet locked.

The policy is **MVP-scoped**: it reflects the single-tenant invariant,
the `admin / user / approver` role catalog, and the four channels we
actually run today (`Web`, `EmailAgent`, `EmailIngest`, `Internal`).
Multi-tenant membership (#63), expert reviewer (#57 Phase 4), and
external API tokens are explicitly out of scope and are listed as
follow-ups in §4.

## 1. Vocabulary

### 1.1 Roles

The catalog `crates/ops/src/context.rs::UserRole` exposes today:

- **`admin`** — tenant administrator. Manages users, tenant settings,
  approval pipeline. Has cross-user visibility within their own tenant.
- **`user`** — ordinary employee. Submits receipts, owns their own
  expense reports. No cross-user visibility.
- **`approver`** — schema-only role today; *no semantics* until #41
  (hyväksymishierarkia) lands. Treated as `user` for read scope and
  `user`-plus-approval-queue-write for #41's eventual rollout.

A fourth role, **`expert_reviewer`**, is referenced in #57 Phase 4 as a
cross-user oversight role. It is **not yet in the schema**; the matrix
lists it as `expert_reviewer (future)` with the proposed scope so the
implementation has a target.

### 1.2 Channels

`crates/ops/src/context.rs::Channel`:

- **`Web`** — authenticated browser session (`sessions` table). CSRF +
  cookie + form submission. Highest-trust interactive channel; the user
  is physically present and can review every action.
- **`EmailAgent`** — the LLM tool-use phase of the inbound email loop.
  Sender is authenticated by SPF/DKIM/DMARC + `user_emails.verified` +
  `tenant_users.status='active'` + `tenants.status='active'` (see
  `ingest::resolve_inbound_sender`). Lower trust than `Web` because:
  (a) email sender authenticity is policy not cryptography for the same
  account, (b) the LLM is the actor making the call, (c) there is no
  per-message confirmation step.
- **`EmailIngest`** — pre-LLM phase of the inbound loop: spam triage,
  attachment extraction, attachment storage. System-driven; the actor
  is the binary, not the user. Reads/writes are scoped to the
  resolved sender's `(tenant, user)` like `EmailAgent` but the LLM
  has not yet been invoked.
- **`Internal`** — bootstrap / migration / system jobs. Used by
  `OpContext::system()` and by background sweepers (`agent_runs`
  cancellation sweeper, expired-session cleanup, expired-pending
  cleanup). Tenant-/user-id are **0** — `require_admin` rejects
  sentinel rows even when role is `Admin` (see
  `OpContext::require_admin` guard).
  Subdivided into `bootstrap` (one-shot setup paths like
  `create_tenant` and `accept_invitation` where the row's owner is
  established as a side effect), `maintenance-global` (sweeper-style
  writes across tenants on bounded predicates — never freeform
  cross-tenant reads/writes; see §1.3), and `global-cache`
  (non-PII shared caches like `exchange_rates_cache` and
  `tax_rates`).
- **`TokenBearer`** — pre-context principal for token-bearing routes
  (`/set-password`, `/accept-invitation`, `/reset-password`,
  onboarding submit). Identity is established by holding a valid,
  unconsumed `auth_tokens` / `invitations` token; the route consumes
  the token and then promotes to `Web` with a session. Strict scope:
  read/write only the user the token binds to, only the action the
  token's `purpose` permits, single-use. Never enumerates other
  rows. Distinct from authenticated `Web` because the actor doesn't
  yet have a session.

### 1.3 Scope codes

The matrix cells use a small vocabulary:

| Code | Meaning |
|------|---------|
| **`own`** | scoped to `(ctx.tenant_id, ctx.actor_user_id)` — only the actor's own rows |
| **`current-tenant`** | the single `tenants` row identified by `ctx.tenant_id`. (Used because `tenants` rows have no `user_id` and the `own`/`tenant` distinction collapses for them.) |
| **`tenant`** | scoped to `(ctx.tenant_id)` — any row in the actor's tenant |
| **`maintenance-global`** | system jobs may write rows across tenants but **only** under bounded predicates like `expires_at < NOW()` or `started_at < cutoff`. Used by sweepers. Never grants free-form cross-tenant reads or writes. |
| **`global-cache`** | a system-shared, non-PII cache (`exchange_rates_cache`, `tax_rates`). No tenant ownership. |
| **`bootstrap-resolver`** | pre-context global lookup needed by inbound resolution (sender email → user/tenant). Read-only; gated by the `InboundResolver` invariant in §4.6. |
| **`forbidden`** | the role/channel cannot perform this op; returns `OpError::Forbidden` (or `InvalidToken` for token-bearing failure modes that must not leak existence) |
| **`pending`** | the op is allowed but creates a `pending_admin_actions` row that requires `Web` confirmation by an admin in the same tenant before the side effect lands. The confirmer may be the original requester or any peer admin (4-eyes is allowed; not enforced as MVP). |
| **`bootstrap`** | the op is allowed only with `OpContext::system()` on a one-shot setup path (registration, invitation accept); not reachable from authenticated channels |

A read column reads as "the rows this principal may see"; a write
column reads as "the rows this principal may insert/update/delete".
"Cross-user read inside tenant" is the difference between `own` and
`tenant`; cross-tenant is **never** allowed except for the small set
of explicitly global lookups.

### 1.4 Channel-aware policy

A row's `EmailAgent` cell may differ from its `Web` cell even for the
same role. The two ways it differs in this matrix:

- **Lower scope** — `tenant` on `Web` becomes `own` on `EmailAgent`
  (the email channel cannot do cross-user reads even for admins,
  because we have no equivalent of the web admin's session-level
  auditability).
- **Pending instead of direct** — destructive admin operations
  (`disable_user`, `update_role` to/from admin) are `pending` on
  `EmailAgent` and direct on `Web`. The agent still records the
  intent and emits a confirmation link to the requesting admin's
  email; the actual side effect lands when the admin clicks the link
  in a `Web` session.

## 2. Matrix

Domain objects, grouped. Cells read `read / write`; e.g. `tenant / own`
means "may read any row in the tenant, may write only their own". A
single cell (e.g. `own`) applies to both unless split.

### 2.1 Identity & membership

| Object | `user` Web | `user` EmailAgent | `admin` Web | `admin` EmailAgent | `approver` (today=user) | `expert_reviewer` (future) | `EmailIngest` | `Internal` | `TokenBearer` |
|--------|-----------|-------------------|-------------|--------------------|------------------------|---------------------------|---------------|------------|---------------|
| **`tenants` (own row)** | current-tenant (read), forbidden (write) | current-tenant (read only) | current-tenant (read + write — `update_settings`) | current-tenant (read), forbidden (write) | =user | tenant (read) | current-tenant (read) | bootstrap (write at create_tenant) | n/a |
| **`users` (identity row)** | own (read profile fields), own (write `name`) | own (read) | tenant (read); tenant (write — only via `invitation::*` and `pending::confirm`, never directly) | own (read); **pending** (write via `invite_user` only — confirms create the row) | =user | tenant (read) | own (read) | bootstrap (write at registration / invitation accept) | own (read+write only the user the token binds to, only the token's bound action) |
| **`tenant_users` (membership)** | own (read role/status) | own (read) | tenant (read + write role/status — `update_role`, `disable_user`, `enable_user`) | own (read); **pending** (write — all of `invite_user`, `enable_user`, `disable_user`, `update_role` route through `pending_admin_actions`; never direct) | =user | tenant (read) | own (read) | bootstrap (write at registration / invitation accept) | own (write — invitation accept only) |
| **`user_emails`** | own (read + write — add/remove non-primary) | own (read only) | tenant (read); own (write — admins manage *their own* emails only) | own (read) | =user | tenant (read) | own (read) | bootstrap-resolver (read for `find_user_by_email`, gated by `InboundResolver` invariant); bootstrap (write at registration / invitation accept) | own (write — verification at invitation accept / set-password) |
| **`invitations`** | forbidden (no per-user invite list) | forbidden | tenant (read + write — `invite_user`, cancel) | forbidden (read); **pending** (write — invitation row is created on confirm, not on agent call) | =user | tenant (read) | n/a | bootstrap (write at accept_invitation) | own (read + write — accept the bound invitation) |
| **`auth_tokens`** (verification, password reset, onboarding) | n/a | forbidden | n/a (admins do not directly read other users' tokens; `gsadmin password-reset` mints via `Internal`) | forbidden | =user | forbidden | n/a | bootstrap (mint + consume) | own (read + consume only the bound token, single-use) |
| **`sessions`** | own (write — login/logout) | n/a | own (write — own login/logout); tenant (cross-user delete only as transactional side effect of `disable_user`, never as a standalone op) | n/a | =user | forbidden | n/a | maintenance-global (sweeper deletes expired across tenants on bounded `expires_at <` predicate) | own (write — login establishes session) |

### 2.2 Receipts, attachments, expenses

| Object | `user` Web | `user` EmailAgent | `admin` Web | `admin` EmailAgent | `approver` (today=user) | `expert_reviewer` (future) | `EmailIngest` | `Internal` |
|--------|-----------|-------------------|-------------|--------------------|------------------------|---------------------------|---------------|------------|
| **`receipts`** | own (read + write — `save_receipt`, `update_receipt`) | own (read + write via tools — same fns) | own (read + write); cross-user read in admin UI is **policy: own only** for MVP, escalation is a pending decision | own | =user | tenant (read) for review, no write | n/a | n/a |
| **`receipt_revisions`** | own (read) | own (read) | own | own | =user | tenant (read) | n/a | n/a |
| **`receipt_attachments`** (junction → `attachments`) | own (read inline bytes via `/receipts/:id/attachments/:aid`) | own (read via tool) | own | own | =user | tenant (read) | n/a | n/a |
| **`attachments`** (raw bytes) | own (read via receipts route only — never naked `/attachments/:id`) | own (read via receipt-context tool) | own | own | =user | tenant (read via receipt context) | own (write — `save_attachment` during ingest) | n/a |
| **`extractions`** | own (read) | own (read for context); write via `record_extraction` happens on `EmailIngest`, not here | own (read) | own (read) | =user | tenant (read) | own (write — `record_extraction`) | n/a |
| **`load_extraction_summaries`** | n/a (tool only) | own (read for re-use across the same `message_id`); see §4 open question | n/a | own | n/a | n/a (covered by per-receipt views in Phase 4) | own | n/a |
| **`expenses`** | own (read + write — `add_expense`, `update_expense`, `set_expense_status`) | own (read + write via tools) | own; **#41 will add cross-user reads for approvers/admins on the approval queue, not for the expense detail view** | own | =user (until #41) | tenant (read) | n/a | n/a |

### 2.3 Threads, conversations, ingest lifecycle

| Object | `user` Web | `user` EmailAgent | `admin` Web | `admin` EmailAgent | `approver` (today=user) | `expert_reviewer` (future) | `EmailIngest` | `Internal` |
|--------|-----------|-------------------|-------------|--------------------|------------------------|---------------------------|---------------|------------|
| **`threads`** | own (read; future) | own (read for thread-aware tools) | own | own | =user | tenant (read) | own (write — `claim_with_thread`, `create_thread_tx`) | n/a |
| **`thread_messages`** | own (read; future) | own (read) | own | own | =user | tenant (read) | own (write — `record_thread_message_tx`) | n/a |
| **`email_processing`** | n/a | n/a (tool can ask "did this message process?" via #57 Phase 4 expert UI) | n/a | n/a | n/a | tenant (read) | own (write — `try_claim_message`, `update_status`, `update_spam_verdict`, `mark_failed`, retry) | bootstrap (sweeper) |
| **`conversations`** | own (read; future "/me transcript" route) | own (read recent N for context) | own (read) | own (read) | =user | tenant (read) | own (write — `save_conversation_messages_by_sender`, `persist_successful_reply`) | n/a |
| **`user_profiles`** | own (read + write — settings page, future) | own (read via `get_user_context`); own (write via `update_user_preferences`, `update_user_notes`, onboarding submit) | own | own | =user | tenant (read; profile is part of expense context) | own (read for context only) | bootstrap (onboarding submit) |
| **`user_profile_revisions`** | own (read) | own (read) | own | own | =user | tenant (read) | own (write — agent edits emit revisions) | n/a |

### 2.4 Audit, agent trace, system

| Object | `user` Web | `user` EmailAgent | `admin` Web | `admin` EmailAgent | `approver` (today=user) | `expert_reviewer` (future) | `EmailIngest` | `Internal` |
|--------|-----------|-------------------|-------------|--------------------|------------------------|---------------------------|---------------|------------|
| **`audit_events`** | forbidden (read); own (write happens implicitly inside ops fns, not by user request) | forbidden (read); same write semantics | tenant (read — admin audit log, future #57 §6) | forbidden (read); same write semantics | =user | tenant (read) — primary consumer | own (write — implicitly inside ops fns) | own (write — implicitly) |
| **`agent_runs`** + **`agent_steps`** | own (read; future #57 Phase 3 UI) | n/a | tenant (read — admin needs cross-user trace access for forensics: "what did the agent do for user X on inbound message Y?") | n/a | =user | tenant (read) — primary consumer | own (write — `start_run`, `record_step`, `finalize_run`, `record_inline_decision_run`) | maintenance-global (write — sweeper finalizes `aborted_cancelled` across tenants on `started_at <` predicate) | n/a |
| **`pending_admin_actions`** | forbidden (a non-admin can never confirm; pending rows are admin-authority operations) | forbidden | tenant (read); tenant (write — `confirm_pending` re-runs the role check and re-locks the target row, so any tenant admin can confirm any pending in their tenant; tenant `cancel_pending` for cleanup) | n/a — agent only **creates** pending rows via `create_pending`; confirmation always lands on `Web` | =user (forbidden) | tenant (read) | n/a | maintenance-global (expiry sweeper) | n/a |
| **`exchange_rates_cache`** | n/a (no direct user touch) | n/a | n/a | n/a | n/a | n/a | global (read + write — system-shared cache, no PII) | own (write — backfills) |
| **`tax_rates`** | global (read — public Verohallinto data) | global (read) | global | global | global | global | global (read) | own (write — admin-pushed updates) |

## 3. Follow-up issues (per ops fn that does not yet match)

Most ops functions already match the policy because the matrix codified
the existing default (own-scoped for everything except admin tenant
operations and the small set of global lookups). The mismatches:

- **`ops::user::is_known_sender`** — currently *global* (no tenant
  scope). Matrix says global is OK for the spam-triage trust signal
  *because the MVP single-tenant invariant makes tenant scope
  tautological* (see comment in `is_known_sender`). When multi-tenant
  membership (#63) lands, this becomes a real cross-tenant leak. **No
  action now.** Re-evaluate as part of #63.

- **`ops::extractions::load_extraction_summaries`** — **done in #99
  (2026-05-01).** SQL now filters `(tenant_id, user_id, message_id)`
  with `user_id` sourced from `ctx.actor_user_id`; cross-user
  same-message-id covered by ops-side sqlx test
  `cross_user_same_message_id_isolates_per_caller`.

- **`crates/server::tools::user::get_user_context`** &
  **`tools::user::util::load_profile_snapshot_tx`** /
  **`ensure_profile_exists_tx`** — query references `users.email`,
  `users.role`, `users.tenant_id` which **do not exist** in the
  post-A3 schema. Tool is broken. Matrix says it's `own (read)` — the
  rewrite must scope by `(ctx.tenant_id, ctx.user_id)` via
  `tenant_users` JOIN. **Open spin-off #100.** (Independent of #67;
  the tool was broken regardless.)

- **`agent_runs.tenant_id` NOT NULL invariant** vs.
  **`unknown_sender` decision-row mismatch** — matrix says `unknown_sender`
  events are `email_processing.status` not `agent_runs`. Already
  documented in `crates/ops/AGENTS.md`. **No action.**

- **Cross-user admin reads on receipts / expenses** — current
  `crates/ops/src/receipts/view.rs` is strict own-scope, including
  `admin`. Matrix says `admin Web` = own (until #41 approval queue).
  **Matches.** When #41 lands the approval queue gets its own ops
  surface; the existing fns stay own-scoped.

- **Audit log read by admin** — there is **no** read fn for
  `audit_events` today. Matrix says `admin Web = tenant` for read.
  **Open spin-off #101 once the admin audit-log UI is needed**
  (driven by #57 §6, not by #67 directly).

The remaining ops functions (`tenant::*`, `auth::login`,
`session::*`, `password_reset::*`, `invitation::*`,
`user::list_users`/`update_role`/`disable_user`/`enable_user`,
`receipts::*`, `expenses::*`, `attachments::save_attachment`,
`extractions::record_extraction`, `agent_trace::*`,
`ingest::*`, `onboarding::*`, `user_profile::*`) match the matrix as
written — verified by spot-checking each fn's WHERE clauses against
the corresponding cell.

## 4. Open questions / not-decided

### 4.1 `expert_reviewer` scope is provisional

The matrix lists `tenant (read)` for all rows the reviewer needs to
see. The actual #57 Phase 4 design has not landed, and there is a
real question whether the reviewer should be **per-tenant** (one
reviewer per tenant) or **assigned** (a reviewer claims a small set
of tenants and only sees those). The latter is closer to the privacy
goal of #36. The matrix is non-binding for this row until Phase 4
specs the model.

### 4.2 Multi-tenant membership (#63)

When the same user belongs to multiple tenants, several rows change:

- `find_user_by_email` cannot deterministically pick a tenant; the
  caller must pass tenant context. The MVP `LIMIT 2` trip-wire
  becomes the right *error path* for the wrong call shape.
- `is_known_sender` becomes truly tenant-scoped (the MVP "global is
  fine" reasoning evaporates).
- `Internal` channel session resolution acquires "active tenant"
  semantics — `sessions.tenant_id` becomes the disambiguator.
- The pending-admin-actions rows must record the *acting* tenant.

This is filed as a multi-issue cluster behind #63; #67 stays
single-tenant for MVP correctness.

### 4.3 External / API tokens

`/api/*` endpoints today are session-cookie-authenticated. When API
tokens land (Phase 4+), they introduce a fifth channel `ApiKey` with
its own scope rules — typically `tenant (read + write)` for an
admin-issued token, scoped further by the token's permitted op list.
**Out of scope for #67.** Filed as #102 placeholder.

### 4.4 `EmailIngest` write surface

The matrix says `EmailIngest` writes `own`-scoped rows for the
resolved sender's `(tenant, user)`. There is no separate
`UserRole::System` distinct from `UserRole::User`; the channel is
the discriminator. This works because `EmailIngest` ops never call
`require_admin`. If a future ingest-time op needs admin (e.g. an
auto-cleanup of stale invitations triggered by a reply), the
`Internal` channel + `system()` context is the right pattern, not
`EmailIngest` + admin role.

### 4.5 Channel-aware policy: which ops are `pending`?

**All four `EmailAgent` admin operations route through `pending`:**
`invite_user`, `enable_user`, `disable_user`, `update_role`.

Initial v1 strawman split this — `disable_user` / `update_role` were
pending, `invite_user` / `enable_user` direct. v1's LLM review (Gemini
3.1 Pro + GPT-5.5, both P0) flipped that split, and the lock at v1.1
goes with the uniform "everything pending" rule. Reasoning:

- **`invite_user`**: not low-impact. The invitation creates a real
  tenant membership and emails an accept link to whatever address
  the agent extracted. Prompt injection in inbound mail can steer
  the LLM to invite an attacker-controlled address. Once that user
  accepts they can read tenant-scoped surfaces (per their role) and
  submit fraudulent expenses. Direct invitation creation from the
  email channel is therefore a privilege-grant primitive driven by
  natural language. The pending step buys an admin a one-click
  sanity check on the destination address before the row lands.
  **`invite_user` with `role=admin` is still rejected at the ops
  layer regardless of channel** — pending doesn't unlock admin
  invites; that path is closed in MVP.
- **`enable_user`**: not low-impact. A disabled user is usually
  disabled for cause (departure, security incident, suspected
  fraud). LLM-driven re-enable could undo a deliberate lockout.
  Pending puts a Web confirmation at the moment the lockout reverses.
- **`disable_user`**: high blast radius (sessions cleared, user
  cut off, data hidden from admin lists). Pending.
- **`update_role`**: escalation/de-escalation of admin powers when
  the change touches `admin`; even `user ↔ approver` matters once
  #41 lands and approver gets cross-user authority. The MVP rule is
  "any role change is pending" — simpler than per-target heuristics,
  one-click for the admin, future-safe.

**Uniformity makes the policy easier to reason about:** there is
exactly one rule for `Channel::EmailAgent + admin op + tenant data
write = pending`. No per-action exceptions, no "but enable is
different". If a future op needs to be direct from the agent, the
review here flips the burden of proof onto that op's author — they
have to argue for why the uniform rule shouldn't apply.

**What pending does *not* gate:**

- Read-only EmailAgent ops (`get_user_context`, `list_users`-style
  reads). These remain direct because the matrix already restricts
  what they can read (own/tenant per row).
- `update_user_preferences` / `update_user_notes`. These are
  self-service writes scoped to `(ctx.tenant_id, ctx.user_id)` and
  do not change anyone else's authority or data. Direct is fine.
- Web channel admin operations. The admin is already in a session,
  staring at the page; pending would be busywork.

This is the policy as of v1.1 (locked post-LLM-review). It can be
revised once we have operational data on admin friction with the
pending step. If the pending UX proves annoying for low-stakes
operations (unlikely — it's one extra click), the next revision
will revisit the uniformity rule, not paper over per-action
exceptions.

### 4.6 Inbound resolver + global identity invariants

`ops::user::find_user_by_email` and `ops::ingest::resolve_inbound_sender`
do a **bootstrap-resolver** read on `user_emails` joined to
`tenant_users` + `tenants`. The invariants the resolver must satisfy
before returning a usable `(tenant_id, user_id)` pair:

- email is normalized via `ops::email::normalize`
- `LOWER(user_emails.email)` matches exactly (the unique index
  guarantees ≤1 row in MVP single-tenant; ≥2 rows is an
  `InvalidInput` error today, not a `LIMIT 1` non-determinism)
- `user_emails.verified = true` (ingest gate)
- `tenant_users.status = 'active'`
- `tenants.status = 'active'`

Ambiguous matches **fail closed** — we treat the message as
`unknown_sender` and the agent does not run. The `LIMIT 2` trip-wire
in `find_user_by_email` is an error detector, not an authorization
model: when multi-tenant membership (#63) lands, `find_user_by_email`
gets a tenant disambiguator (recipient address, alias, configured
domain) and the trip-wire becomes a "did the disambiguator narrow
correctly?" check.

`ops::user::is_known_sender` is also a `bootstrap-resolver` read —
returns a boolean, never a tenant id, used by the spam triage. It's
"global" only in the sense that the lookup ignores tenant scope; in
MVP single-tenant the address is one-to-one with a tenant anyway.
When #63 lands, the spam triage either takes a tenant context from
the recipient address or moves to a strictly per-(tenant) lookup.

## 5. Versioning

- **v1 (2026-05-01)**: strawman.
- **v1.1 (2026-05-01)**: post-LLM-review (Gemini 3.1 Pro + GPT-5.5).
  Locked. Major changes from v1:
  - **All four `EmailAgent` admin operations route through `pending`**
    (not just `disable_user` and `update_role`). Driven by P0 from
    both reviewers — direct `invite_user`/`enable_user` from email
    are escalation primitives accessible via prompt injection.
  - **`pending_admin_actions` semantics tightened**: non-admins
    cannot confirm; tenant admins can confirm any pending in their
    tenant; rejection of stale-admin attempts happens at confirm
    time via `ctx.require_admin()` against the current row.
  - **`agent_runs` admin Web read scope** lifted from `own` to
    `tenant` (forensic access).
  - **`sessions` cross-user delete language** clarified: only as a
    transactional side effect of `disable_user`, never as a
    standalone op.
  - **`Internal` channel split** into `bootstrap` /
    `maintenance-global` / `global-cache` so sentinel-tenant rows
    can't masquerade as `own` writes.
  - **`TokenBearer` principal** added for pre-context routes
    (`/set-password`, `/accept-invitation`, `/reset-password`,
    onboarding submit).
  - **`InboundResolver` / `bootstrap-resolver` invariants** added
    in §4.6.
  - **`current-tenant` scope code** introduced for `tenants` row to
    fix the `own`/`tenant` ambiguity when there is no `user_id`.
  - **`load_extraction_summaries`** mandatory tightening to
    `(tenant_id, user_id, message_id)` (#99) — not deferred to #63.

  Future revisions are tracked here with rationale. Once a row is
  locked, ops authors implement to it; if behaviour drifts, the
  drift is filed as a follow-up issue (per §3) rather than papered
  over by quietly editing the row.
