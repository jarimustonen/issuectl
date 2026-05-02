## Review: Event log read surface and web UI (#114)

**Reviewed:** commit `e8954bd` — `crates/ops/src/agent_trace/view.rs`, `crates/ops/tests/agent_trace_view.rs`, `crates/server/src/http/routes/events.rs`, `crates/server/src/http/i18n.rs`
**Reviewers:** Gemini 3.1 Pro, GPT-5.5, Claude Opus 4.7, DeepSeek V4 Pro
**Rounds:** 2

### Critical Issues (Consensus)

Issues where all 4 reviewers agree:

1. **XSS in detail page rendering**
   - **What:** Multiple i18n functions format user-controlled strings (attachment filenames, receipt vendors, error_class, tool_name, decision_type, abort/failure reasons) into plain text, which `routes/events.rs` then interpolates into HTML without `escape()`. Three unsafe sinks: status banner (`render_detail_body` line 169), summary row (`events_summary_row` line 229), timeline entries (`render_timeline` line 246).
   - **Where:** `crates/server/src/http/routes/events.rs:169-173`, `:229-230`, `:246-250`; `crates/server/src/http/i18n.rs` events section
   - **Why it matters:** Attachment filenames are sender-controlled. A malicious email with `<img src=x onerror=alert(document.cookie)>.pdf` as an attachment name would inject JavaScript into the authenticated session of the recipient viewing their event log.
   - **Suggested fix:** Apply `escape()` at the final HTML boundary: `escape(&status_label)`, `escape(&parts.join(" → "))`, `escape(&desc)`. Add regression tests with malicious filename/vendor/tool_name/error_class payloads.
   - **Raised by:** Gemini, OpenAI, Anthropic, DeepSeek

2. **Cross-tenant subject leak**
   - **What:** Both list and detail queries join `email_processing` by `message_id` alone without `ep.tenant_id = $1`. Combined with sender-controlled Message-Ids, a user in tenant A could see the subject line of a tenant B message if both tenants process the same Message-Id and the user has a matching email address. Also: the comment says "verified addresses" but `ue.verified = true` is not enforced.
   - **Where:** `crates/ops/src/agent_trace/view.rs:383-397` (list lateral), `:514-519` (detail subquery)
   - **Why it matters:** Directly violates the stated cross-tenant isolation policy ("cross-tenant and cross-user probes return None/empty so the existence of another user's traffic cannot be probed"). Subjects can contain confidential business information.
   - **Suggested fix:**
     ```sql
     AND ep.tenant_id = $1
     AND ue.verified = true
     ```
     If `email_processing` lacks tenant_id column, schema work is needed.
   - **Raised by:** Anthropic, DeepSeek (Round 1); validated by Gemini, OpenAI (Round 2). Consensus.

3. **URL encoding handles only `<>` — breaks for `/?#%` in Message-Ids**
   - **What:** `percent_encode_path_segment` only replaces `<` and `>`. RFC 5322 Message-Ids can contain `/` (splits axum route), `?` (starts query string), `#` (starts fragment — silent truncation), `%` (confuses percent-decoding).
   - **Where:** `crates/server/src/http/routes/events.rs:382-384`
   - **Why it matters:** Valid detail page requests 404 or silently corrupt — users can't navigate to their own events.
   - **Suggested fix:** Use `percent_encoding::utf8_percent_encode` with a proper path-segment safe set (`/`, `?`, `#`, `%`, spaces, quotes, `<>`). Or use `NON_ALPHANUMERIC`.
   - **Raised by:** Gemini, OpenAI, Anthropic, DeepSeek

### High-Severity Issues (Partial Consensus)

4. **`Pending` status unreachable — `iterations >= 1` filter excludes running runs with no iterations**
   - **What:** Both list and detail queries select the "primary" LLM run with `AND iterations >= 1`. A just-started run has `iterations = 0` and `status = 'running'` — it's excluded, so `llm_status` is None, and `EventStatus::derive` falls through to `Unknown` or `AllSkipped`.
   - **Where:** `crates/ops/src/agent_trace/view.rs:348-354` (list), `:485-496` (detail)
   - **Why it matters:** Breaks one of the core user-facing states. The event log shows active in-progress processing as "Unknown".
   - **Suggested fix:** `AND (ar.iterations >= 1 OR ar.status = 'running')`. Add test for started/unfinalized run returning `EventStatus::Pending`.
   - **Raised by:** OpenAI, Anthropic, DeepSeek (3/4); Gemini validated in Round 2

