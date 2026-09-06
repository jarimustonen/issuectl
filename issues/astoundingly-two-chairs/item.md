---
created: 2026-09-03
updated: 2026-09-03
type: feature
status: done
priority: normal
provenance: other
provenance_detail: taskfleet implementation task
source_ref: taskfleet:01m1khdx02st05xkbaf2xqy475/task:canon-s15-skill-targets
originating_run: 01m1khdx02st05xkbaf2xqy475
originating_run_kind: spinoff
commits:
- hash: 3fb4ee20a9abfad4dd7fd5bc973dd589ed3734f1
  summary: 'feat(skill): make agent targets first-class'
closed: 2026-09-03
---

# Make Claude pi and Codex skill targets first-class

## Description

## Goal

Bring the bundled skill installer into conformance with project-canon 0.8.0 §15: first-class Claude, pi, and Codex targets; default/all selection; target override; dry-run; no-clobber/force behavior; and complete machine-readable capability metadata.

## Acceptance Criteria

- [x] `skill list --json` declares `supported_agents` and complete install/layout capabilities.
- [x] `skill install` supports `claude`, `pi`, `codex`, and `all`, defaults to `all`, and supports `--target`, `--dry-run`, and explicit `--force`.
- [x] All three bundled skills install without drift in each native layout.
- [x] Focused tests, docs, templates, dogfooded copies, canon §15 checks, and the repository green gate pass.

## Resolution

### 2026-09-03T12:09:52Z · @issuectl

Implemented and verified with the full repository green gate and disposable project-canon §15 sandbox checks.
