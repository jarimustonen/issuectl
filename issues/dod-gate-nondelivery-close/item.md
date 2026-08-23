---
created: 2026-08-21
updated: 2026-08-23
type: bug
reporter: jari
status: in-progress
priority: low
lane: lifecycle
---

# close: the Definition-of-Done gate fires on non-delivery closing statuses

## Description

## Description

The Definition-of-Done gate fires on **every** closing status, including the ones that mean
"no work was delivered". Closing an issue as a duplicate emits an acceptance-criteria warning
that cannot be satisfied without lying.

## Observed

```console
$ issuectl close apply-inline-json --status duplicate --as agent-triage --comment "..."
Closed apply-inline-json (by agent-triage)
warning: dod: status "duplicate" requires `## Acceptance Criteria` with at least one checked
item, but the section is missing (set `dod.strict: true` in `.schema.yaml` to block)
```

`evaluate_dod` in `crates/issuectl-core/src/transitions.rs` gates on the transition being
*into a closing status* and does not distinguish which closing status it is. It correctly
skips non-closing transitions and already-closing states, but treats `duplicate`, `wontfix`,
`obsolete`, and `cannot-reproduce` exactly like `fixed` and `done`.

## Expected

The gate applies to **delivery** closes (`fixed`, `done`) and stays silent for
**non-delivery** closes (`duplicate`, `wontfix`, `obsolete`, `cannot-reproduce`). An issue
closed as a duplicate has, by definition, no acceptance criteria of its own to satisfy — the
surviving issue carries them.

## Why it matters

Two effects, both real:

1. **Noise in the default configuration.** Every duplicate/wontfix close prints a warning
   that the closer must consciously ignore. Warnings that are routinely ignored stop being
   read, which weakens the DoD gate where it actually matters.
2. **A hard block under `dod.strict: true`.** In a repo that opts into strict mode, closing a
   duplicate becomes impossible without first adding a fake checked acceptance-criteria
   section. That is a correctness problem, not a cosmetic one: strict mode should tighten the
   delivery gate, not prevent triage hygiene.

## Suggested fix

Classify closing statuses rather than treating them as one set. The schema already carries
`status_classes` for the closing/non-closing distinction; a delivery-vs-non-delivery split is
the natural extension. A conservative first cut is to gate DoD on the built-in delivery
statuses only, and let a project widen the delivery set in `issues/.schema.yaml` if it
declares custom closing statuses that do imply delivery.

Note the schema-config angle when picking the shape: hardcoding a status list in
`transitions.rs` reintroduces the same rigidity for projects with custom closing statuses.

## Provenance

Observed 2026-08-20 during an unlaned-issue triage pass in this repo, while closing
@apply-inline-json as a duplicate of @apply-patch-from-stdin. Confirmed against
`transitions.rs` (the gate keys on "is closing", not on which closing status).
