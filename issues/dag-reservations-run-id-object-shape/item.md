---
created: 2026-08-12
updated: 2026-08-14
type: improvement
status: done
priority: normal
closed: 2026-08-14
commits:
- hash: edf9e2e
  summary: 'review(dag): collapse duplicate reservation key lists + tighten docs/test'
---

# dag reservations: accept run_id in object shape, not only array-of-holds

## Description

`Reservations::from_json` (dag.rs) accepts two shapes: an object `{"lanes":[..],"collision":[..]}` and an array of holds `[{"run_id"?,"lane"?,"collision"?}]`. `run_id` is allowed inside array-of-holds entries (`RESERVATION_HOLD_KEYS`) but rejected at the top-level object (`RESERVATION_OBJECT_KEYS`), because the strict unknown-key check errors on any key not in the allowed list.

Consequence: a caller wanting to attach a tracking `run_id` to a single hold must wrap it in a one-element array purely to satisfy the parser — `{"run_id":"r1","lane":"x"}` errors, `[{"run_id":"r1","lane":"x"}]` is accepted. That is an arbitrary asymmetry in the AI-first input contract; an agent constructing the object shape will hit a confusing "unknown key" error for a field the array shape happily accepts.

## Fix direction
Add `"run_id"` to `RESERVATION_OBJECT_KEYS` so the object shape accepts (and ignores) a tracking id consistently with the array shape. One-line change plus a test asserting `{"run_id":"r","lane":"x"}` parses.

## Scope note
Unrelated to the DAG closing-status filter; small but touches the reservations-parsing contract and deserves its own issue + test rather than being smuggled into an unrelated fix.

## Resolution

### 2026-08-14T09:31:28Z · @issuectl

Landed on main (edf9e2e): object-shape reservation arm + RESERVATION_KEYS + test. Worker merged code without closing; verified present on main.
