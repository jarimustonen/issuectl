# Design: Skill-based tool architecture

_Issue: #33 · Status: draft v2 · Updated: 2026-04-28_

## 0. Operating context

This is an **MVP design**. Per the root `AGENTS.md` design principles:

- **Functional correctness is the only first-class goal.** Cost, token, and latency optimizations come *after* the system works correctly.
- **The user-visible system is not latency-critical.** The primary channel is email — the user submits a message and does not expect an instant response. Multi-second or even multi-minute agent loops are acceptable.
- **10–30 % token savings are not a basis for architectural decisions in this phase.**

This explicitly justifies design choices that an optimization-minded reviewer would otherwise reject. In particular, this design accepts extra LLM round-trips (`read_skill` calls) and a larger conversation history footprint as a price for clearer per-tool guidance and lower cross-tool coupling. Optimizations are deferred to follow-up issues:

- **#34** — compound (transactional) tools (e.g. `save_receipt_expense`).
- **#35** — proactive skill injection into the system prompt.

## 1. Goals & non-goals

**Goals**

- Each tool is a self-contained module: code, schema, and a markdown skill file describing usage.
- Adding a new tool means adding one directory + one line in the registry list.
- Each tool has a dedicated, focused skill the agent can read on demand.
- The system prompt remains short and *general* — it describes the agent's role and the operational style, not specific tool sequences.
- Pragmatic, incremental migration: existing tools keep working while we move them one at a time. Single dispatcher path from day 1.

**Non-goals (explicitly deferred)**

- Token cost / latency optimization — not a goal in MVP. Skills add round-trips; that's accepted.
- Compound transactional tools — see #34.
- Proactive skill injection / trigger words — see #35.
- Hot-reloading skills at runtime. Skills are embedded into the binary via `include_str!`.
- Per-tenant skill overrides.
- Locale-aware skill loading. Skills are written in **English** so they match the agent's internal reasoning language regardless of user-facing language.
- Replacing the agent loop or conversation model.

## 2. Background — current state

Today (`services/email/src/tools/`, `services/email/src/agent.rs`):

- `tools/definitions.rs` — `phase1_tools()` returns a hand-coded `Vec<Value>` of 10 JSON-Schema tool definitions.
- `tools/handlers.rs` — every handler function (`save_receipt`, `add_expense`, …) in one ~1000-line file.
- `tools/mod.rs` — a `match tool_name { ... }` dispatcher.
- `agent.rs` — `SYSTEM_PROMPT_WITH_TOOLS` const string: ~50 lines including hard-coded cross-tool rules ("ALWAYS use save_receipt AND add_expense"), cached via ephemeral cache.

Adding a tool today touches at minimum: `definitions.rs`, `handlers.rs`, `mod.rs`, and often `agent.rs`. Cross-tool rules are baked into the prompt and grow as we add tools.

## 3. Skill file format and location

### 3.1 Format

Skill files are markdown. Skill files are written in **English** with Finnish domain terms preserved (matkalasku, kuitti). This matches the existing system prompt's language pattern and avoids steering English-speaking users with Finnish prose.

```markdown
# save_receipt

## When to use

Call this tool whenever you receive receipt or invoice data from the user, either
- as an attachment that has been pre-processed (extraction summary appended to the user message), or
- as text the user typed describing a receipt.

Do not call this tool to "save" something the user has only asked about (e.g. "what would you save?"). The user message must contain a real receipt.

## When NOT to use

- For mileage, per-diem, or meal-allowance items — those go straight to `add_expense` with the appropriate `expense_type`.
- To update an already-saved receipt — use `update_receipt` instead.

## Parameters

The JSON schema is the source of truth for parameter types and required fields. This section explains semantics and edge cases that don't fit in schema descriptions:

- `vendor`: trim whitespace; use the merchant name as printed.
- `category`: prefer `accommodation` for hotel-chain breakfast receipts even though the line items are food.
- `extraction_id`: pass when the receipt came from a pre-processed attachment so the receipt links back to the OCR record.
- `items[]`: pass when the OCR extraction provides line items.

## Output shape

On success, `data` contains `{ receipt_id: <int>, vendor, total_amount, currency }`. The returned `receipt_id` is the input to a follow-up `add_expense` call.

## Common patterns

After `save_receipt` succeeds, call `add_expense` with `receipt_id` set. Receipts only become billable when an expense row exists.

If the user message contains multiple receipts, save each one fully (save_receipt → add_expense pair) before moving to the next. The agent loop runs tool calls sequentially, so chains are deterministic.

## Edge cases

- If `total_amount` is missing from the extraction, ask the user — do not invent a value.
- Negative amounts: never. If you see a refund, ask the user how to handle it.
```

