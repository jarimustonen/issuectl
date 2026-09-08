## Review: Broken pipe handling

**Reviewed:** `crates/issuectl/src/cmd/mod.rs`, `crates/issuectl/tests/cli_broken_pipe.rs`, and the branch diff against `main`
**Reviewers:** gemini-3.1-pro-preview, gpt-5.6-sol, claude-fable-5, deepseek-v4-pro
**Rounds:** 2 cross-review rounds after the independent review

### Critical Issues (Consensus)

1. **The first regression test was racy**
   - **What:** Spawning with piped stdout and dropping the reader afterward allowed the child to finish its small write before the drop, so the old implementation could pass.
   - **Where:** `crates/issuectl/tests/cli_broken_pipe.rs`
   - **Why it matters:** The test did not reliably exercise `BrokenPipe`.
   - **Resolution:** Replaced it with a Unix socket pair whose peer is closed before spawn. The test now invokes the reported `--json dag` path.
   - **Raised by:** all four reviewers.

2. **Immediate `process::exit(0)` could abort work after output**
   - **What:** The initial implementation exited from the low-level output seam and skipped destructors or any command work following an output call.
   - **Where:** `crates/issuectl/src/cmd/mod.rs::emit_stdout`
   - **Why it matters:** A mutating text command could stop after partial progress while reporting success.
   - **Resolution:** `BrokenPipe` now sets `STDOUT_BROKEN`; subsequent rendering is suppressed while command execution completes. A `scan-todos --file-intake` black-box test proves a mutation following failed output still lands.
   - **Raised by:** all four reviewers.

### Partial Consensus

1. **Test the non-BrokenPipe branch**
   - The reviewers requested evidence that unrelated stdout failures remain fatal. A Linux-only `/dev/full` black-box test now checks the nonzero status and diagnostic. This preserves, rather than redesigns, the existing non-EPIPE panic behavior.

2. **Exercise text output as well as JSON**
   - A text-mode scan-todos case now covers the second branch and the continue-after-EPIPE design. The required JSON regression remains the primary test.

### Disputed Issues

1. **Exit 0 versus SIGPIPE-style 141**
   - **For 141:** Gemini argued that conventional `pipefail` behavior should expose truncated output.
   - **Against:** The task explicitly requires ordinary, successful downstream termination. The other reviewers agreed 141 conflicts with that contract.
   - **Moderator's take:** Exit 0 is binding here; the objection is rejected.

2. **Continue read-only work after EPIPE**
   - **Concern:** Gemini noted that suppression lets a large read-only command finish computation after its consumer exits.
   - **Against:** Immediate exit is unsafe at a shared seam also used around mutations, and distinguishing command classes would broaden this fix substantially.
   - **Moderator's take:** This is a known trade-off, not a blocker for the observed panic.

### Minor Findings / Out of Scope

- Direct stdout writers in `issuectl-core`, clap-owned help/version output, and stderr EPIPE do not pass through `emit_stdout`. They predate this patch and were not demonstrated by the reported JSON DAG failure.
- Per-emission flushes and panic-based handling for non-EPIPE errors also predate this patch.
- Windows does not receive the Unix closed-peer black-box test; current release targets are Unix.
- An attempted completions reroute was removed because it would have changed raw `--json completions` output semantics.

### What's Solid

- The final implementation distinguishes only `ErrorKind::BrokenPipe`; every other error follows the existing fatal path.
- Both write-time and flush-time failures are observed.
- The black-box setup deterministically creates a closed stdout peer and checks status plus byte-empty stderr.
- The final production change is localized to the central output seam and does not alter JSON envelopes or skill-facing CLI behavior.

### Moderator's Assessment

OpenAI made the strongest overall review: it identified both the test race and the lifecycle risk, then converged on the narrow final scope. Claude gave the best concrete mutation-after-output test recommendation. The most important correction was replacing `process::exit(0)` with suppression while command work completes. No confirmed residual finding warrants expanding this focused patch or filing a follow-up from this run.
