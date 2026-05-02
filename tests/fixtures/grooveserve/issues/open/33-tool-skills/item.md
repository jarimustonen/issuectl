---
created: 2026-04-27
updated: 2026-04-29
type: feature
reporter: jari
assignee: jari
status: in-progress
priority: normal
labels: [agent, tools, architecture]
commits:
  - hash: c38777b
    summary: "docs(issues): design skill-based tool architecture (#33)"
  - hash: 2e85d84
    summary: "refactor(email): wrap tools in Tool trait and registry (#33 step 2)"
  - hash: 204ac56
    summary: "refactor(email): split tools/mod.rs into focused submodules"
  - hash: 9ac187c
    summary: "refactor(email): extract handler helpers into tools/util.rs"
  - hash: c59d01a
    summary: "fix(email): treat needs_info as clarification, not tool error"
  - hash: cd7989c
    summary: "chore(email): apply small review fixes from #33 step 2"
  - hash: 8dc8c4d
    summary: "test(email): cover validate failures, dispatch, and wrapper faithfulness"
  - hash: 7ea6381
    summary: "feat(email): add read_skill meta-tool and real skill bodies (#33 step 3a)"
  - hash: abe80e4
    summary: "docs(issues): update #33 status — step 3a done, log decisions for step 3b"
  - hash: 9ebeddd
    summary: "fix(email): raise MAX_TOOL_ITERATIONS to 200 and instrument read_skill"
  - hash: a7e6b23
    summary: "fix(email): English tool descriptions with routing hints"
  - hash: 9ca5e22
    summary: "fix(email): reject negative amounts and no-op preference updates"
  - hash: 3bb6c68
    summary: "fix(email): validate enum filters in update_receipt and list handlers"
  - hash: be9f878
    summary: "fix(email): deterministic line numbering in get_draft_summary"
  - hash: d8496b9
    summary: "fix(email): per-currency totals in get_draft_summary"
  - hash: 18e87c1
    summary: "fix(email): harden add_expense idempotency key"
  - hash: 286d186
    summary: "fix(email): tighten save_receipt vendor dedup to include date and amount"
  - hash: 1025d99
    summary: "fix(email): correct misleading skill content for step-3b accuracy"
  - hash: 3abfd98
    summary: "docs(issues): align design §5.1 with the JSON-wrap decision"
  - hash: 49ddbc3
    summary: "feat(email): swap to skill-based prompt and one-tool-call rule (#33 step 3b)"
  - hash: 083780a
    summary: "refactor(email): inline handler bodies into per-tool execute methods (#33 step 4)"
  - hash: a157f53
    summary: "fix(email): defuse dangling tool_use on truncation and empty-tool stop reasons (#33 review)"
  - hash: 33972da
    summary: "fix(email): add currency, category, expense_type to add_expense idempotency key (#33 review)"
  - hash: fee12c8
    summary: "fix(email): reject empty-string updates in update_receipt and update_expense (#33 review)"
  - hash: 6eba1f6
    summary: "refactor(email): dedupe status enum and tighten update_receipt status flow (#33 review)"
  - hash: 86b30af
    summary: "feat(email): add truncated indicator to list_expenses and list_receipts (#33 review)"
  - hash: b4cdf5c
    summary: "fix(email): refine receipt→expense invariant in system prompt (#33 review)"
  - hash: 5ce1d71
    summary: "docs(email): correct dispatch test docstring and update_user_preferences null wording (#33 review)"
---

# 33. Skill-based tool architecture

_Source: `services/email/src/tools/`, `services/email/src/agent.rs`_

## Description

Refactor the agent's tool system so that each tool has an associated **skill file** — a self-contained markdown document describing when to use the tool, parameter semantics, examples, and edge cases. Add a `read_skill` meta-tool the agent can call on demand. The goal is to keep the system prompt minimal while still giving the agent rich, per-tool guidance when it needs it.

## Motivation

