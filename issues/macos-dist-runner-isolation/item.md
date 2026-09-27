---
created: 2026-09-27
updated: 2026-09-27
type: bug
reporter: agent
status: open
priority: normal
---

# Isolate cargo-dist on self-hosted macOS release runner

_Source: .github/workflows/release.yml_

## Description

## Observation

`.github/workflows/release.yml` runs `matrix.install_dist.run` on a `self-hosted` macOS ARM64 runner (`dist-workspace.toml` github-custom-runners). cargo-dist's generated installer writes `dist` to the runner user's persistent `~/.cargo/bin`. A real 2026-09-24 Taskfleet job 107705797379 logged `installing to /Users/jari/.cargo/bin`, showing the same configuration causes persistent runner pollution. Project Canon and issuectl have not yet run this specific release under a new isolation guard; avoid claiming an observed occurrence here.

## Expected

Install the pinned cargo-dist into a unique job-scoped directory on macOS (`RUNNER_TEMP`), verify resolved `dist` is from that directory, and leave the user's persistent Cargo toolchain and all Linux/hosted runners unchanged. Preserve this override through cargo-dist workflow regeneration. Test isolation and generated workflow consistency; only cut a release after this gate is green.
