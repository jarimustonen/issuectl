---
created: 2026-08-12
updated: 2026-08-12
type: improvement
status: open
priority: normal
---

# dag: schema-aware in-progress/underway classification

## Description

`crate::dag` classifies "done-ness" schema-aware via `status_class(schema, &i.status) == StatusClass::Closing`, but detects work-underway with a hardcoded string literal: `const IN_PROGRESS: &str = "in-progress"` and `let underway = i.status == IN_PROGRESS` (dag.rs). A project whose `issues/.schema.yaml` uses a custom underway status (e.g. `running`, `doing`, `active`, or a non-English value) gets `underway = false`, so an already-running head-of-line reads `spawnable: true` — a scheduler trusting `spawnable` could launch a second worker on work in flight.

This is the same class of bug as dag-lists-closed-issues (hardcoded status semantics in a schema-configurable tool), just on the underway axis instead of the closing axis. It predates that fix.

## Fix direction
Introduce a schema-aware "underway/in-progress" notion parallel to `StatusClass::Closing`. Options: a third `StatusClass::InProgress` variant (touches `schema.rs`, `status_classes` merge, and every match on `StatusClass`), or a dedicated `is_underway(schema, status)` helper backed by `status_classes`. Then replace `i.status == IN_PROGRESS` in `make_issue` with the schema-aware check. Add a test: a custom underway status classified in the schema is not spawnable.

## Scope note
Its own design — extends the schema status-class taxonomy and ripples to every `StatusClass` consumer, so it needs a dedicated issue rather than riding along with the closing-status filter.