The skill body is pure markdown. There is **no YAML frontmatter** — metadata that the registry needs (`summary`, `related`, `required_after`) lives as `Tool` trait methods, decided at implementation time. This avoids a YAML parser dependency, eliminates LLM-visible metadata noise, and gets compile-time validation for free.

### 3.2 Location

Skills live next to the tool code:

```
services/email/src/tools/
├── mod.rs               # registry + dispatch (explicit list)
├── context.rs           # ToolContext, ToolOutput, ToolRuntime
├── meta/
│   ├── mod.rs
│   ├── read_skill.rs
│   └── read_skill.skill.md
├── receipts/
│   ├── mod.rs
│   ├── save_receipt.rs
│   ├── save_receipt.skill.md
│   ├── update_receipt.rs
│   ├── update_receipt.skill.md
│   ├── list_receipts.rs
│   └── list_receipts.skill.md
├── expenses/
│   ├── mod.rs
│   ├── add_expense.rs
│   ├── add_expense.skill.md
│   └── ...
└── user/
    ├── mod.rs
    ├── get_user_context.rs
    ├── get_user_context.skill.md
    ├── update_user_preferences.rs
    └── update_user_preferences.skill.md
```

Skills are embedded via `include_str!` so the binary remains a single artifact. The cross-compile pipeline (#23) doesn't change.

## 4. Tool registration mechanism

### 4.1 The `Tool` trait

```rust
// services/email/src/tools/mod.rs

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    /// Tool name (as the LLM sees it). Must be unique.
    fn name(&self) -> &'static str;

    /// One-line description that goes into the Anthropic `tools` array
    /// (the `description` field). The agent sees this on every request,
    /// so descriptions for tools with cross-tool dependencies must include
    /// a routing hint (e.g. "Required before add_expense when the user
    /// provided a receipt"). Keep under ~200 chars.
    fn description(&self) -> &'static str;

    /// JSON Schema for `input_schema`.
    fn input_schema(&self) -> serde_json::Value;

    /// The skill body as a markdown string. Returned by `read_skill`.
    /// Typically `include_str!("./save_receipt.skill.md")`.
    fn skill(&self) -> &'static str;

    /// Execute the tool.
    ///
    /// `runtime` is `&mut` so future iterations can carry an active
    /// transaction handle (#27 / #34) without breaking the trait surface.
    async fn execute(
        &self,
        runtime: &mut ToolRuntime<'_>,
        ctx: &ToolContext,
        input: serde_json::Value,
    ) -> ToolOutput;
}
```

`ToolRuntime` holds shared dependencies (DB pool, plus any HTTP clients / config the future roadmap needs). The exact shape is an implementation detail — we'll start with `{ db: &PgPool }` and grow it as #15/#16/#18/#19/#20 land. The point is that the trait does not bake `&PgPool` directly, and the `&mut` reference leaves room for #27/#34's transaction-scoped runtime without a trait-breaking change.

Optional metadata (e.g. `summary`, `related`, `required_after`) can be added as default-implemented trait methods if needed during implementation. They are not required by this design.

### 4.2 Explicit registry — no `inventory`

We deliberately avoid the `inventory` crate. Linker-based distributed slices have DCE/LTO/cross-target risks (cross-compile pipeline #23 uses different linkers from dev) and nondeterministic ordering. For ~20 tools, an explicit list is simpler and verifiably correct.

```rust
// services/email/src/tools/mod.rs

use std::sync::LazyLock;

static TOOLS: LazyLock<Vec<Box<dyn Tool>>> = LazyLock::new(|| {
    let tools: Vec<Box<dyn Tool>> = vec![
        // Order matches the existing `phase1_tools()` order so step 2 of the
        // migration does not change what the model sees. New tools are added
        // at the end. `read_skill` is the meta-tool and listed first.
        Box::new(meta::ReadSkill),
        Box::new(receipts::SaveReceipt),
        Box::new(receipts::UpdateReceipt),
        Box::new(receipts::ListReceipts),
        Box::new(expenses::AddExpense),
        Box::new(expenses::UpdateExpense),
        Box::new(expenses::SetExpenseStatus),
        Box::new(expenses::ListExpenses),
        Box::new(expenses::GetDraftSummary),
        Box::new(user::GetUserContext),
        Box::new(user::UpdateUserPreferences),
    ];
    validate(&tools).expect("invalid tool registry");
    tools
});

pub fn registry() -> &'static [Box<dyn Tool>] {
    TOOLS.as_slice()
}

pub fn get(name: &str) -> Option<&'static dyn Tool> {
    TOOLS.iter().find(|t| t.name() == name).map(|b| b.as_ref())
}

/// Build the JSON tool array Anthropic expects.
/// Called from agent.rs in place of the deleted `definitions::phase1_tools()`.
pub fn anthropic_tools() -> Vec<serde_json::Value> {
    registry()
        .iter()
        .map(|t| serde_json::json!({
            "name": t.name(),
            "description": t.description(),
            "input_schema": t.input_schema(),
        }))
        .collect()
}
```

Validation (run once at first access):
- Tool names are unique.
- Tool names match `^[a-z][a-z0-9_]{0,63}$`.
- Skill body is non-empty.
- Description is non-empty and under a reasonable size limit.
- `input_schema` is a JSON object with `"type": "object"`.
- No tool name collides with `read_skill`.

A startup smoke test in `main.rs` calls `registry()` so validation runs at boot, not on the first inbound email.

### 4.3 Adding a new tool

End-to-end checklist for a new `lookup_tax_rate` tool:

1. Create `services/email/src/tools/tax/lookup_tax_rate.rs` with a `LookupTaxRate` struct implementing `Tool`.
2. Create `lookup_tax_rate.skill.md` next to it.
3. Add `pub mod lookup_tax_rate;` to `tax/mod.rs` (and `pub mod tax;` to `tools/mod.rs` if the directory is new).
4. Add one line to the `TOOLS` list: `Box::new(tax::LookupTaxRate),`.

Four edits, all local, all in the same logical area. The `TOOLS` list edit is the deliberate trade-off: explicit is safer than `inventory` magic, and one line per tool is acceptable.

## 5. The `read_skill` meta-tool

### 5.1 Behavior

```json
{
  "name": "read_skill",
  "description": "Read the detailed usage guide for a tool. Call this before using a tool you have not used yet in this conversation.",
  "input_schema": {
    "type": "object",
    "properties": {
      "tool": { "type": "string", "description": "The name of the tool whose skill you want to read." }
    },
    "required": ["tool"],
    "additionalProperties": false
  }
}
```

Behavior:

- Returns the markdown body of the skill file inside the standard `ToolOutput::success(json!({"markdown": ...}))` envelope, the same shape every other tool uses.
- If the tool name is unknown, returns `ToolOutput::error("Unknown tool: <name>. Available: <comma-separated list>")`.
- `read_skill` itself does **not** have a `*.skill.md` file. The system prompt explains how `read_skill` works directly, so a skill-of-skill would be dead code. `read_skill` is also exempt from the "call `read_skill` before first use" rule — it's the bootstrap.

**Wire format — JSON-wrapped, not raw markdown.** An earlier draft of this design proposed bypassing `serde_json::to_string` for `read_skill` so the model received raw markdown in `tool_result.content`. Implementation chose the simpler path: serialize the markdown as a JSON string under a `markdown` key, exactly like every other tool's `data` payload. Per §0 (functional correctness only, no token-or-bytes optimisation in MVP), the model parses JSON-escaped markdown without trouble, and a one-off dispatcher carve-out for `read_skill` adds asymmetry no other tool needs. Revisit only if MVP empirically shows the agent struggling with JSON-escaped skill bodies.

### 5.2 No `list_skills`

We do **not** add a `list_skills` meta-tool. The Anthropic Messages API requires sending the entire `tools` array (each with `name` + `description`) on every request. The agent already has the catalog in context. A `list_skills` tool would be a round-trip to retrieve information already in the request.

If the API ever changes such that not all tools are sent up-front, revisit then.

### 5.3 Why `read_skill` is the right pattern here

The skill exists because each tool has details — when to use it, when *not* to use it, parameter semantics that don't fit in schema descriptions, edge cases, output shape — that are too verbose to put in the system prompt for *every* tool simultaneously, and that the agent only needs when it's about to use the tool.

The pattern is: **for a tool you are about to use, call `read_skill` first to get the detailed instructions.**

This is acceptable because:

- The system is **not latency-critical** (per §0). An extra round-trip is fine.
- We are **not optimizing tokens** in MVP (per §0). Persisting the skill body in conversation history for the rest of the conversation is fine.
- It keeps the system prompt focused on *role and style* rather than per-tool minutiae.
- It localizes per-tool details to per-tool files, which is the maintainability benefit of the whole design.

The cost — extra LLM iterations and token usage — is the deliberate price we pay for clearer separation. Issue #35 will revisit this once we have telemetry from the running system.

## 6. System prompt changes

### 6.1 Before (today)

The current prompt mixes role, style, and *per-tool imperatives*:

```
You are the Grooveserve expense report assistant (matkalaskuassistentti). [...]

Your capabilities:
- Receive receipts and travel information from users
- Save receipts and expenses to the database using your tools
- ...

CRITICAL RULES — you MUST follow these:
1. ALWAYS use save_receipt AND add_expense tools when you receive receipt/invoice data. ...
2. When listing expenses, ALWAYS call get_draft_summary or list_expenses. ...
3. When the user asks to update or correct data, ALWAYS call the update tool. ...
4. After tool calls succeed, confirm what was saved with the data from tool results.

Guidelines:
- Always reply in Finnish unless the user writes in another language
- ...

Writing style:
[~30 more lines]
```

The "CRITICAL RULES" section grows every time we add a tool with a usage subtlety.

### 6.2 After

The prompt now describes the agent's role and operating style. It does not list per-tool sequencing rules — those move into the relevant skill files, which the agent reads on demand.

```
You are the Grooveserve expense report assistant (matkalaskuassistentti). You help
users manage their travel and expense reporting via email. You handle receipts,
invoices, mileage, and per-diem entries that users send you, and you persist them
carefully and completely so that nothing the user reports is lost or partially
saved.

Every receipt must become a billable expense — saving a receipt is only the first
step; the corresponding expense row must also be created. The same applies to
corrections: when receipt data changes, the linked expense data changes too.

You have a set of tools available — see the `tools` array. Each tool has a one-line
description that includes routing hints (e.g. which tool to call first when there
is a chain). The detailed usage guide for a tool is available via the `read_skill`
meta-tool. For a tool you are about to use that you have not used yet in this
conversation, call `read_skill` first to get the detailed instructions on its
usage patterns.

Make exactly one tool call per message. Do not batch multiple tool calls in a
single message. After each call, wait for the result, then make the next call in
your next message.

Never fabricate data — call the appropriate tool or ask the user. After every
database mutation, confirm what was saved using the data from the tool result.

Language & style:
- Reply in Finnish unless the user writes in another language.
- Write like a real person writing an email. Plain text, no headings, no code blocks.
- Bullet lists only for concrete items (amounts, options).
- Use tables for expense summaries: `| # | Kulu | Summa | ALV |`.
- Finnish number format to users (24,50 €), decimal points in tool calls (24.50).
- End every reply with: "Ystävällisin terveisin, Grooveserve-tiimi, grooveserve.com".
- Never use raw URLs — always descriptive link text: [avaa matkalasku](url).
```

Notes:

- The role description ("you handle receipts...you persist them carefully and completely") replaces the per-tool "ALWAYS use save_receipt AND add_expense" imperative with a general operating principle. The detail of *how* to persist completely lives in the relevant skill files.
- "For a tool you are about to use that you have not used yet in this conversation, call `read_skill` first" makes the pattern explicit. The pattern is what the agent is meant to follow.
- This prompt does **not** scale linearly with tool count. Adding tools doesn't add lines here — it adds skill files.

## 7. Agent loop changes

### 7.1 One tool call per assistant message

The Anthropic API allows the assistant to emit multiple `tool_use` blocks in a single message. The dispatcher rejects any batch with more than one. **Exactly one tool call per assistant message**, full stop.

If the model returns a message with N > 1 `tool_use` blocks, the dispatcher returns `tool_result` with `is_error: true` for the second through Nth block, with content like:

```
Only one tool call is allowed per assistant message. Make this call alone in
the next message, then continue with subsequent calls one at a time.
```

The first block is still executed normally.

Rationale: per §0 we are not optimizing latency. Multiple tool calls in one batch invite two kinds of correctness bugs:

1. **Skipped skill reads.** If the model batches `read_skill { tool: "save_receipt" }` and `save_receipt { ... }` together, it has not actually seen the skill content before choosing parameters — it predicted them.
2. **Invented dependent IDs.** If the model batches `save_receipt` and `add_expense { receipt_id: 42, ... }` together, the `receipt_id` is fabricated because `save_receipt`'s result has not been returned yet.

Restricting to one call per message eliminates both. The system prompt also instructs this directly so the model rarely tries to batch in the first place — the dispatcher rule is defense in depth.

This is intentionally conservative for MVP. We can relax it later (e.g. allow multiple `read_skill` calls in a batch when they are genuinely independent) if we hit a real workflow that needs it. Until then, simplicity wins.

### 7.2 Iteration limit

`MAX_TOOL_ITERATIONS = 10` was set conservatively for a tighter old loop. The new loop will use more iterations: a typical receipt workflow becomes `read_skill(save_receipt)` → `save_receipt` → `read_skill(add_expense)` → `add_expense` → final reply (5 iterations). Multi-receipt or correction emails can easily push higher, and as the tool surface grows we expect tools to become more granular (more, smaller tools per workflow).

We **raise the limit to 200**. This is generous enough that legitimate workflows don't hit it even with many small tools, while still preventing genuinely runaway loops (e.g. a buggy tool returning a soft error that the model retries forever, blocking the IMAP worker).

The limit is **never removed**. A bound of some kind is required for safety; the threshold is just calibrated for the new pattern. A loop terminating at 200 iterations is still a permanent failure (`AgentError::Permanent`) for that email.

## 8. Migration path

The migration is incremental but uses a **single dispatcher path from day 1**. We do not run two registries in parallel.

### Step 1 — preconditions

Tests pass on `main`, no in-flight changes to `tools/`. (Already true.)

### Step 2 — introduce the `Tool` trait + wrap *every* legacy handler

Add `Tool` trait, `ToolRuntime`, `ToolContext`, `ToolOutput`, `LazyLock<Vec<Box<dyn Tool>>>` registry, and `anthropic_tools()` helper to `tools/mod.rs`.

For *every* existing tool, create a `Tool` impl that:

- Returns the existing description and JSON schema (lifted from `definitions.rs`).
- Returns a placeholder skill string (`"# save_receipt\n\nSkill not yet written."`) — to be replaced in step 3.
- In `execute`, calls the existing handler in `handlers.rs`.

In `agent.rs`:

- Replace `let tool_definitions = crate::tools::definitions::phase1_tools();` with `let tool_definitions = crate::tools::anthropic_tools();`.
- Replace the call site that goes into `tools::execute_inner`'s `match`: dispatch now uses `tools::get(name)` and the resulting trait object's `execute` method.

The legacy `definitions.rs::phase1_tools()` and the `match tool_name { ... }` dispatcher are deleted in the same PR. From here on, *all* tool definitions and dispatch flow through the registry.

This step is one PR. It is purely structural: behavior is unchanged. The registry list is intentionally not sorted — its order matches the existing `phase1_tools()` ordering so that what the model sees in the `tools` array is exactly the same as before.

A golden-file test verifies that `anthropic_tools()` produces the same JSON as the deleted `phase1_tools()` (same names, same descriptions, same schemas, same order).

### Step 3a — add `read_skill` and write real skills, keep old prompt

Implement `read_skill` as a `Tool` impl in `tools/meta/read_skill.rs` and add it to the registry.

Write a real skill body for each existing tool — replacing the placeholder strings from step 2. The skill content captures the per-tool usage guidance that previously lived in the system prompt or only in the developer's head.

**Do not change `SYSTEM_PROMPT_WITH_TOOLS` yet.** The agent *can* call `read_skill` after this step (the tool is registered, the skills are populated), but the prompt still has the old imperatives, so the agent's behavior is essentially unchanged. This proves out the infrastructure without flipping behavior.

### Step 3b — swap the system prompt

Replace `SYSTEM_PROMPT_WITH_TOOLS` with the §6.2 form. The cross-tool imperatives ("ALWAYS use save_receipt AND add_expense") are removed; the new prompt has the role description, the receipt→expense invariant, and the `read_skill`-on-first-use instruction.

This is the behavioral change. It is a one-file, ~50-line diff. We expect to discover issues here; iterate on prompt wording as needed. Skill content can also be tuned without changing the prompt.

Splitting 3a/3b means infrastructure rollout and behavioral rollout are separate commits — easier to review, easier to revert one without the other.

### Step 4 — move handler bodies into per-tool files

Now that the trait is the single source of truth, move each handler's body from `handlers.rs` into the corresponding per-tool file (`tools/receipts/save_receipt.rs` etc.). One commit per logical group (receipts, expenses, user) is fine — each is purely a code-organization change with no behavioral effect.

After step 4, `handlers.rs` and `definitions.rs` are gone.

### Step 5 — clean up

- Delete `definitions.rs` and `handlers.rs`.
- `tools/mod.rs` keeps only the trait, contexts, registry, dispatch, and validation.
- Final pass on the system prompt and skill content based on observations from step 3.

### Why this ordering matters

Step 2 (wrap-everything-then-flip) avoids the "duplicate tool name in API request" footgun. There is never a moment where two registries are alive. Each step is reviewable, testable, and revertable.

## 9. Worked example — `save_receipt`

The complete files for `save_receipt` after migration:

### 9.1 `services/email/src/tools/receipts/save_receipt.rs`

```rust
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::{Tool, ToolContext, ToolOutput, ToolRuntime};

#[derive(Default)]
pub struct SaveReceipt;

#[async_trait]
impl Tool for SaveReceipt {
    fn name(&self) -> &'static str { "save_receipt" }

    fn description(&self) -> &'static str {
        "Save a receipt extracted from an attachment or described by the user. \
         Required before add_expense when the user provided a receipt."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "vendor":         { "type": "string",  "description": "Merchant name" },
                "receipt_date":   { "type": "string",  "description": "YYYY-MM-DD" },
                "total_amount":   { "type": "number",  "description": "Total in receipt currency" },
                "currency":       { "type": "string",  "default": "EUR" },
                "category":       {
                    "type": "string",
                    "enum": ["food","accommodation","transport","fuel","parking",
                             "software","telecom","office","other"]
                },
                "payment_method": { "type": "string", "enum": ["card","cash","invoice"] },
                "items":          { "type": "array",  "items": { /* ... */ } },
                "raw_text":       { "type": "string" },
                "extraction_id":  { "type": "integer" },
                "confidence":     { "type": "number" }
            },
            "required": ["vendor", "total_amount", "category"]
        })
    }

    fn skill(&self) -> &'static str {
        include_str!("./save_receipt.skill.md")
    }

    async fn execute(
        &self,
        runtime: &mut ToolRuntime<'_>,
        ctx: &ToolContext,
        input: Value,
    ) -> ToolOutput {
        // (body identical to today's handlers::save_receipt, using runtime.db
        // instead of a bare PgPool argument)
        # unimplemented!()
    }
}
```

### 9.2 `services/email/src/tools/receipts/save_receipt.skill.md`

(Shown in §3.1 above.)

### 9.3 What the agent sees

In the `tools` array (sent on every request):

```json
{
  "name": "save_receipt",
  "description": "Save a receipt extracted from an attachment or described by the user.",
  "input_schema": { /* schema */ }
}
```

A typical loop on first use of `save_receipt` in a conversation:

```
turn 1 assistant → tool_use: read_skill { tool: "save_receipt" }
turn 1 user      → tool_result: <skill markdown>
turn 2 assistant → tool_use: save_receipt { vendor: "...", ... }
turn 2 user      → tool_result: { ok: true, data: { receipt_id: 42, ... } }
turn 3 assistant → tool_use: read_skill { tool: "add_expense" }
turn 3 user      → tool_result: <skill markdown>
turn 4 assistant → tool_use: add_expense { receipt_id: 42, ... }
turn 4 user      → tool_result: { ok: true, data: { expense_id: 17, ... } }
turn 5 assistant → text: "Tallensin kuitin..." (final reply)
```

Five iterations for a single receipt. That's why `MAX_TOOL_ITERATIONS` is raised in §7.2.

When the agent has already used `save_receipt` and `add_expense` earlier in the conversation, the skill reads are skipped — Anthropic sees them in the prior conversation and reuses the knowledge.

## 10. Open implementation questions (decided at implementation time)

These are intentionally not pinned in the design. They are implementation decisions:

1. **Whether to add `summary()`/`related()`/`required_after()` as default trait methods.** Add if they help; skip if they don't. Not required for the MVP design.
2. **`ToolRuntime` field set.** Starts as `{ db: &PgPool }`. Grows as needed when #15/#16/#18/#19/#20 land. Mutability is fixed (`&mut`), but the field set is open.
3. **Typed input parsing.** Each handler defines its own `#[derive(Deserialize)]` input struct and parses `serde_json::Value` at the top of `execute`. **Use a shared helper** so invalid-input errors have a uniform format:

   ```rust
   pub fn parse_tool_input<T: DeserializeOwned>(
       tool_name: &str,
       input: Value,
   ) -> Result<T, ToolOutput> {
       serde_json::from_value(input).map_err(|e| {
           ToolOutput::error(format!("Invalid input for {tool_name}: {e}"))
       })
   }
   ```

