---
created: 2026-09-27
updated: 2026-09-27
type: bug
reporter: agent
status: fixed
priority: normal
closed: 2026-09-27
closed_by: agent
---

# Isolate cargo-dist on self-hosted macOS release runner

_Source: .github/workflows/release.yml_

## Description

## Observation

`.github/workflows/release.yml` runs `matrix.install_dist.run` on a `self-hosted` macOS ARM64 runner (`dist-workspace.toml` github-custom-runners). cargo-dist's generated installer writes `dist` to the runner user's persistent `~/.cargo/bin`. A real 2026-09-24 Taskfleet job 107705797379 logged `installing to /Users/jari/.cargo/bin`, showing the same configuration causes persistent runner pollution. Project Canon and issuectl have not yet run this specific release under a new isolation guard; avoid claiming an observed occurrence here.

## Expected

Install the pinned cargo-dist into a unique job-scoped directory on macOS (`RUNNER_TEMP`), verify resolved `dist` is from that directory, and leave the user's persistent Cargo toolchain and all Linux/hosted runners unchanged. Preserve this override through cargo-dist workflow regeneration. Test isolation and generated workflow consistency; only cut a release after this gate is green.

## Reopen Notes — 2026-09-27

_Add rationale for reopening here._

## Comments

### 2026-09-27T06:53:36Z · @agent

Real 0.18.6 release attempt 2026-09-27, Shipshape run 01M3GSYQA8SH57FATVFZXD5J9J, failed safely at build before crates/tag/publication: dist build rejects modified .github/workflows/release.yml as stale because dist-workspace.toml lacks allow-dirty = ["ci"]. Run was abandoned with reason. Reopen until the scoped allowance, regeneration checker and actual pre-publish dist build pass; then reseal a new plan and publish. Do not regenerate away the runner isolation override.
