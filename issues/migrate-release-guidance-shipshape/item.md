---
created: 2026-08-23
updated: 2026-09-02
type: chore
status: done
priority: normal
provenance: other
provenance_detail: fleet product migration
source_ref: taskfleet:01m0qg3nqzb6m7vqd5ac001bdm/task:shipshape-product-migration
originating_run: 01m0qg3nqzb6m7vqd5ac001bdm
originating_run_kind: spinoff
closed: 2026-09-02
closed_by: agent-triage
---

# Migrate active release guidance to Shipshape

## Description

## Goal

Migrate this repository's active release-engine documentation and operational contract from the retired `ossctl` product/CLI and `/oss-*` skill namespace to Shipshape (`shipshape`, `/shipshape-*`).

## Scope

Semantically inspect every tracked `ossctl` match. Update active commands, skill references, setup and current release guidance. Preserve historical changelog entries, issue records, release evidence, and the permanent compatibility identifiers required by Shipshape ADR-0005, including the `OSS-RELEASE.md` filename.

## Acceptance

- Active release instructions consistently invoke `shipshape` and `/shipshape-*`.
- Historical and compatibility references remain intact and are documented in the migration report.
- The repository green gate passes.

## Comments

### 2026-08-23T14:36:17Z · @agent-spinoff

Implementation complete in run 01m0qg3nqzb6m7vqd5ac001bdm. Active release guidance now uses Shipshape commands and /shipshape-* skills; historical issue bodies, changelog entries, dated release evidence, the legacy GitHub repository coordinate, and the OSS-RELEASE.md compatibility filename were deliberately retained. Full repository green gate passed: fmt, clippy -D warnings, workspace tests, workspace build, and rustdoc -D warnings.

## Resolution

### 2026-09-02T05:12:59Z · @agent-triage

Active release guidance migration and its green gate were already completed; machine-level Shipshape rollout remains separate Homebase work.
