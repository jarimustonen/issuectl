## Review: doctor workflow metadata registration

**Reviewed:** commit `71eecca` (`crates/issuectl-core/src/schema.rs`, doctor checks/tests, parser/canonical/mutation context)
**Reviewers:** `gemini-3.1-pro-preview`, `gpt-5.6-sol`, `claude-fable-5`, `deepseek-v4-pro`
**Rounds:** 2 cross-review rounds, plus one bounded context follow-up per reviewer

### Critical Issues (Consensus)

No critical production defect survived source-grounded cross-review. All reviewers ultimately agreed that default-schema registration is the repository-consistent mechanism for workflow-owned string fields: doctor derives known names from the effective schema, standard writers emit strings through `--field key=value`, project schemas layer over built-ins, and the generated AGENTS policy intentionally reflects the effective schema. The hardcoded doctor known-key list is reserved chiefly for shapes schema v1 cannot express, such as integer `lane_seq` and mapping-list `commits`.

### Confirmed Findings

1. **Workflow field documentation described enum-free strings as fully “open-valued.”**
   - **What:** `FieldSpec` scalar validation accepts strings, not arbitrary YAML values. The workflow is compatible because its documented writer path emits strings, but the initial wording and test name blurred schema typing with typed `Issue` slots.
   - **Where:** `crates/issuectl-core/src/schema.rs`, workflow metadata declarations and schema test.
   - **Why it matters:** A maintainer could incorrectly infer that numeric or structured values are accepted, or add enums that make independently evolving review vocabularies fail writes.
   - **Suggested fix:** State that fields are optional strings retained in `Issue.extra`, deliberately enum-free; explain `review_source` versus repo-configurable `provenance`.
   - **Raised by:** all four reviewers.

2. **The schema-installation assertion used layered loading and did not inspect installed content.**
   - **What:** Calling `load()` after `ensure_default_written()` overlays built-in defaults, so field-presence assertions could pass even if the file lacked the declarations.
   - **Where:** `crates/issuectl-core/src/schema.rs`, `ensure_default_written_creates_file`.
   - **Why it matters:** The test claimed direct bootstrap coverage without independently checking the emitted schema artifact.
   - **Suggested fix:** Read and deserialize the installed YAML bytes directly and assert all eight fields.
   - **Raised by:** OpenAI, Gemini, Anthropic, DeepSeek.

3. **The explicit `Issue.extra` compatibility guarantee lacked a field-specific parser regression.**
   - **What:** Code inspection confirms schema declarations do not affect parser flattening or canonical projection, but no test pinned all eight names in `Issue.extra`.
   - **Where:** `crates/issuectl-core/src/parser.rs` and the compatibility comment in `schema.rs`.
   - **Why it matters:** A later typed-field promotion could accidentally alter canonical token projection unless promotion parity is handled deliberately.
   - **Suggested fix:** Parse an issue containing all eight fields and assert every key remains in `Issue.extra`.
   - **Raised by:** OpenAI, Gemini, Anthropic; DeepSeek considered it redundant but agreed current behavior is safe.

### Disputed Issues

1. **Default schema versus doctor-only registration**
   - **For doctor-only:** Gemini initially argued schema declaration creates string constraints and adds workflow details to user schema/AGENTS output.
   - **For schema registration:** OpenAI, Anthropic, and DeepSeek cited existing intake-field precedent and the hardcoded list’s schema-inexpressible shapes. The authoritative workflow contract confirmed string-valued writes.
   - **Moderator's take:** Schema registration is correct here. Gemini withdrew the objection in the final round.

2. **A shared Rust vocabulary constant**
   - **For:** DeepSeek argued it would reduce repeated lists.
   - **Against:** Anthropic and OpenAI noted no production code consumes such a list; a test-only authority derived from the implementation would make tests more self-referential and would not solve cross-repository contract drift.
   - **Moderator's take:** Do not add an abstraction solely for test deduplication. Independent explicit lists are acceptable for this bounded eight-field contract.

3. **End-to-end missing-schema / doctor --fix lifecycle test**
   - **For:** Early reviews requested a pre-scan, bootstrap, and post-scan test.
   - **Against:** Final review observed the path composes directly from existing default fallback, bootstrap tests, raw emitted schema, and effective-schema doctor tests.
   - **Moderator's take:** Useful consolidation but not warranted additional scope once raw installation and effective old-schema paths are covered.

### Dropped Concerns

- Workflow vocabulary incompleteness: the authoritative run filing contract confirms exactly three core and five optional metadata fields; model IDs use labels and intake identity uses existing provenance/source-ref fields.
- Default enums for classification/outcome/severity/confidence: rejected because independently evolving values and casing must not turn optional enrichment into hard schema failures.
- AGENTS.md “pollution”: generated managed content intentionally documents the effective schema; drift after a built-in schema change is expected regeneration behavior, not a static shipped-template mismatch. This repository has no dogfooded `.issuectl/AGENTS.md` copy.
- `blocked_by` regression: no workflow field was promoted to typed `Frontmatter`; `blocked_by` remains in `extra` exactly as required.
- Existing numeric `estimate` warning: outside the reviewed issue and not established as part of this contract; no follow-up filed.

### What's Solid

- The eight fields match the authoritative filing contract exactly.
- All are optional and enum-free, matching evolving reviewer/orchestrator values.
- Existing project schemas inherit the defaults without rewriting their committed files.
- Doctor performs exact-name recognition; the `whimsy` control proves arbitrary undeclared extensions still warn.
- Parser and canonical code keep the metadata in `Issue.extra`; `blocked_by` is untouched.

### Moderator's Assessment

OpenAI made the strongest overall argument by separating verified runtime correctness from test-proof weaknesses and by correcting the initial overstatement about schema registration. Anthropic best explained the repository convention and later dropped its own unnecessary constant proposal. The single most important correction was making the schema contract precise: these are enum-free strings written through `--field`, not arbitrary YAML values.
