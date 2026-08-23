## Review: Shipped `/issue` create-path and update-echo guidance

**Reviewed:** `e332fde..146a042`, chiefly `crates/issuectl-core/templates/issue-{skill,prompt}.md` and the related black-box contract tests  
**Reviewers:** `gemini-3.1-pro-preview`, `gpt-5.6-sol`, `claude-opus-5`, `deepseek-v4-pro`  
**Rounds:** 2 cross-review rounds after independent review

### Critical Issues (Consensus)

No runtime correctness defect remained after verification. The reviewers agreed that the corrected `.data.path`, conditional scheduling echoes, and command-specific `blocked_by` representations match the implementation.

### Confirmed Findings

1. **Claude and Codex contract bodies were not pinned to each other**
   - **What:** Existing tests bound each template to its dogfooded copy, but did not prove that the Claude body and frontmatter-stripped Codex body remained identical.
   - **Where:** `crates/issuectl-core/src/skill.rs`, `templates_differ_between_agents`.
   - **Impact:** A future edit and reinstall could leave one shipped agent format stale while all copy-vs-template tests pass.
   - **Resolution:** Added body-equality coverage using the established frontmatter-stripping pattern.
   - **Raised by:** all four reviewers.

2. **Prose assertions were sensitive to Markdown wrapping**
   - **What:** One assertion embedded a literal newline in `canonical bare-slug\nblocked_by list`; other checks depended on fenced-JSON spacing.
   - **Where:** `crates/issuectl/tests/cli_json_echo.rs` and `cli_new.rs`.
   - **Impact:** A semantics-preserving reflow could fail CI, while Codex semantic coverage remained indirect.
   - **Resolution:** Normalize whitespace and run semantic assertions over both shipped templates.
   - **Raised by:** all four reviewers.

3. **The absent-versus-null scheduling echo needed a black-box clear case**
   - **What:** Existing focused tests covered report-builder clear behavior, but the new end-to-end contract test only exercised `--lane` set.
   - **Where:** `crates/issuectl/tests/cli_json_echo.rs`.
   - **Impact:** The agent-facing distinction between an omitted unrequested key and a requested key echoed as `null` was not pinned at the process boundary.
   - **Resolution:** Added `update --no-lane --json` coverage asserting present `null` and absent unrequested fields.
   - **Raised by:** Gemini, OpenAI, Anthropic, DeepSeek.

4. **The DAG “each issue row” claim exceeded its explicit black-box assertions**
   - **What:** The existing DAG test asserted a populated bare-slug blocker list but not empty lists on scheduled and unscheduled rows.
   - **Where:** `crates/issuectl/tests/cli_dag.rs` and the new skill paragraph.
   - **Impact:** The prose claimed a stable array on every row without directly testing empty-row shape.
   - **Resolution:** Added empty-array assertions for both scheduled and unscheduled rows.
   - **Raised by:** OpenAI, Anthropic, DeepSeek.

5. **Filesystem guidance should emphasize the returned contract, not path reconstruction**
   - **What:** The fixed prose still stated the current flat layout as a durable bullet.
   - **Where:** Create section in `crates/issuectl-core/templates/issue-{skill,prompt}.md`.
   - **Impact:** A future layout migration could recreate the same class of stale guidance.
   - **Resolution:** Direct agents to `.data.path` and explicitly say not to reconstruct paths from slugs; label the shown object as the plain-create `.data` shape.
   - **Raised by:** OpenAI and Anthropic; accepted as a low-cost durability improvement.

### Disputed or Dropped Concerns

1. **Add `blocked_by` to update echoes** — dropped as out of scope and contrary to the accepted contract. `blocked_by` intentionally remains an untyped canonical read projection; this task documents current behavior.
2. **Normalize `show` and `dag` sigils** — dropped. `show` uses frontmatter-facing `@slug` references while the scheduling DAG uses bare graph-node slugs; both shapes are tested and intentionally command-specific.
3. **Duplicate every scheduling echo case as a black-box test** — narrowed. Existing focused tests cover lane-seq, collision, no-op, and clear report-builder semantics; one set, one absent, and one clear process-level case adequately bind integration.
4. **Document create's all-three scheduling echo behavior here** — dropped as a pre-existing omission not introduced by this fix.
5. **Warn about concurrency between update and show** — dropped as unnecessary detail for this correction; `show` is clearly described as a current-state read, not an update echo.

### What's Solid

- The corrected create result uses `.data.path` and `.data.dir`, matching the exact black-box payload.
- `update` conditionally echoes only the requested `lane`, `lane_seq`, and `collision` scheduling fields; `blocked_by` is absent.
- `show` returns canonical `@`-prefixed blocker references and `dag` returns bare scheduling slugs.
- Claude/Codex templates and repo dogfooded copies are regenerated from the same body and byte-checked.

### Moderator's Assessment

Anthropic made the strongest final argument by locating the exact existing frontmatter-stripping test pattern and retracting earlier scope creep. OpenAI provided the broadest test matrix, while DeepSeek most consistently focused on the DAG claim. The single most important fix was pinning Claude/Codex body equality; it closes the only route by which half the shipped contract could silently remain stale.
