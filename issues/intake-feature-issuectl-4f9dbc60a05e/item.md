---
created: 2026-08-17
updated: 2026-08-19
type: feature
reporter: jari
status: done
priority: normal
labels:
- via:agent-homebase-wrapup
lane: intake
lane_seq: 20
collision: [crates/issuectl-core/src/doctor, crates/issuectl-core/src/agents.rs]
commits:
- hash: dd06403
  summary: start deferred label retirement
- hash: b6bd6ee
  summary: add doctor cleanup and agent guidance
- hash: 05cf99a
  summary: preserve intake migration semantics and structured partial errors
- hash: f59d0dc
  summary: record implementation commits
- hash: 25ca28d
  summary: add doctor cleanup and agent guidance (rebased)
- hash: 1dd3a76
  summary: preserve intake migration semantics and structured partial errors (rebased)
- hash: edb3a86
  summary: record implementation commits (rebased)
- hash: 644e91b
  summary: close deferred label retirement (rebased)
closed: 2026-08-19
---

# Retire the deferred label; doctor must flag residual uses

## Description

Retire the deferred label; doctor must flag residual uses

Decision (Jari, 2026-08-17, orchestratectl stint-5 wrap-up): the `deferred` label is RETIRED.

Background: `issuectl dag` surfaced a `deferred`-labelled issue as a lane head (orchestratectl's `config-show-layered-view`, label set 2026-08-16, stale by the time it scheduled). The old markdown-DAG convention excluded `deferred` from the active set; `issuectl dag` has no such exclusion. Rather than teach `dag` the label, the label goes away — under the no-backlog model (ADR 0010) an open issue is laned or untriaged-transient, and "deferred" is expressed by lane position / blocked_by, not a label.

Expected:
- `issuectl doctor` flags any issue still carrying the `deferred` label (residual from the retired convention) and suggests removing it — ideally as a `--fix`-able check.
- Documentation (issues/AGENTS.md schema prose) stops mentioning `deferred` as a lifecycle label.
- `dag`/`ls` need no new logic; the label simply stops existing.

Observed today: `issuectl dag` (issuectl current as of 2026-08-17) ranked the deferred-labelled issue as head-of-line with no indication of the label.

## Comments

### 2026-08-17T17:16:21Z · @agent-stint

Triage: accepted (maintainer decision recorded in the report). Retire the deferred label: doctor warning (+ --fix removal), scaffold prose update; check interaction with intake_migrate's deferred-label handling. Laned to intake (seq 20) because it touches the same legacy-label vocabulary the queue fix is reworking.