4. **`needs_info` should not be `is_error: true`.** Today's `agent.rs` treats every `ok == false` as `tool_error` (`is_error: true`). `ToolOutput::needs_info(...)` is a valid clarification flow, not a failure — the model should not see it as an error. The dispatcher should serialize `needs_info` as a normal `tool_result` with structured content explaining what to ask the user. Pin this at implementation; it is a tightening of the existing dispatcher, not a new abstraction.
5. **Skill body minimum/maximum size.** Add CI lint if and when bloat becomes an issue.

## 11. Success criteria

- All 10 existing tools migrated with no behavior regressions on the existing test suite.
- New tools can be added by creating one directory + one registry line.
- Adding a tool no longer requires editing the system prompt.
- The system prompt describes role and style; it does not list per-tool imperatives.
- Tool-specific guidance lives in skill files; the agent reads them via `read_skill` before first use.
- The agent loop completes typical multi-tool email workflows without exhausting iterations.

We do **not** measure token usage, latency, or call counts as success criteria for this issue. Those become relevant in #35.

## 12. Follow-ups (deferred from this design)

- **#34 — compound transactional tools** (e.g. `save_receipt_expense`) once the tool system is stable.
- **#35 — proactive skill injection** if telemetry shows `read_skill` round-trips are a real problem after MVP ships.
- Tool-array prompt caching (`cache_control` on the `tools` array) — separate small issue, independent of #33.
