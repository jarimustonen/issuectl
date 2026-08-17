---
created: 2026-08-17
updated: 2026-08-17
type: feature
reporter: jari
status: open
priority: normal
labels:
- via:agent-homebase-wrapup
- needs-triage
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