The system prompt in `agent.rs` (`SYSTEM_PROMPT_WITH_TOOLS`) is already ~50 lines and will only grow as we add tools (#15 OCR, #20 Verohallinto-korvaukset, #21 hyväksyntä-kierto, #18 Netvisor, #19 Procountor, #16/#17 calendar). Tool definitions live in one monolithic file (`definitions.rs`) and dispatch happens via a hand-maintained `match` in `mod.rs`. Adding a tool currently requires touching 3+ files, plus often the system prompt itself.

A skill-based design fixes:

1. **System prompt focus** — long-form per-tool usage docs move out of the prompt; the prompt describes role and operating style, not per-tool minutiae
2. **Modularity** — each tool becomes a self-contained module: code + schema + skill markdown, in one directory
3. **Discoverability** — the agent sees the tool catalog in the API tools array (description + name) on every request, and can read each tool's detailed skill via the `read_skill` meta-tool

## Status

The design (`design.md`) is committed. Implementation is split into the steps in design §8, and progress is tracked here.

| Step | Sisältö | Status |
|------|---------|--------|
| 1 | Preconditions (tests green, no in-flight tools/ changes) | ✅ done before start |
| **2** | **Tool trait + registry + 10 wrappers + snapshot test + LLM review + review-fix follow-ups** | ✅ **done** on `design-tool-skills`, 7 commits |
| **3a** | **`read_skill` meta-tool, real skill bodies, keep old prompt** | ✅ **done** on `design-tool-skills` |
| **3a follow-up** | **LLM-review fixes (10 commits) + 3 spin-offs (per-currency totals, idempotency hardening, vendor dedup)** | ✅ **done** on `design-tool-skills` |
| **3b** | **Swap `SYSTEM_PROMPT_WITH_TOOLS` to design §6.2 form + enforce one tool_use per assistant message + update prompt-content test** | ✅ **done** on `design-tool-skills` |
| **4** | **Inline 10 handler bodies into per-tool `execute` methods; delete `handlers.rs`** | ✅ **done** on `design-tool-skills` |
| **post-4 review** | **4-LLM review (`history/review-tool-skills-step3b-step4-impl.md`) + assessment + 7 fix commits** | ✅ **done** on `design-tool-skills` |
| 5 | Final pass on prompt + skill wording driven by production observations from running 3b | ⏳ blocked on production data |

## Notes for the next agent

### Decided during step 2

- **`read_skill` raw markdown vs JSON-wrapped**: design §5.1 left the choice between (a) a dispatcher special-case for `read_skill` and (b) a `ToolPayload::Text` variant on `ToolOutput`. Decision: **neither — JSON-wrapped is fine**. Per §0 (functional correctness only, no token optimisation in MVP), `read_skill` will return `ToolOutput::success(json!({"markdown": "..."}))` and Claude parses the JSON-wrapped markdown without trouble. Revisit only if MVP empirically shows the model struggling with JSON-escaped skill bodies.

### Decided during step 3a

- **Snapshot test had to be updated, not just the placeholder test.** `tests/tools_snapshot.rs` pinned the wire shape against the (now-deleted) `phase1_tools()`; adding `read_skill` at the head of the catalog is a deliberate divergence. The snapshot now includes `read_skill` and the helper test `anthropic_tools_count_and_order` lists it first. Future tool additions update the same snapshot in lockstep.
- **`read_skill` registered first.** In `registry.rs::TOOLS`, `meta::ReadSkill` is the first entry so the agent sees the meta-tool at the top of the catalog on every request. The remaining 10 keep their step-2 order.
- **`ReadSkill` has its own non-empty skill body but no `.skill.md` file.** Registry validation requires every skill body to be non-empty, so `ReadSkill::skill()` returns a one-line hard-coded string explaining that the system prompt documents `read_skill` directly. This keeps the validation rule simple and `read_skill` self-resolvable (a sanity test calls it with `tool: "read_skill"`).
- **Skill bodies are written in English with Finnish domain terms preserved (matkalasku, kuitti, kulurivi, päiväraha, ALV, etc.).** This matches design §3.1: skills target the agent's reasoning language, not the user's reply language. User-facing reply formatting (number format, signature, table style) belongs in the system prompt — see step 3b.

### Things step 3a did

- Implemented `read_skill` as a `Tool` impl in `services/email/src/tools/meta/read_skill.rs`. Looks up the named tool via `registry::get(name)` and returns `ToolOutput::success(json!({"markdown": tool.skill()}))`. Unknown names return `ToolOutput::error("Unknown tool: <name>. Available: <comma-separated list>")`. Registered in `registry.rs::TOOLS` (and `meta` module exported from `tools/mod.rs`).
- Replaced the placeholder skill body in each of the 10 wrappers with `include_str!("./<name>.skill.md")` and authored the corresponding markdown files (10 in `receipts/`, `expenses/`, `user/`).
- Inverted `tools::registry::tests::step2_skills_are_placeholders` → `skills_have_no_placeholder_marker`: asserts no tool's skill contains `"Skill not yet written."`.

### Step 3a follow-up (LLM review of step 3a → 10 commits)

`history/review-tool-skills-step3a-impl.md` captured a 4-LLM critique of the step-3a artifact. After triage and verification (a few reviewer claims were rejected after checking against the migrations), 14 of the surviving findings + 3 spin-offs were applied on the same branch. Highlights of what changed *here* (so the next agent does not re-discover it):

- **`MAX_TOOL_ITERATIONS` is now 200.** Originally planned for step 3b, but pulled forward because `read_skill`'s description ("Call this before using a tool you have not used yet…") is itself an instruction the model follows even with the old prompt — a 4-receipt email could already overflow at 10. Raised here so the new pattern is safe to run.
- **All 10 tool descriptions are now English with explicit routing hints** (`save_receipt` → `add_expense`, `update_receipt` ↔ `update_expense`, `list_*` vs `get_draft_summary`). The step-3b prompt (design §6.2) tells the model the descriptions carry routing hints; that promise is now true. Snapshot updated in lockstep.
- **`read_skill` emits tracing events** on `skill_usage` target so #35 can be data-driven.
- **Handler invariants now match skill claims** that previously over-promised: `parse_money` outputs are passed through `reject_negative` / `reject_negative_optional` so the "never negative" rule is real; `update_receipt.status`, `list_*.status` and `list_*.category` go through `validate_optional_enum`; `update_user_preferences` rejects no-op calls.
- **Three pre-existing handler bugs were fixed as spin-offs**:
  - `get_draft_summary` returns `totals_by_currency` (object keyed by currency) instead of summing across currencies and labelling EUR.
  - `add_expense` idempotency key includes `receipt_id` and uses `Decimal::normalize`, so two distinct receipt-backed expenses don't collapse and `24.50` ≡ `24.5` on retry.
  - **Migration 009** replaces the receipt unique index `(tenant, user, message_id, vendor)` with `(tenant, user, message_id, vendor, receipt_date, total_amount)` so two same-vendor receipts in one email no longer collapse into one row.
- **Skill content corrections**: `add_expense` no longer claims to "inherit financial data" from the kuitti; `update_user_preferences` no longer claims `null` clears a JSONB key (it sets it to JSON null); `get_user_context` no longer instructs the model to "log it" or to invent km distances/Verohallinto rates from addresses.
- **Validation cap counts chars, not bytes** (`registry.rs::validate`).
- **Design.md §5.1 now agrees with `item.md`** that `read_skill` returns JSON-wrapped markdown — earlier they contradicted each other.

Findings deliberately not fixed here:

- **JSONB merge against NULL `preferences`** (4/4 reviewers flagged as "first-write data loss") — false positive. Migration 005:148 declares `preferences JSONB NOT NULL DEFAULT '{}'`, so no NULL ever exists. The COALESCE "fix" was unnecessary.
- **Whitespace trim for `update_receipt.vendor` / `update_expense.description`** — RARE in practice (Claude doesn't emit reuna-välejä) and NEUTRAL readability. Dropped.
- **Hotel-breakfast → accommodation domain claim** in `save_receipt.skill.md` — not verified against Finnish bookkeeping/Verohallinto practice. Filed as **#36 (Skillien ja työkalujen mukauttaminen Suomen lainsäädäntöön)** for systematic audit.

### Things step 3b did

All three planned changes landed in `agent.rs`:

- **`SYSTEM_PROMPT_WITH_TOOLS` is now in design §6.2 form.** The "CRITICAL RULES" section (per-tool imperatives like "ALWAYS use save_receipt AND add_expense") is gone. The new prompt has the role description, the receipt→expense invariant ("Every receipt must become a billable expense"), the `read_skill`-on-first-use instruction, the one-call-per-message rule, and the existing language/style guidance condensed into a "Language & style" bullet list.
- **One-tool-call-per-message enforcement (design §7.1) is live in the agent loop.** When the model returns a message with N≥2 `tool_use` blocks, the dispatcher executes the first one normally and returns `tool_result(is_error: true)` for blocks 2..N with the message extracted as `BATCHED_TOOL_USE_REJECTION` (`"Only one tool call is allowed per assistant message. Make this call alone in the next message, then continue with subsequent calls one at a time."`). The first block still runs; only the extras are rejected. Tracing emits a `warn` on `llm_errors` for each rejected block with `tool`, `tool_use_id`, `batch_index`, `batch_size` so the rate is observable.
- **`system_prompt_with_tools_contains_key_instructions` test was rewritten.** Positive assertions: prompt contains `matkalaskuassistentti`, `read_skill`, `one tool call per message`, `Finnish`. Negative assertions: prompt does NOT contain `CRITICAL RULES` or `ALWAYS use save_receipt`, so a regression to the pre-step-3b shape fails this test rather than passing silently. Added `batched_tool_use_rejection_message_is_actionable` to pin the rejection wording (`one tool call`, `next message`).
- **`MAX_TOOL_ITERATIONS = 200` was already in place from the 3a follow-up** — no further change needed.

### Decided during step 3b

- **Signature kept multi-line, not the comma-separated form in design §6.2.** Design §6.2 reads `"Ystävällisin terveisin, Grooveserve-tiimi, grooveserve.com"` on a single line. The implementation kept the multi-line form (`"Ystävällisin terveisin,\nGrooveserve-tiimi\ngrooveserve.com"`, three lines) that the old prompt rendered, on the read that the design's commas were a doc-formatting compromise — emails actually render the signature as three lines, and the existing prompt's three-line shape is the production-validated artefact. Revisit if the model produces awkward signatures with the new prompt.
- **Rejection message extracted as a `const`, not inlined.** This makes the message testable (`batched_tool_use_rejection_message_is_actionable`) and keeps the cross-tool-call enforcement code (defense in depth) self-documenting alongside the system prompt instructing single-call-per-message.

### Things step 4 did

The mechanical body-move planned in design §8 step 4. After this step `tools/` has no `handlers.rs` — every tool's logic lives in its own per-tool file alongside its `Tool` impl, schema, and skill markdown.

- **Inlined all 10 handler bodies into their respective `execute` methods.** Receipts (`save_receipt`, `update_receipt`, `list_receipts`), expenses (`add_expense`, `update_expense`, `set_expense_status`, `list_expenses`, `get_draft_summary`), and user (`get_user_context`, `update_user_preferences`) — each per-tool file now imports the helpers it needs from `crate::tools::util` (e.g. `parse_money`, `validate_category`, `RECEIPT_STATUS_VALUES`) directly, and the `execute` body opens with `let pool = runtime.db;` to keep the SQL closer to today's diff-friendly form.
- **Row structs relocated.** `ReceiptRow` is now a private struct in `receipts/list_receipts.rs` (its only consumer). `UserContextRow` is now private to `user/get_user_context.rs`. `ExpenseRow` is shared by `list_expenses.rs` and `get_draft_summary.rs`, so it lives `pub(super)` in `expenses/mod.rs` — the two consumers `use super::ExpenseRow`. No new file is added for the shared row.
- **`handlers.rs` and `mod handlers;` deleted.** `tools/mod.rs` now exports only `context`, `dispatch`, `expenses`, `meta`, `output`, `receipts`, `registry`, `user`, `util`. The dispatch path is unchanged; the `wrappers_fast_fail_with_correct_handler_message` test in `dispatch.rs` still pins the five required-input tools to their bodies (its docstring updated to drop the now-meaningless "wrapper vs handler" framing).
- **Stale comments swept**: `util.rs`'s "until step 4" remark is gone, and `registry.rs`'s reference to the deleted `definitions.rs::phase1_tools()` is rewritten to point at the snapshot test as the order pin.

### Step 3b/4 LLM-review follow-up (7 fix commits)

After step 4 landed, a 4-LLM review (Gemini 3.1 Pro / GPT-5.5 / Claude Opus 4.7 / DeepSeek V4 Pro) ran in `history/review-tool-skills-step3b-step4-impl.md` followed by an `/assess-findings` triage that filtered the findings by likelihood × readability impact. Seven applied; one was spun off to **#37**; everything else was either dropped (mostly RARE+NEUTRAL noise) or already decided in 3b/4.

Highlights of what changed in this round (so the next agent doesn't have to re-derive the rationale):

- **Agent loop cannot corrupt conversation history any more.** Both `StopReason::MaxTokens` and `StopReason::Unknown` now strip dangling `tool_use` blocks from the last assistant message before breaking, via `strip_dangling_tool_use(messages, stop_reason)`. Without this, a model that finished a `tool_use` block before hitting `max_tokens` would have left an unanswered `tool_use` in `loop_messages` → next inbound email replays it → Anthropic 400. All four reviewers concurred this was the highest-impact bug in the diff.
- **`StopReason::ToolUse` with zero tool_use blocks now fails fast** with `AgentError::Permanent`, instead of pushing an empty user message that the next request would 400 on.
- **`add_expense` idempotency key now includes `currency`, `category`, and `expense_type`** in addition to the previous `(message_id, receipt_id, description, expense_date, amount.normalize())`. Previously two same-amount receiptless expenses on the same day in EUR vs USD (or food vs office) collapsed into one row via `ON CONFLICT`, with the second call's input echoed back to the model as "saved". The construction is now in a private `build_idempotency_key` helper with five focused unit tests.
- **`update_receipt` and `update_expense` no longer accept empty-string updates.** New `tools/util.rs::optional_non_empty_str` helper trims and treats empty/whitespace as "no update". All text fields that back a `COALESCE` are routed through it, and `has_update` is computed from the parsed `Option<...>` values so the "at least one field" guard now matches what actually gets bound.
- **`set_expense_status` uses `EXPENSE_STATUS_VALUES` const** instead of duplicating the inline `["draft", "confirmed", "rejected"]` list. Drift prevention.
- **`update_receipt` status flow tidied:** the previous double-read (validate raw `input.get("status")`, then bind the separately-parsed `optional_non_empty_str` value) is consolidated. Empty/whitespace status is now consistently "no update", matching other empty-string fields.
- **`list_expenses` and `list_receipts` now expose a `truncated` indicator** in their output (`{count, truncated, limit}`). They `SELECT limit + 1` rows and clip back to `limit`, so the model can tell "user has exactly 50 expenses" from "user has 200 expenses, here are the first 50". Skill files updated to describe the new shape.
- **System prompt's receipt→expense invariant rewritten** from the absolute "Every receipt must become a billable expense" to "Every receipt belongs to some expense unless the user explicitly says otherwise. ... One expense can carry multiple receipts or vouchers (e.g. a hotel stay where the room and breakfast are billed separately ...)". Two real-world cases the old wording missed: user-explicit opt-out, and 1:N receipt-to-expense (hotel + breakfast). Test pinned with `Every receipt belongs to` and `multiple receipts` substring assertions.
- **`dispatch.rs::wrappers_fast_fail_with_correct_handler_message` docstring** stopped claiming "Rust's type system" covers SQL column names — it doesn't, `sqlx::query_as` is runtime-checked. New caveat block lists what the test actually covers (first required-field validation for the five mutation tools) and what it doesn't (list/get tools, post-validation paths).
- **`update_user_preferences.skill.md` clarifies null semantics**: bare `{"preferences": null}` is rejected by the no-update guard; only nested `{"preferences": {"foo": null}}` makes it through to the JSONB merge.

### Spin-off filed: #37

The "agent cannot clear an optional field" limitation (M1 in the review report) was confirmed and accepted as a real correctness gap, but the fix requires building a dynamic SQL pattern across three update tools — too large to bundle with the review-fix commits. Filed as **#37 (Update-työkalut eivät voi tyhjentää valinnaisia kenttiä)** with three implementation options (separate `clear_*` tool, sentinel value, or dynamic SQL builder — option 3 recommended).

### Decided during step 4

- **`ExpenseRow` lives in `expenses/mod.rs`, not a new `row.rs` file.** Two consumers don't justify a separate file in MVP. `pub(super)` keeps it scoped to the `expenses` directory; nothing outside imports it.
- **`UserContextRow`'s `_input` parameter is now underscore-prefixed.** The `get_user_context` body never reads `input` (its schema declares no properties), and the old wrapper passed it through `handlers::get_user_context` which also ignored it. Inlining made the unused binding explicit; renaming to `_input` documents the intent at the trait-impl boundary instead of leaving an unused-variable warning to the rustc.
- **Step 5 is now blocked on production data.** Step 5 in design §8 is "final pass on system prompt and skill content based on observations from step 3 in production". The structural items it listed (delete `definitions.rs` and `handlers.rs`) are already done — `definitions.rs` went in step 2, `handlers.rs` here. What remains needs telemetry from the new prompt actually running on real emails, which can only happen after this branch merges. The status table reflects that.

### Review findings dropped, with rationale

The LLM review of step 2 (history/review-tool-skills-step2-impl.md) raised several findings that were deliberately dropped, not deferred:

- **`Box<dyn Tool>` for ZSTs** — cosmetic; works fine, refactor-by-momentum not justified.
- **Schema validation depth** — Anthropic's API rejects malformed schemas on first request with a clear error; reimplementing JSON-Schema validation locally adds defensive code without correctness payoff.
- **`&mut ToolRuntime` is premature** — yes, the `&mut` does no useful work today. But the cost of dropping it is one PR touching 11 files, and the trait will likely change at #34 anyway. Either way `&mut` was free insurance that turned out unneeded; net cost of leaving it is ~zero.
- **`registry()` leaks `Box<dyn Tool>`** — couples with the ZST point above; if both ever matter, fix together in step 4.
- **Cache `anthropic_tools()` in `LazyLock`** — RARE likelihood (current schemas all `json!()` literals, fully deterministic), NEUTRAL readability, defensive against a class of bug we don't have. Drop.
- **Reserve `read_skill` name in `validate()`** — RARE likelihood (only triggers if someone deliberately names a tool `read_skill`), NEUTRAL readability (a guard line that itself becomes obsolete in step 3a). Drop.
- **Smoke-test calls `anthropic_tools()` too** — `json!()` macros don't panic at runtime. Drop.

## See

- `design.md` — full design proposal (§5.1 amended in 3a follow-up to match the JSON-wrap decision)
- `history/review-tool-skills-step2-impl.md` — LLM review of the step 2 implementation, used to drive the review-fix commits
- `history/review-tool-skills-step3a-impl.md` — LLM review of the step 3a implementation (4-reviewer critique + 2-round cross-review + moderator triage), used to drive the 3a follow-up commits
- `history/review-tool-skills-step3b-step4-impl.md` — LLM review of the step 3b + step 4 implementation, used to drive the post-4 review-fix commits (and #37 spin-off)
- **#36** Skillien ja työkalujen mukauttaminen Suomen lainsäädäntöön — domain-claim audit spawned from the 3a review's hotel-breakfast finding
- **#37** Update-työkalut eivät voi tyhjentää valinnaisia kenttiä — dynamic-SQL field-clearing limitation, spawned from the 3b/4 review's "Cannot unset fields" finding
