## Review: Generated issue skill contract alignment

**Reviewed:** `main...HEAD`, especially `crates/issuectl-core/templates/issue-skill.md`, `issue-intake-skill.md`, generated Claude/pi/Codex copies, and focused assertions in `crates/issuectl-core/src/skill.rs`
**Reviewers:** `gemini-3.1-pro-preview`, `gpt-5.6-sol`, `claude-fable-5`, `deepseek-v4-pro`
**Rounds:** 2 cross-review rounds after independent review
**External contract checked:** installed Taskfleet 0.7.1 bundled `worktree-bug-analysis`, `worktree-spinoff`, and `stint-start` skills

### Critical Issues (Consensus)

No material issue remained after the review fixes. The panel initially found the following defects, all corrected before this report was finalized:

1. **Alternative-heading analysis could respawn across invocations**
   - **What:** The first revision checked `## Suspected Root Cause` only after spawning a worker, while issuectl projects only exact `## Triage analysis` sections.
   - **Where:** `crates/issuectl-core/templates/issue-intake-skill.md`, Steps 2–3.
   - **Impact:** A Taskfleet-compliant analysis using its permitted alternative heading could be repeated on every later intake run.
   - **Resolution:** Existing exact or alternative analysis is now checked before classification and spawning. The alternative is handled honestly as provenance-free, untrusted issue-analysis data.
   - **Raised by:** all four reviewers.

2. **Settlement failure states were under-specified**
   - **What:** The first revision used `run wait` without a workflow-specific timeout and did not fully cover partial spawn failure, run-to-slug correlation, malformed/null reports, aggregate wait errors, or unreadable `run show` output.
   - **Where:** `crates/issuectl-core/templates/issue-intake-skill.md`, Step 3.
   - **Impact:** Intake could wait too long, lose correlation or diagnostics, or infer success from incomplete state.
   - **Resolution:** The contract now retains `(slug, run id)` pairs, checks supervisor health, uses a two-hour bounded wait, branches on timeout and other errors, falls back to individual inspection, and never infers success from malformed/null reports.
   - **Raised by:** all four reviewers, with detailed failure-state coverage from OpenAI, Anthropic, and DeepSeek.

3. **Landing semantics were too broad**
   - **What:** The first revision treated every `landed: false` as epistemically uncertain.
   - **Where:** `crates/issuectl-core/templates/issue-intake-skill.md`, Step 3.
   - **Impact:** It contradicted Taskfleet's distinction between `landed_method: unverified` and a git-verified non-landing.
   - **Resolution:** Only `unverified` is treated as unknown; git-verified false is a confirmed non-landing. Git history, ancestry, and worker branches remain prohibited as completion checks.
   - **Raised by:** Anthropic and DeepSeek; accepted by the full panel.

4. **Close-layout wording still implied legacy paths**
   - **What:** The initial correction still described `issues/closed/` as current and left older close prose saying status changes move directories.
   - **Where:** `crates/issuectl-core/templates/issue-skill.md`, Close, Update, and Notes sections.
   - **Impact:** Agents could reconstruct a legacy path or misread `moved_to_closed` as a filesystem move.
   - **Resolution:** Unarchived active and closed items are documented as flat; archive storage is bucketed and separate. The real `moved_to_closed: true` field is explicitly documented as a legacy-named lifecycle-transition indicator.
   - **Raised by:** Anthropic and DeepSeek; independently verified against `mutate/update.rs`, `mutate/shared.rs`, and `cmd/write.rs`.

### Disputed or Incorrect Findings

1. **Change issuectl to recognize both analysis headings**
   - **For:** Reviewers noted that central parsing would remove caller-side compatibility prose.
   - **Against:** This task fixes the shipped caller contract and must not invent a Taskfleet guarantee. Issuectl's exact-heading projection is an intentional current contract; changing it would broaden CLI behavior beyond this focused fix.
   - **Moderator:** Keep the localized pre-spawn compatibility check and track Taskfleet heading convergence separately.

2. **Issuectl intake JSON is unwrapped**
   - **Finding:** One reviewer inferred handler-local printing meant `.data.body` and `.data.analysis` did not exist.
   - **Assessment:** Incorrect. Worktree-local issuectl 0.18.3 empirically emits `{schema_version,data,warnings}` for `intake queue --json`; the shared output layer wraps command data.

3. **Generated copies were absent**
   - **Finding:** Scoped diff attachments led one reviewer to infer the Claude/pi/Codex copies and paired prompt templates had not been regenerated.
   - **Assessment:** Incorrect. The full diff includes every affected generated copy, and `dogfooded_copies_match_templates` passes.

4. **Broken-pipe work was reverted by this branch**
   - **Finding:** A two-dot `git diff main` attached during the last cross-review included changes that landed on `main` after this worktree branched, so all reviewers flagged them as an unrelated revert.
   - **Assessment:** Incorrect as a branch-authored change. `git diff main...HEAD` contains no broken-pipe files; the apparent reversal is source-branch drift and will be removed by rebasing onto current `main` before merge.

### Minor Findings

- Re-reading every settled issue is one harmless extra read for git-verified non-landings, but keeps the content-verification path uniform for `unverified`; no change recommended.
- Hard-coded Taskfleet 0.7.1 compatibility prose creates maintenance pressure. This is deliberate because the finding was verified against that exact released contract; future Taskfleet changes require a fresh contract check.

### What's Solid

- The contract now uses Taskfleet's canonical `run wait` → `run show` → `landed`/structured-report flow and retains failure diagnostics.
- Bug-only routing is explicit across `/issue`, `/issue-intake`, and the repo-local `/triage-bugs` alias.
- The stale `intake-return` claim is removed consistently with the current `stint-start` DAG/disposition contract.
- Flat and archived paths match issuectl 0.18.3 behavior.
- Section-aware regression assertions pin the pre-spawn heading check and removal of stale completion/path phrases without snapshotting the entire prose.

### Moderator's assessment

OpenAI produced the strongest failure-state analysis; DeepSeek most clearly caught the ordering and stale cross-reference problem; Anthropic was strongest on exact Taskfleet landing semantics; Gemini confirmed the final state machine coherently. The single most important issue was the cross-invocation alternative-heading respawn loop, now fixed before spawn. No unresolved material defect remains in the reviewed skill-contract changes.
