---
created: 2026-09-03
updated: 2026-09-03
type: feature
status: in-progress
priority: normal
provenance: other
provenance_detail: orchestratectl implementation task
source_ref: orchestratectl:01m1khdx02st05xkbaf2xqy475/task:canon-s15-skill-targets
originating_run: 01m1khdx02st05xkbaf2xqy475
originating_run_kind: spinoff
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
