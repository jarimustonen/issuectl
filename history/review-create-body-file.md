## Review: create `--body-file` structured Markdown fix

**Reviewed:** complete `main..HEAD` production, test, changelog, and agent-contract diff across `write.rs`, `mutate/{new_issue,new_api,intake}.rs`, `recurrence.rs`, `transfer.rs`, CLI dispatch/help/tests, and all shipped/dogfooded `/issue` templates
**Reviewers:** `gemini-3.1-pro-preview`, `gpt-5.6-sol`, `claude-fable-5`, `deepseek-v4-pro`
**Rounds:** independent review plus two complete cross-review rounds. Anthropic received one bounded context follow-up. DeepSeek's independent response was truncated in transport, then successfully completed on its existing thread before both cross-review rounds; all four final positions are present.

### Critical Issues (Consensus)

No production blocker survived verification and cross-review.

### Warranted In-Patch Improvements

1. **Structured-body schema completion lacked a positive regression**
   - **What:** the horizontal-rule test proved that existing required sections are not duplicated, but did not prove that a genuinely missing section is appended with canonical spacing.
   - **Where:** `crates/issuectl-core/src/mutate/new_issue.rs`, `structured_body_with_horizontal_rule_does_not_duplicate_required_sections`.
   - **Why it matters:** the agent-facing contract explicitly says repository schemas may append missing required H2 stubs.
   - **Resolution:** expanded the core mutation regression to require `Quick Test`, omit it from the supplied structured body, and assert one correctly separated appended stub while preserving the horizontal rule.
   - **Raised by:** all four reviewers in their final positions.

2. **Agent guidance overstated default epic rendering**
   - **What:** the templates first acknowledged schema-generated stubs, then categorically said the CLI does not write the recommended epic sections.
   - **Where:** `crates/issuectl-core/templates/issue-{skill,prompt}.md` and both dogfooded copies.
   - **Why it matters:** a configured repository schema can append those sections, so the categorical wording was imprecise.
   - **Resolution:** qualified the sentence as default-renderer behavior and preserved byte-equivalent Claude/Codex bodies plus rendered dogfooded copies.
   - **Raised by:** OpenAI; accepted by the moderator after direct comparison with the schema-completion path.

### Confirmed Follow-Up Defect

1. **Issuectl JSON export-to-import duplicates structured body headings**
   - **What:** JSON export emits the complete structured `Issue.body`; import accepts `body` as an alias for free-text `description`, then adds a generated `## Description` wrapper. Re-import can therefore produce duplicate Description headings (and preserves any exported H1 content inside the new body).
   - **Where:** `crates/issuectl-core/src/transfer.rs`, `ImportRecord.description` and `ImportRecord::into_new_args`.
   - **Why it matters:** the module explicitly supports parsing issuectl's own JSON export, while its current “round trip” test stops before rendering the imported issue.
   - **Scope:** real and pre-existing. This task explicitly requires import semantics to remain unchanged, so it must not be silently folded into the create-only patch. It warrants a separate intake issue and design of how `body` versus `description` selects structured/free-text mode.
   - **Raised by:** Anthropic and DeepSeek independently; Gemini agreed after cross-review; OpenAI confirmed it but correctly classified it out of scope.

### Disputed Issues

1. **Should the changelog entry move from Fixed to Changed?**
   - **For:** Anthropic and DeepSeek argued that wrapper-free plain-prose files are an intentional semantic change and migration-visible.
   - **Against:** Gemini and OpenAI observed that this is the exact corrected `create --body-file` contract and the entry precisely scopes itself to that bug fix.
   - **Moderator's take:** keep it under Fixed. Correcting persisted output necessarily changes behavior; Keep a Changelog's Fixed category is appropriate when the entry plainly states the new body-file semantics.

2. **Should heading-less structured bodies warn or error?**
   - **For:** Anthropic and DeepSeek argued that, when a schema requires Description, heading-less prose remains before an appended empty stub.
   - **Against:** Gemini and OpenAI noted that Markdown bodies need not contain H2 headings and the declared contract makes the file responsible for its structure; schema completion correctly appends sections that are actually absent.
   - **Moderator's take:** do not add content sniffing, warnings, or rejection. Such behavior would penalize valid Markdown and introduce a second implicit body-mode grammar. The positive schema-stub test now pins the intended behavior.

3. **Should `description: Option<String>` plus `structured_body: bool` become an enum now?**
   - **For:** all reviewers noted that the pair permits `None + true` and makes every creation path maintain a cross-field invariant.
   - **Against:** every current constructor is explicit and correct, empty CLI files are rejected, and replacing both owned and borrowed argument shapes would broaden this targeted fix without a demonstrated production failure.
   - **Moderator's take:** valid design debt, but too broad for this patch and below the filing bar without an observed consequence. Keep the explicit compatibility assignments and focused tests.

### Minor and Dropped Findings

- Trailing document whitespace normalization is pre-existing, deliberate, and shared with body replacement; it is not introduced by structured mode.
- Empty/whitespace-only body files are already rejected before mutation.
- The alleged missing blank line before schema stubs was false; the append path explicitly ensures `\n\n`.
- Gemini's claim that `item_text::split` was a no-op was false and retracted: `render_new_item_from_fm` includes serialized frontmatter.
- OpenAI's temporary claims of template drift and a missing test-constructor field resulted from a reduced cross-round attachment set; the full tree disproved both, and OpenAI retracted them.
- Intake, recurrence, core API, and foreign-import free-text modes are intentional compatibility behavior required by this task.
- A no-H1 rule, unclosed-fence rejection, broader CommonMark heading grammar, duplicate-precheck rendering, and test renames are unrelated hardening or cleanup without a demonstrated regression here.

### What's Solid

- The bug reproduced on the pre-fix `main`: a file beginning with `## Description` received an additional empty generated Description heading.
- Runtime records the input source before consuming it, so file/stdin bodies become structured while inline descriptions retain the wrapper.
- The canonical frontmatter splitter prevents body horizontal rules from truncating required-section detection.
- Source placement, file/stdin dispatch, wrapper suppression, inline compatibility, and schema completion are covered at the appropriate core/process boundaries.
- All non-create call sites explicitly preserve their existing semantics; locking and schema validation remain centralized in core mutation code.
- Shipped Claude/Codex templates and dogfooded copies remain synchronized.

### Moderator's Assessment

OpenAI gave the strongest final review: it corrected its attachment-driven mistakes, separated declared behavior from regressions, and kept the scope disciplined. Anthropic contributed the most important new out-of-scope defect by tracing issuectl's own JSON export into import rendering. DeepSeek persistently stress-tested schema interactions, although it continued to overstate heading-less bodies as corruption after the contract was clarified. Gemini made two substantive factual mistakes and explicitly retracted both.

The single most important in-patch action was adding the positive missing-section regression. The only confirmed follow-up with credible real-world impact is the pre-existing JSON export/import heading duplication; it should be filed unlaned through intake with this run's review provenance.
