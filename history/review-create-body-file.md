## Review: create `--body-file` structured Markdown fix

**Reviewed:** `origin/main..HEAD` plus working-tree review fixes in `crates/issuectl-core/src/write.rs`, `crates/issuectl-core/src/mutate/new_issue.rs`, `crates/issuectl/src/cmd/{mod,runtime}.rs`, `crates/issuectl/tests/cli_new.rs`, and the shipped `/issue` templates
**Reviewers:** `gemini-3.1-pro-preview`, `gpt-5.6-sol`, `claude-fable-5`, `deepseek-v4-pro`
**Rounds:** two critique rounds after one bounded context follow-up. DeepSeek completed the independent review, context revision, and first cross-review; its second cross-review repeatedly failed with provider overload/503, so that final position is unavailable.

### Critical Issues (Consensus)

1. **Naive body extraction can duplicate required sections after a Markdown horizontal rule**
   - **What:** `do_new_locked` used `render.split("---\n\n").nth(1)` to find the body. A structured body can contain the same delimiter as a horizontal rule; headings after it were then invisible to required-section detection and could be appended again.
   - **Where:** `crates/issuectl-core/src/mutate/new_issue.rs`, required-section stub generation.
   - **Why it matters:** repositories declaring `body_sections` could receive duplicate H2 sections from valid structured input, recreating the class of defect this change fixes.
   - **Suggested fix:** use the canonical `item_text::split` boundary parser and add a structured-body + horizontal-rule + required-section regression.
   - **Raised by:** all four reviewers; strongest concrete demonstration from OpenAI and DeepSeek.
   - **Resolution:** fixed during review and covered by a core mutation test.

2. **The changed renderer branch lacked an adjacent unit test**
   - **What:** subprocess tests covered the result, but the `structured_body` branch in `render_new_item_from_fm` had no test next to `write.rs`, contrary to the repository's default test-placement rule.
   - **Where:** `crates/issuectl-core/src/write.rs` tests.
   - **Why it matters:** failures would be less local and the core rendering contract was not directly pinned.
   - **Suggested fix:** retain black-box file/stdin dispatch tests and add a renderer unit test covering wrapper suppression and source placement.
   - **Raised by:** all four reviewers, with Anthropic rating it minor rather than blocking.
   - **Resolution:** fixed during review.

3. **The `/issue` template contradicted its new body-file guidance**
   - **What:** the body-file paragraph said no wrapper is added, while a later step still said `issuectl create` always writes `## Description`.
   - **Where:** `crates/issuectl-core/templates/issue-{skill,prompt}.md`, “Flesh out the body”.
   - **Why it matters:** these templates are the agent-facing contract; the contradiction could make agents edit a heading that does not exist.
   - **Suggested fix:** qualify the minimal body as the no-`--body-file` case and mention schema-required stubs.
   - **Raised by:** OpenAI, Anthropic, and DeepSeek; Gemini agreed in the final round.
   - **Resolution:** fixed during review; both formats and dogfooded copies regenerated.

4. **The user-visible fix needed an Unreleased changelog entry**
   - **What:** persisted output for an existing CLI invocation changed without a changelog note.
   - **Where:** `CHANGELOG.md` `[Unreleased] / Fixed`.
   - **Why it matters:** release notes would omit a user-visible bug fix.
   - **Suggested fix:** add a concise fixed entry through the curated changelog workflow.
   - **Raised by:** OpenAI, Anthropic, and DeepSeek.
   - **Resolution:** fixed during review.

### Disputed Issues

1. **Should `intake file --body-file` also become structured-body input?**
   - **For:** Gemini, Anthropic, and DeepSeek observed that intake collapses file input into free text and therefore still adds a wrapper. The standalone intake skill says the report is captured “verbatim”, and intake is the recommended filing path.
   - **Against:** OpenAI noted that intake help explicitly calls the body free text, core sets `structured_body: false` deliberately, and this task explicitly scopes behavior to `create` while preserving other creation paths.
   - **Moderator's take:** the code claim is real, but changing intake here would violate the bounded compatibility scope and requires a product decision about verbatim structured reports versus generated reception structure. Defer rather than silently broaden this patch. No issue is filed from this review because there is no independently observed user occurrence and the current help specifies free-text semantics.

2. **Should the Boolean body-mode flag be replaced with an enum now?**
   - **For:** all reviewers noted that `description: Option<_>` plus `structured_body: bool` permits contradictory states and requires callers to coordinate two fields.
   - **Against:** the affected core crate is explicitly internal, the CLI cannot produce `structured_body=true` without content because empty files are rejected, and an enum refactor would broaden a narrowly scoped fix without demonstrated production impact.
   - **Moderator's take:** valid design debt, but not a warranted refactor in this bug fix. The explicit constructor updates and focused tests make the current state acceptable.

3. **Does suppressing the wrapper for plain-prose body files break compatibility?**
   - **For:** Gemini and Anthropic noted that existing scripts may have used body files as multiline free text and expected a generated heading.
   - **Against:** OpenAI and the final Gemini round emphasized that deterministic source-mode semantics are the documented contract; content sniffing would be ambiguous and the reported issue specifically asks for complete structured Markdown.
   - **Moderator's take:** drop as a defect. This is the intended, now-explicit behavior and is covered in the changelog.

### Minor Findings

- `run_with_stdin` writes a small fixture before draining output; safe for this test, but unsuitable as a future large-input helper.
- “Complete body” could imply byte preservation even though trailing whitespace is deliberately normalized and required-section stubs may be appended. The revised template now says “structured Markdown content” and documents schema augmentation.
- Existing duplicate headings are not migrated. Automatic repair cannot reliably distinguish generated duplicates from authored structure and is outside this forward-write fix.

### Dropped Concerns

- Public semver break: repository policy explicitly states that `issuectl-core` public items are not the binary's semver contract.
- Empty body files: `read_body_file_arg` already rejects empty and whitespace-only input.
- Claude/Codex or dogfooded-template drift: regeneration and dogfood installation were run; focused tests will verify byte identity.
- Trailing whitespace handling: pre-existing, deliberate, and documented by the body-file reader.

### What's Solid

- File/source identity is captured before consuming `body_file`, then flows through the locked schema-validated mutation path.
- Inline `--description`, intake, API, import, recurrence, and other creation semantics remain unchanged.
- File and stdin behavior, source placement, duplicate-heading suppression, and inline compatibility are covered end to end.
- Empty input, invalid UTF-8, input caps, and read failures remain handled before mutation.

### Moderator's Assessment

OpenAI made the strongest arguments overall: it separated the explicit create-only scope from adjacent intake behavior, retracted concerns after receiving context, and supplied the canonical-splitter failure mode and focused fix. Anthropic was strongest on agent-documentation contradictions and the custom-schema reachability nuance. Gemini correctly converged on deferring intake after initially overstating it. DeepSeek independently corroborated the key findings but its provider failed during the second cross-review round.

The single most important change was replacing the naive frontmatter split: without it, a valid structured body could still acquire duplicate schema-required headings after a horizontal rule. The warranted findings were applied; intake semantics and the enum refactor remain deliberate deferrals rather than hidden defects.
