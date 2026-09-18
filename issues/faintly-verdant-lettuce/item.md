---
created: 2026-09-18
updated: 2026-09-18
type: chore
reporter: jari
status: done
priority: normal
provenance: other
provenance_detail: Taskfleet implementation task
source_ref: taskfleet:01m2sm2dm5r2t9xz5s7bg50v8d/task:cargo-dist-0.33.0
originating_run: 01m2sm2dm5r2t9xz5s7bg50v8d
originating_run_kind: spinoff
closed: 2026-09-18
---

# Upgrade cargo-dist generator to 0.33.0

## Description

Upgrade the repository's pinned cargo-dist generator from 0.32.0 to 0.33.0 and regenerate the cargo-dist-owned release workflow using the exact disposable 0.33.0 binary.

Preserve the target matrix, self-hosted macOS ARM64 runner, shell and Homebrew installers, tap publishing, GitHub attestations, and tag trigger. Validate generation, planning, YAML parsing, and the full repository green gate without cutting a release.

## Acceptance Criteria

- [x] `cargo-dist-version` is pinned exactly to 0.33.0 and the owned workflow is regenerated.
- [x] Existing release targets, runners, installers, publishing, attestations, and tag behavior remain intact.
- [x] Exact 0.33.0 generate check, plan, YAML parse, and repository green gate pass.

## Resolution

### 2026-09-18T06:47:09Z · @issuectl

Upgraded the cargo-dist pin to 0.33.0 and regenerated the owned workflow with the exact disposable binary. Verified generation check, JSON plan, YAML parsing, preserved release invariants, and the full repository green gate.
