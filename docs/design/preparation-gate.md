# Preparation gate for the scheduling DAG

Issue status describes lifecycle, not permission to start work. A repository can
opt into a separate approval for an issue's **next agreed worker action** (such
as design, verification, or implementation) by adding this to
`issues/.schema.yaml`:

```yaml
preparation_gate: true
```

With no setting (or `false`), scheduling retains its existing behavior. In an
opted-in repo, `preparation` is an optional frontmatter scalar with exactly
three accepted values: `pending`, `reviewing`, `ready`. An absent field is
**not ready**. Use the validated mutation path, for example:

```sh
issuectl update <slug> --field preparation=pending --json
issuectl update <slug> --field preparation=reviewing --json
issuectl update <slug> --field preparation=ready --json
```

`issuectl --json dag` reports `preparation` (null if absent) and
`preparation_reason` (null unless an opted-in head is unready; stable values
`preparation_missing`, `preparation_pending`, `preparation_reviewing`). The text
view shows the same reason on the head. An unready head stays at the front of
its lane: a ready successor does not leapfrog it. Open dependencies can still
advance the head to a different runnable issue, as before. Reservations and
blockers still independently prevent spawning. `spawnable` is false for an
unready head, including one with status `testing`: this governs **new worker
starts**, not completion of tests already underway or status transitions.
Approval is not blanket permission to implement and does not stand in for a
real person's consent. Once the agreed action changes, revisit preparation;
set it back to `pending` or `reviewing` until that next action is approved.

## Migration

Run `issuectl doctor` and `issuectl --json dag` before enabling the gate.
Decide each issue's next action with the owner, record that decision in the
issue, and set preparation for that issue individually. Do **not** bulk-mark
legacy issues ready or infer approval from `in-progress`/`testing` status.
Enable the gate only when the team is prepared for missing states to stop new
spawns. Existing testing may finish without backfilling approval; starting a
new testing worker needs `ready`. `issuectl doctor` validates present values,
but absence is intentional and is reported by the DAG, not a doctor error.
