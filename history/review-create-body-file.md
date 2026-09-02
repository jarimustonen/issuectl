## Review: create `--body-file` structured Markdown fix

**Reviewed:** final `main..52f09a1` production, tests, changelog, CLI help, and shipped/dogfooded agent-contract diff
**Reviewers:** `gemini-3.1-pro-preview`, `gpt-5.6-sol`, `claude-fable-5`, `deepseek-v4-pro`
**Rounds:** one complete recovered independent review and two complete four-model cross-review rounds from the preserved implementation review, followed by a fresh supplementary review of the adopted tree. The preserved raw evidence is complete and is the authoritative multi-model review; the fresh supplement found and drove three final localized improvements. See [`review-create-body-file-evidence.md`](review-create-body-file-evidence.md).

### Critical Issues (Consensus)

No production blocker survived source verification and cross-review.

### Warranted In-Patch Improvements

1. **Structured-body schema completion lacked a positive regression**
   - **What:** the original horizontal-rule test proved that existing required sections were not duplicated, but did not prove that a genuinely missing section is appended with canonical spacing.
   - **Where:** `crates/issuectl-core/src/mutate/new_issue.rs`, `structured_body_with_horizontal_rule_does_not_duplicate_required_sections`.
   - **Resolution:** require an omitted `Quick Test` section and assert one correctly separated appended stub while preserving the horizontal rule.
   - **Raised by:** all four preserved reviewers in their final positions.

2. **Agent guidance overstated default epic rendering**
   - **What:** the templates categorically said the CLI does not write recommended epic sections even though repository schemas can append them.
   - **Where:** both shipped `/issue` templates and dogfooded copies.
   - **Resolution:** qualify the statement as default-renderer behavior.
   - **Raised by:** OpenAI in the preserved review; accepted after direct source verification.

3. **Touched epic guidance recommended a reserved legacy section**
   - **What:** the edited guidance still told agents to create `## Notes`, while CLI and doctor treat it as a legacy alias and direct users to `## Comments`.
   - **Where:** both shipped `/issue` templates and dogfooded copies.
   - **Resolution:** use canonical `## Comments` in all four rendered/template copies.
   - **Raised by:** DeepSeek in the fresh review; independently confirmed against `body_sections::LEGACY_SECTION_ALIASES` and CLI help.

4. **The full CLI-to-schema composition lacked a black-box regression**
   - **What:** separate tests covered runtime mode propagation and core schema completion, but no built-binary test composed `--body-file`, a horizontal rule, existing required H2s, and an appended missing stub.
   - **Where:** `crates/issuectl/tests/cli_new.rs`.
   - **Resolution:** add `body_file_schema_appends_only_missing_sections`, which exercises the complete command path and exact ordering.
   - **Raised by:** OpenAI, Anthropic, and DeepSeek in the fresh review; Gemini accepted it in cross-review.

5. **CLI help omitted schema-appended stubs**
   - **What:** templates documented schema completion but `create --help` did not.
   - **Where:** `crates/issuectl/src/cmd/mod.rs`.
   - **Resolution:** state that repository schemas may append stubs for missing H2 sections.
   - **Raised by:** OpenAI in the fresh review; accepted by the other available cross-reviewers.

### Confirmed Follow-Up Defect

1. **Issuectl JSON export-to-import duplicates structured body headings**
   - **What:** JSON export emits the complete structured `Issue.body`; import accepts `body` as an alias for free-text `description`, then adds a generated `## Description` wrapper. Re-import can therefore duplicate the heading and nest the exported H1.
   - **Where:** `crates/issuectl-core/src/transfer.rs`, `ImportRecord.description` and `ImportRecord::into_new_args`.
   - **Scope:** real, independently reproduced, pre-existing, and intentionally outside this create-only patch. It is preserved unlaned and untriaged as `@dreadfully-robust-pencil`.
   - **Raised by:** Anthropic and DeepSeek independently; Gemini and OpenAI later confirmed the defect and its separate scope.

### Disputed and Dropped Issues

- **Fixed versus Changed changelog category:** keep the entry under Fixed. The prior implementation contradicted the released help contract that file Markdown is written below the H1; the entry precisely describes the corrected duplicate-heading behavior.
- **Warnings or content sniffing for heading-less body files:** reject. Markdown need not contain H2 headings, the source-based mode is explicit, and sniffing would create a second implicit grammar.
- **Replace `description + structured_body` with an enum now:** valid maintainability debt, but all constructors are explicit, empty CLI files are rejected, and a cross-layer enum refactor is not justified by a current failure.
- **Public Rust API break:** incorrect under this repository's explicit contract: `issuectl-core` is published but internal, its modules are `#[doc(hidden)]`, and only the binary CLI is the semver contract.
- **Transfer/new API/intake/recurrence should switch modes:** incorrect for this patch. Their free-text semantics are an explicit compatibility requirement; the own-export alias defect needs separate design.
- **Fence and horizontal-rule parsing is unsafe:** incorrect. `item_text::split` selects canonical generated frontmatter and `all_h2_sections` ignores fenced pseudo-headings.
- **Empty files can produce title-only output:** incorrect; `read_body_file_arg` rejects empty and whitespace-only input.
- **Case-insensitive section matching, H1 rejection, unclosed-fence rejection, and stdin-helper hardening:** optional hardening or test debt without a demonstrated production regression here.

### What's Solid

- The bug independently reproduced on pre-fix `main`: a file beginning with `## Description` received another empty generated Description heading.
- Runtime records the input source before consuming it, so file/stdin bodies become structured while inline descriptions retain the wrapper.
- Canonical frontmatter splitting prevents a body horizontal rule from truncating required-section detection.
- File, stdin, source preamble, inline compatibility, schema completion, and full CLI/schema composition are tested.
- All non-create constructors explicitly preserve prior free-text behavior.
- Claude/Codex template bodies and rendered dogfooded copies are enforced by repository tests.

### Moderator's Assessment

The preserved complete review remains valid for the adopted core patch. The fresh supplement was useful: DeepSeek found the `## Notes` contradiction, while OpenAI identified the strongest remaining coverage and help gaps. Those localized findings were source-confirmed and fixed. A fresh DeepSeek cross-review call later hit account exhaustion twice; this did not invalidate the complete preserved DeepSeek independent recovery and both complete preserved cross-review rounds. All failed and recovered attempts are retained in the evidence appendix rather than hidden.

The final tree has no confirmed in-scope blocker. The only independently reproduced follow-up is the own-JSON export/import corruption already preserved as an unlaned intake item.
