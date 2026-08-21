## Review: apply-patch-from-stdin

**Reviewed:** committed implementation from `97ae6ea..9e75f69`, centered on `crates/issuectl-core/src/patch_input.rs`, both CLI dispatch surfaces, help, templates, and black-box parity tests
**Reviewers:** Gemini 3.1 Pro Preview, GPT-5.6, Claude Opus 5, DeepSeek V4 Pro
**Rounds:** 2

### Critical Issues (Consensus)

1. **Path-shape inference misclassifies both paths and inline payloads**
   - **What:** `looks_path_shaped` treated a missing extensionless filename as an unsupported form, while inline JSON containing a dot or slash fell through as a missing file. Its `exists()` preflight also discarded metadata errors and introduced a needless read race.
   - **Where:** `crates/issuectl-core/src/patch_input.rs`, input source classification.
   - **Why it matters:** The diagnostic contradicted the documented “path or stdin” grammar and recreated the exact misleading missing-file message for common JSON payloads.
   - **Suggested fix:** Detect only lexically unambiguous inline payloads (`{` or `[` after leading whitespace); treat every other non-`-` token as a path and let `read_to_string` provide the authoritative I/O error.
   - **Raised by:** all four reviewers.

### Partial-Consensus Findings

1. **Shared-core errors had avoidable CLI coupling and duplicate context**
   - The moved `dry_run` error recommended canonical `update --patch-file` even when reached through `apply`, and the outer and inner parse contexts both said “patch fields.” Reviewers differed on whether this was architectural debt or a blocking boundary violation, but agreed the message should be verb-neutral and the context non-duplicative.

2. **Regression coverage was too narrow around the new classifier**
   - Tests covered dotless/slashless inline JSON, YAML stdin, and a missing `.yaml` path, but not dotted/slashed inline payloads, extensionless missing paths, JSON stdin, empty stdin, or missing-path parity. All reviewers recommended focused additions; they differed only on how many edge cases were necessary.

3. **The raw `json_output` boolean obscured its semantic purpose**
   - Reviewers recommended representing the real policy—whether `expected_version` is required—rather than passing a presentation-mode boolean into the parser. Most treated this as maintainability improvement rather than a correctness failure.

### Disputed Issues

1. **TTY refusal and patch-size cap**
   - **For:** One first-round review argued that bare `-` on a terminal can block and unbounded stdin can consume excessive memory.
   - **Against:** Other reviewers noted that blocking on explicitly requested stdin is conventional Unix behavior and file patch input was already fully buffered and unbounded.
   - **Moderator's take:** Out of scope and not a regression. No change or follow-up issue is justified by this review.

2. **Stable patch-specific JSON error codes**
   - **For:** One reviewer argued agent callers should not substring-match `command-failed` prose.
   - **Against:** The filed issue explicitly asks for accepted-form messages, and changing the repository-wide error taxonomy is broader than this patch.
   - **Moderator's take:** A plausible canon-wide concern, but not localizable to this issue and not a merge blocker.

3. **Test-only parser forwarding shim**
   - **For:** Some reviewers considered the `#[cfg(test)]` forwarding function unnecessary indirection.
   - **Against:** DeepSeek considered a test-only alias to the single core parser harmless.
   - **Moderator's take:** Minor cleanup only; retaining it does not create runtime drift.

### Dropped Concerns

- Claims that `patch_input` was not registered in `lib.rs`, skill templates were not updated, and dogfooded copies were missing were retracted in round two after the omitted files were supplied. The full green gate had already verified module registration and template parity.
- A claimed Windows failure for `./-` was explicitly retracted after checking `Path::components` behavior.
- Concerns that `update --patch-file` might bypass the shared loader were retracted because the black-box stdin parity test proves both dispatch paths reach it.

### What's Solid

- Both canonical `update --patch-file` and compatibility `apply` call one loader/parser and one mutation path.
- The two-root black-box test compares dry-run envelopes, write envelopes, and resulting issue bytes.
- Exact `-` matching leaves `./-` available for a literal dash filename.
- The skill template and dogfooded Claude/Codex copies document piping, JSON payload support through path/stdin, and the inline-argv decision.
- Injecting `Read` makes actual stdin read failures testable without process tricks.

### Moderator's Assessment

GPT-5.6 made the strongest overall argument: it identified the classifier defect, explained the metadata-error/TOCTOU consequence, and separated blocking correctness issues from lower-priority API cleanup. Claude Opus added the clearest explanation of how the `dry_run` remediation regressed during the parser move, although several first-round claims were based on incomplete context and were later retracted. The single most important correction is to replace path-shape inference with deterministic lexical inline-payload detection and authoritative file reads.
