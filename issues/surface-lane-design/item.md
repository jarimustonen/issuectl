---
created: 2026-08-16
updated: 2026-08-16
type: feature
status: done
priority: normal
lane: cli-fixes
lane_seq: 70
closed: 2026-08-16
---

# Surface lane-design guidance in dag --help and show per-lane depth

## Description

Follow-up to @intake-feature-issuectl-c633267ba553.

Document the lane-design guidance from [docs/design/lane-design.md](../../docs/design/lane-design.md) in `issuectl dag --help`, and add per-lane depth plus the spawnable-head count to `issuectl dag` output.

Deferred from the documentation-only implementation because both changes touch the CLI/DAG output surface.