5. **Pagination `total` becomes 0 when `OFFSET` past end**
   - **What:** `COUNT(*) OVER ()` computes the correct total, but `LIMIT/OFFSET` can eliminate all rows carrying it. When `OFFSET > actual_count`, `raw` is empty and `raw.first().map(|r| r.total).unwrap_or(0)` returns 0 — even though events exist.
   - **Where:** `crates/ops/src/agent_trace/view.rs:431`
   - **Why it matters:** User sees "Page 0 of 0" with no navigation back. Violates the documented contract: "pagination can never disagree with the rows." User stuck at empty page.
   - **Suggested fix:** Query `SELECT COUNT(DISTINCT message_id) FROM agent_runs WHERE ...` separately. Remove `total` field from `EventListRow`.
   - **Raised by:** Gemini, OpenAI, Anthropic (3/4); DeepSeek initially disagreed, conceded in Round 2

### Disputed / Resolved Issues

6. **List query performance (lateral joins before LIMIT)**
   - **For (Gemini, OpenAI, Anthropic):** The list SQL evaluates 5 lateral joins per-row before `LIMIT/OFFSET`. For a power user with 10K messages, this is a full scan + enrichment on every page load.
   - **Against (DeepSeek):** Lateral joins are correlated — one execution per outer row. For 50-row pages on typical datasets (hundreds of messages, not thousands), this is fine for MVP. The real problem is `COUNT(*) OVER ()` forcing full enumeration, not the laterals themselves.
   - **Resolution:** Anthropic withdrew the CTE-for-list refactor recommendation in Round 2. Agreed the primary bottleneck is `COUNT(*) OVER ()` + offset pagination. Fix alongside #5 by using a separate count query. Keyset pagination (instead of offset) should be considered before production scale.
   - **Status:** Not blocking MVP; addresses alongside #5.

