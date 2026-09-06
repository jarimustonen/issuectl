---
created: 2026-09-03
updated: 2026-09-03
type: chore
status: done
priority: normal
provenance: other
provenance_detail: Orchestrated repository maintenance
source_ref: taskfleet:01m1kqhkep6x7ymm5s1z9vmnze/task:cargo-dist-0.32.0
originating_run: 01m1kqhkep6x7ymm5s1z9vmnze
originating_run_kind: spinoff
closed: 2026-09-03
---

# Upgrade cargo-dist generator to 0.32.0

## Description

Upgrade the repository's pinned cargo-dist generator from 0.28.2 to 0.32.0 and regenerate the cargo-dist-owned GitHub release workflow from `dist-workspace.toml`.

Preserve the configured target matrix, self-hosted macOS ARM64 runner, shell and Homebrew installers, tap publishing, GitHub attestations, and tag-triggered release behavior. Validate the generated workflow, cargo-dist plan/check, and the repository green gate without publishing.

## Acceptance Criteria

- [x] Pin cargo-dist exactly to 0.32.0 and regenerate the owned workflow.
- [x] Preserve and verify the release targets, runner policy, installers, Homebrew publishing, attestations, and tag trigger.
- [x] Pass cargo-dist checks, YAML parsing, and the repository green gate without publishing.

## Resolution

### 2026-09-03T13:40:45Z · @issuectl

Upgraded the pin to cargo-dist 0.32.0 and regenerated release.yml with the exact disposable 0.32.0 binary. Verified dist generate --check, dist plan, workflow YAML parsing, preserved targets/custom runner/installers/Homebrew/attestations/tag trigger, and the full repository green gate.
