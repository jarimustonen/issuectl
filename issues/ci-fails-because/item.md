---
created: 2026-10-05
updated: 2026-10-05
type: bug
reporter: mail-triage
status: open
priority: normal
---

# CI fails because issuectl-core path version mismatches CLI requirement

_Source: github-actions:37288081598_

## Description
CI on main fails on all three platforms and release workflow guard: `crates/issuectl/Cargo.toml` requires `issuectl-core = { path = "../issuectl-core", version = "0.19.0" }`, but the local `issuectl-core` manifest is still at 0.18.7. Cargo cannot select the path dependency even before building. The release workflow's temporary checkout fails for the same reason.

Failed run: https://github.com/jarimustonen/issuectl/actions/runs/37288081598 (commit 5621a23612cacb0499003904f743da33c1c016c8).

## Reproduction
`cargo build --all-targets` on current main; Cargo prints `failed to select a version for the requirement issuectl-core = "^0.19.0"`, `candidate versions found which didn't match: 0.18.7`, exit 101. `scripts/release_workflow.py` also fails at `dist generate --mode ci` exit 255.

## Quick Test
Align workspace manifest versions (and lockfile) in the intended release sequence; run `cargo build --all-targets`, `cargo clippy --all-targets`, and the release workflow guard in CI, verifying green on both Linux and macOS.