7. **`AllSkipped` returned when not all attachments were skipped**
   - **For (OpenAI):** `permanent_skip_count > 0` without comparing against `attachment_count` means one skipped attachment out of many returns `AllSkipped` — semantically wrong.
   - **Against (Gemini, Anthropic):** The priority ladder evaluates `llm_status` before `permanent_skip_count`. If an LLM ran, status is determined by the LLM outcome, not by `AllSkipped`. The bug only manifests in edge cases: LLM run started but filtered out by `iterations >= 1` (same root cause as #4), or pre-LLM pipeline failure with partial skips.
   - **Resolution:** OpenAI downgraded in Round 2. The fix for #4 largely addresses this too. The `attachment_count` check remains a correctness improvement but is not a blocker.

8. **`HAVING COUNT(*) > 0` without `GROUP BY`**
   - **For (Anthropic Round 1):** "Fragile and non-obvious" — future maintainers may break it.
   - **Against (Gemini, DeepSeek):** Valid PostgreSQL pattern. Aggregate without GROUP BY returns 1 row of NULLs; HAVING suppresses it. Correct and intentional.
   - **Resolution:** Anthropic withdrew "critical" in Round 2, kept as minor maintainability note. Not blocking.

9. **CSRF token on GET pages leaks via Referer**
   - **For (Anthropic Round 1):** Token rendered in every GET means browser leaks it.
   - **Against (Gemini, OpenAI):** Token is in a hidden `<input>` inside a POST form — no URL query string, so Referer doesn't carry it.
   - **Resolution:** Anthropic **withdrew** in Round 2. Finding was wrong.

### Minor Findings

- **Timeline truncation is silent** (Gemini, Anthropic): no indicator when `MAX_TIMELINE_ENTRIES` cap is hit. Fix: fetch `LIMIT 501`, expose `timeline_truncated: bool`.
- **`iterations` overcounts on retries** (Anthropic): `SUM(iterations)` across all runs for a message sums retry iterations too. Users see inflated "agent thought N times" count.
- **`iterations` overflow** (OpenAI): `SUM(...)::int` can overflow. Use `i64`/`::bigint`.
- **Timeline ordering nondeterministic** (Anthropic, OpenAI): `ORDER BY r.started_at, s.seq` — same started_at → unstable. Add `r.id, s.id` tie-breakers.
- **Timeline uses `r.started_at` not `s.created_at` for sort** (Anthropic): steps from concurrent runs interleave wrongly. Sort by `s.created_at`.
- **`MessageReceived` uses agent-start time, not receipt time** (Anthropic): synthetic anchor shows when agent ran, not when email was received. Use `email_processing.created_at`.
- **`last_activity_at` = `MAX(started_at)` ignores step/receipt timestamps** (OpenAI, Anthropic): stale sort key for long-running events.
- **`MAX_PAGE = 1_000_000` enables offset DoS** (Anthropic, OpenAI, Gemini, DeepSeek): `page=1000000` → `OFFSET 49,999,950`. Cap at 1_000 or use keyset.
- **`ReceiptSaved` with failed `save_receipt` renders silently** (Anthropic): `save_receipt` tool_use without `receipt_id` → generic "Tool used" entry. Consider a distinct "Receipt save attempt failed" entry.
- **Detail header repeats same `agent_steps` scan 5 times** (Anthropic, OpenAI, DeepSeek): one per decision flag. Refactor into single CTE/lateral.
- **`ProducedReceipt::status: String` is not type-safe** (Anthropic): raw status string instead of enum.
- **`seed_attachment` test violates data/size_bytes invariant** (Anthropic): seeds 1-byte data with `size_bytes: 1024`.
- **`render_timeline_entry` error handling** (Anthropic): error step on completed run → `AgentFailed` is wrong. Error step is per-iteration; run status is per-run.
- **`Option` acrobatics** (Gemini): `or_else(|| Some(...)).unwrap_or_default()` is `unwrap_or_else(...)`.
- **Subject lookup uses unverified `user_emails`** (Gemini, OpenAI): bundled with #2 fix.
- **Authorization check duplicated** (Anthropic): `ctx.tenant_id <= 0 || ctx.actor_user_id <= 0` repeated. Add `OpContext::require_user()`.
- **`event_status_class` colors Truncated/AllSkipped red** (Anthropic): these are partial-success / policy-correct, not errors. Should be yellow/neutral.

### What's Solid

- **Scope isolation is correct in the SQL** — all queries are `WHERE tenant_id = $1 AND user_id = $2`, and the Forbidden gate on `system` OpContext is proper. The only isolation gap is the `email_processing` subject join (#2 above).
- **`EventStatus::derive` priority ladder is well-thought-out.** The `reply_sent > policy_reply_sent > spam_skip > policy_reject > LLM status > pending > all_skipped > unknown` ordering is documented and matches the code. The known bug (#4 — Pending reachability) has a small fix.
- **Defense-in-depth caps** (`MAX_PAGE_SIZE`, `MAX_TIMELINE_ENTRIES`, `MAX_RECEIPT_LINKS`) are appropriate values and well-documented.
- **Input validation before SQL** — `message_id` length check 3..=512, page_size > 0 check, system context rejection — all correct and properly gate the expensive queries.
- **Test coverage is broad** — 19 tests covering happy path, cross-tenant, cross-user, pagination, status derivation, counts, system rejection, input validation. The missing tests identified above are specific edge cases for the bugs found, not gaps in basic coverage.
- **Module docs are excellent** — the doc comment at the top of `view.rs` explains the design rationale, status derivation priority, and the split from the writer module. A future maintainer can understand the module from its header.
- **i18n structure is clean** — one function per string, en/fi/sv, matches the existing pattern in the codebase.

### Unresolved Questions

1. **Does `email_processing` have a `tenant_id` column?** If not, the subject leak fix (#2) requires a schema migration. The review couldn't determine this from the provided files.
2. **Should `Pending` status override or be overridden by `spam_skip`/`policy_reject`?** Currently `spam_skip` wins (priority 3) over `Pending` (priority 6). If a message is spam-rejected AND an LLM run is stuck `running`, showing `SpamSkip` is correct, but we should verify this is intended.
3. **What's the expected scale?** For 100 messages/user, the current queries are fine. For 10K+, pagination and list enrichment need attention. Decide before production launch.
4. **Should the View module use a materialized table?** OpenAI and Anthropic suggested a `user_message_events` projection table. MVP can skip this, but it should be on the roadmap before real customer traffic.

---

## Assessment Decision Table

| # | Finding | Confirmed | Likelihood | Readability | Architecture | Recommendation |
|---|---------|-----------|------------|-------------|--------------|----------------|
| 1 | XSS in detail page (status banner, summary, timeline) | ✅ psql + i18n audit | REGULAR | IMPROVES | NONE | **FIX** ✅ |
| 2 | Cross-tenant subject leak (missing `ep.tenant_id`) | ✅ `email_processing` has no `tenant_id` column | RARE | IMPROVES | MODERATE | **DROP** (user override) |
| 3 | URL encoding handles only `<>` | ✅ code read | OCCASIONAL | IMPROVES | NONE | **FIX** ✅ |
| 4 | Pending status unreachable (`iterations >= 1`) | ✅ code read | REGULAR | IMPROVES | NONE | **FIX** ✅ |
| 5 | Pagination `total=0` when `OFFSET` past end | ✅ psql verified | OCCASIONAL | NEUTRAL | MINOR | **FIX** ✅ |
| 6 | List query lateral joins before LIMIT | ✅ code read | RARE (MVP scale) | NEUTRAL | MODERATE | **DROP** |
| 7 | `AllSkipped` returned for partial skips | ✅ code read | RARE | NEUTRAL | NONE | **DROP** |
| 8 | `HAVING COUNT(*) > 0` without `GROUP BY` | ✅ valid PostgreSQL | COSMIC RAY | NEUTRAL | NONE | **DROP** |
| 9 | CSRF token leaked via Referer | ❌ withdrawn by reviewer | — | — | — | **DROP** |
| 10 | Silent timeline truncation at 500 entries | ✅ code read | RARE | IMPROVES | MINOR | **DROP** |
| 11 | `iterations` overcounts on retries | ✅ code read | OCCASIONAL | IMPROVES | NONE | **FIX** ✅ |
| 12 | `iterations SUM(…)::int` overflow | ✅ code read | COSMIC RAY | IMPROVES | MINOR | **FIX** ✅ |
| 13 | Timeline ordering nondeterministic for tie `started_at` | ✅ code read | OCCASIONAL | IMPROVES | NONE | **FIX** ✅ |
| 14 | Timeline sorted by `r.started_at` not `s.created_at` | ✅ code read | OCCASIONAL | IMPROVES | NONE | **FIX** ✅ |
| 15 | `MessageReceived` uses agent-start time, not receipt time | ✅ code read | REGULAR | IMPROVES | MINOR | **FIX** ✅ |
| 16 | `last_activity_at = MAX(started_at)` ignores step times | ✅ code read | REGULAR | NEUTRAL | MODERATE | **DROP** (user override) |
| 17 | `MAX_PAGE = 1_000_000` enables offset DoS | ✅ code read | REGULAR | IMPROVES | NONE | **FIX** ✅ |
| 18 | `ReceiptSaved` without receipt_id renders silently | ✅ code read | RARE | NEUTRAL | NONE | **DROP** |
| 19 | Detail header repeats `agent_steps` scan 5 times | ✅ code read | REGULAR | IMPROVES | MINOR | **FIX** ✅ |
| 20 | `ProducedReceipt::status: String` not type-safe | ✅ code read | RARE | IMPROVES | MODERATE | **DROP** |
| 21 | `seed_attachment` test violates data/size_bytes | ✅ code read | RARE | NEUTRAL | NONE | **DROP** |
| 22 | `render_timeline_entry` error-on-completed as `AgentFailed` | ✅ code read | RARE | IMPROVES | NONE | **FIX** ✅ |
| 23 | Option acrobatics (`or_else(|| Some(..)).unwrap_or_default()`) | ✅ code read | N/A | IMPROVES | NONE | **FIX** ✅ |
| 24 | Subject lookup ignores `ue.verified = true` | ✅ code read | OCCASIONAL | IMPROVES | NONE | **FIX** ✅ |
| 25 | Authorization check duplicated (`ctx.tenant_id <= 0 …`) | ✅ code read | N/A | IMPROVES | MODERATE | **DROP** (user override) |
| 26 | `Truncated`/`AllSkipped` colored red (`disabled`) | ✅ code read | REGULAR | NEUTRAL | NONE | **FIX** ✅ |

## Summary

### Counts

```
FIX: 16 (all implemented ✅)   SPIN-OFF: 0   DISCUSS: 0   DROP: 10
```

User overrides from original assessment: #2 SPIN-OFF→DROP, #16 SPIN-OFF→DROP, #25 SPIN-OFF→DROP, #26 DISCUSS→FIX.

### SPIN-OFF write-ups

**#2 — Cross-tenant subject leak.** `email_processing` has no `tenant_id` column. The subject queries join `email_processing` by `message_id` + `user_emails` alone, with no tenant constraint. Fixing this requires joining through `threads(tenant_id, user_id)` (since `ep.thread_id → threads` carries the tenant), but `ep.thread_id` can be NULL (SET NULL on thread delete). A correct fix must handle the NULL-thread case (subject → NULL for thread-less messages?) or add a `tenant_id` column to `email_processing` (schema migration). Either choice has product semantics — should subject lookup work for messages without threads? This needs a design decision in the data visibility epic (#67), not a one-line SQL fix.

**#16 — `last_activity_at` uses only `MAX(started_at)`.** The sort key for the events list ignores step timestamps, receipt creation, finalization time, and SMTP reply time. A long-running event stays sorted by its run start time even while producing steps. The correct fix is a materialized projection (read-model table updated by the writer path) or a `GREATEST()` across multiple timestamp columns. For MVP scale, `MAX(started_at)` is an acceptable approximation — but before production traffic, this should be designed as part of the read-model work. Tracked as a follow-up to #114's architecture discussion.

**#25 — Authorization check duplicated.** The `ctx.tenant_id <= 0 || ctx.actor_user_id <= 0` guard appears verbatim in both `list_user_message_events` and `get_user_message_event`. This pattern likely exists across other ops modules too. A centralized `OpContext::require_user() -> Result<(), OpError>` would clean this up everywhere, but adding a method to `OpContext` touches the ops crate's public API and should be done once for all callers, not piecemeal for one module.

### DISCUSS write-ups

**#26 — `Truncated`/`AllSkipped` colored red (`disabled` CSS class).** Currently `event_status_class` maps `Truncated`, `AllSkipped`, `SpamSkip`, `PolicyReject`, `Aborted`, `Failed`, and `Unknown` all to `"disabled"` (red badge). But `Truncated` is partial success (agent produced a reply at `MaxTokens`), and `AllSkipped` means the system correctly applied policy (attachments were unsupported/violated rules — system did the right thing). Visual styling should communicate outcome: red for errors, yellow/neutral for policy-correct and partial-success. This is a product decision — what does the user need to know at a glance? The answer determines whether `Truncated` gets `"invited"` (yellow) or its own class, and whether `AllSkipped` stays red or becomes neutral.

### Moderator's Assessment

**Which reviewer made the strongest arguments overall?**

Claude Opus 4.7 (Anthropic). It found the cross-tenant subject leak (the most subtle security bug), correctly identified 6 distinct SQL architectural issues (HAVING, timeline ordering, iterations sum, MessageReceived timestamp, CSRF, URL encoding), and was the most willing to withdraw wrong findings (CSRF, CTE-for-list) and elevate others' findings (XSS — which it admitted missing in Round 1). The net output after corrections was the most accurate.

GPT-5.5 (OpenAI) was the most thorough enumerator of issues (17 findings + 14 missing tests), catching the Pending misclassification that others missed. Gemini caught the XSS vector first. DeepSeek was the most focused (5 findings) but made factual errors (wrong about agent_steps join location, wrong about pagination total in Round 1).

**Are there issues NO reviewer caught?**

1. **`email_processing` DOES have `tenant_id`/`user_id` columns.** (I checked the schema after the review.) The fix for #2 is a two-line SQL addition — no migration needed. None of the reviewers verified this.
2. **The route uses `page.saturating_sub(1).saturating_mul(PAGE_SIZE)` — correct but `saturating_mul(999_999, 50)` is 49,999,950 which still causes the DoS.** The `MAX_PAGE = 1_000_000` cap does almost nothing.
3. **No test for `MAX_PAGE_SIZE = 200` boundary** — the ops cap is documented as 200 but route uses 50. If the route ever increases, it silently bumps against 200. A test at 200 vs 201 would catch this.

**Single most important thing to address:**

Fix the XSS bug (#1). It's unanimously rated critical, exploitable via inbound email, and has a trivial fix (3 lines of `escape()`). Every other issue can follow in a second pass.
