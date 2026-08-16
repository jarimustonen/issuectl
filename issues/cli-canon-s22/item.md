---
created: 2026-08-16
updated: 2026-08-16
type: improvement
status: in-progress
priority: normal
labels: [tooling, cli-canon]
lane: cli-canon
lane_seq: 80
commits:
- hash: db047aaec356bf85b3f6d1f8b2b3e1a8620cdef5
  summary: inject Clock into core time paths
---

# cli-canon: §22 Internal layout: library-first core/cli split

## Description

Filed by the `stack-cli-alignment` comparative-audit rollout (homebase epic), via `project-canon review --assume-defaults` (project-canon 0.1.1). Advisory recommendation: **severity: should**, never a release-readiness failure.

**Correction (2026-08-16):** the original `--assume-defaults` audit misread this repository. The workspace already has `crates/issuectl` and `crates/issuectl-core`; the core crate is already clap-free. This issue's remaining actionable delta is an injected `Clock` seam in core for deterministic date-derived behavior and tests.

**Canon:** AGENTS-AI-FIRST-CLI.md §22, with the clock requirement from §19.

**Deliberate decisions:** I/O remains in `issuectl-core` because the filesystem-backed issue records are this project's domain, so abstracting disk access would not improve its hermetic-tempdir tests. The published `issuectl` crate is not renamed to `issuectl-cli`, because that would break its crates.io name. A `*-cli-common` crate is not added because the family has not converged on one.

**Check:** read `Cargo.toml` members; verify `issuectl-core` has no clap dependency; exercise time-dependent core behavior through a fixed `Clock`.
