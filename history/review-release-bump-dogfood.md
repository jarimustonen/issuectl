## Review: release bump dogfood regeneration

**Reviewed:** commit `ad53b0d` and the follow-up hardening in `scripts/release-bump-hook.sh`, `OSS-RELEASE.md`, and `crates/issuectl/tests/cli_skill.rs`
**Reviewers:** `gemini-3.1-pro-preview`, `gpt-5.6-sol`, `claude-fable-5`, `deepseek-v4-pro`
**Rounds:** independent review plus two cross-review rounds

### Critical issues (consensus, fixed during review)

1. **The test-only binary override was a production stale-binary bypass.**
   - **What:** Shipshape inherits the release environment, so `ISSUECTL_RELEASE_HOOK_BIN` could have silently generated tracked copies from an old binary.
   - **Where:** original `scripts/release-bump-hook.sh` build branch.
   - **Resolution:** removed the override from production. The test now intercepts `cargo` through `PATH`, while the hook always takes the production command path.
   - **Raised by:** OpenAI, Anthropic, DeepSeek; accepted by Gemini in cross-review.

2. **The hard-coded Cargo artifact path was invalid with a configured build target.**
   - **What:** `$CARGO_TARGET_DIR/debug/issuectl` is wrong when Cargo places host artifacts beneath a target-triple directory.
   - **Resolution:** the hook now uses `cargo run --bin issuectl`, which owns executable discovery.
   - **Raised by:** all four reviewers.

3. **The original test skipped the release-critical Cargo invocation.**
   - **What:** injecting the binary meant malformed Cargo arguments, wrong CWD, and environment handling could pass CI.
   - **Resolution:** a fake `cargo` records and asserts the production CWD, complete argv, isolated HOME/target, and delegates post-`--` arguments to the test binary. An unmocked manual run additionally exercised real Cargo.
   - **Raised by:** all four reviewers.

4. **Nothing failed closed when generated copies pinned the wrong version.**
   - **What:** Shipshape validates the manifest version after the hook, not generated file content.
   - **Resolution:** the hook checks the bumped workspace-version marker in all nine tracked outputs.
   - **Raised by:** OpenAI, Anthropic, DeepSeek.

### Additional confirmed findings fixed during review

5. **Cargo depended unnecessarily on Shipshape's current directory.** The hook now changes to its own repository root before invoking Cargo.
6. **The broad installer could create or alter `issues/AGENTS.md`.** The hook snapshots an existing scaffold and fails if it changes; it also fails if an absent scaffold is created.
7. **Operator-home checks were incomplete.** The test now plants and verifies all nine managed paths plus the legacy pi manifest in the operator HOME.
8. **The signal trap did not explicitly terminate.** Separate HUP/INT/TERM traps now preserve terminating behavior while EXIT performs cleanup.
9. **The contract check was textual only.** The test now parses YAML frontmatter and asserts `release.bump_hook` structurally.

### Remaining findings

1. **Unscoped workspace-version extraction (low/medium).**
   - `scripts/release-bump-hook.sh:59` takes the first exact `version = "…"` line anywhere in `Cargo.toml`. Valid formatting changes or an earlier table-level version could make a correct cut fail closed or check the wrong value.
   - Reviewers recommend section-scoped parsing or authoritative Cargo metadata.
   - Raised by OpenAI, Anthropic, DeepSeek.

2. **CARGO_HOME/RUSTUP_HOME preservation is not asserted (low).**
   - The fake-Cargo test logs HOME and CARGO_TARGET_DIR, but not the two toolchain homes needed while HOME is isolated.
   - Raised by OpenAI.

3. **The real Cargo execution is manual evidence rather than durable CI coverage (low/medium).**
   - The fake boundary test cannot prove rustup resolution and `env!("CARGO_PKG_VERSION")` embedding in a fresh compilation, although all identified failures now abort before publish and the unmocked hook passed during this work.
   - Raised by OpenAI and Anthropic; DeepSeek considered the production boundary adequately covered when combined with the manual run.

4. **The nine-path inventory is duplicated (low).**
   - The installer catalog, shell verification loop, and Rust test constant must move together when skills or targets change. Current coverage is complete, but future additions could drift.
   - Raised by OpenAI, Anthropic, DeepSeek.

5. **The missing-scaffold failure branch lacks a direct regression test (low).**
   - The successful path proves an existing scaffold is unchanged, but does not run the hook with no scaffold and assert refusal.
   - Raised by DeepSeek.

6. **Byte equality is stricter than the older development-time invariant (disputed).**
   - DeepSeek argued the integration test conflicts with the core test's tolerance for a lagging pinned version during ordinary development.
   - Anthropic treated immediate byte equality as appropriate for this release-bump test. Moderator agrees: this work intentionally tightens the release-commit invariant, and current development starts with synchronized copies.

### Dropped concerns

- A dedicated dogfood generator or new `--no-scaffold` CLI was judged disproportionate after the scaffold guard and all-nine version check made the existing installer fail closed.
- XDG mutation was unsupported by current code; installation targets are explicitly rooted under `--target` and the CLI passes no global pi root.
- Cold compilation is intentional: a disposable target prevents reuse of pre-bump binaries.
- Full git output allowlisting, symlink hardening, strict historical-POSIX portability, and retaining the human installer summary were considered future hardening without demonstrated current impact.

### What's solid

The hook is contract-declared and runs after Shipshape edits the manifest and lockfile but before its bump commit/build/publish barriers. All nine current Claude, pi, and Codex targets are explicit, generation uses an isolated HOME and target, failures stop before irreversible publish, and `issues/AGENTS.md` is protected.

### Moderator's assessment

OpenAI supplied the strongest initial review by tying the ambient seam, target-path assumption, broad installer, and weak test boundary to exact code. Anthropic best distinguished silent corruption from fail-closed release interruption in cross-review. The single most important issue—the ambient stale-binary bypass—was fixed, together with every other confirmed localized high/medium finding. The remaining parser and test assertions are small localized fixes; broader catalog/CI architecture is not justified in this patch.

### Tooling completeness

The independent response was truncated at the model token limit during DeepSeek's section, although its later two cross-review turns completed. Gemini timed out in the final cross-review round after completing the independent and first cross-review rounds. The other three final-round responses completed. These partial failures did not leave a unique claim unassessed, but they are disclosed rather than treated as four complete final turns.
