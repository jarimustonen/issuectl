# Designing a lane structure

A lane is a **serial scheduling queue**, not a label for a topic. `issuectl dag`
computes one runnable head of line for each ordinary lane, so a nine-issue lane
represents nine serial slices of work. Start by choosing how many workers you
want to run concurrently: that is the useful upper bound for the number of
ordinary lanes. It is an upper bound rather than a promise, because blockers,
reservations, and collision tokens can make a head ineligible to spawn.

## Draw boundaries around merge conflicts

Put two issues in the same lane when they cannot be independently completed
and merged. Theme names are fine *names* for those lanes, but are a poor reason
to group work: a `reporting` lane with nine unrelated changes still permits
only one of them at a time.

Use `blocked_by` for a real prerequisite. It orders same-lane dependencies and
also prevents an unsatisfied cross-lane dependency from being spawnable.

## Intra-lane ordering

The order is deterministic: `blocked_by` topology first, then priority (high,
normal, low), `lane_seq`, creation time, and finally slug. Priority deliberately
outranks `lane_seq`. Within one priority band, lower `lane_seq` values come
first, and issues with a value come before issues without one. Use `lane_seq`
only for a coarse preference after dependencies and priority, not to invent a
dependency.

Use `collision:` for a shared cross-lane hotspot. Two otherwise independent
lanes may both touch `crates/issuectl/src/main.rs`; give both issues the same
collision token and have the caller pass live reservations to `issuectl dag`.
That temporarily excludes the conflicting candidate without collapsing the two
whole lanes into one. The caller must claim reservations atomically: two DAG
reads can otherwise both report the same collision token as available.

A file that attracts many issues is therefore a scheduling problem. If one
large file forces nine changes into one conflict lane, splitting it into stable
components can create independently mergeable work and increase throughput.
That is a scheduling move, not cosmetic refactoring.

## Useful conventions

- An accepted, ongoing record that is not execution work can remain open
  **outside the DAG** with `scheduling: excluded` and a non-empty
  `scheduling_reason:`. Set both via `create` or `update --field`; it cannot
  also carry `lane`, `lane_seq`, or `collision`. It remains in `list` and
  `show`, but not in `dag`. This is not a holding area for deferred work.
  Clear both fields before assigning a lane. A lane called `parked` is just
  a normal, executable lane; it does not implement exclusion.
- `lane: unlaned` is reserved and means **confirmed parallel-safe**. Its issues
  appear in `unscheduled`; each is its own head and is not serialized with
  other `unlaned` issues. An absent `lane` also appears in `unscheduled`, but
  means **unclassified**, not reviewed. Both can still be blocked by
  `blocked_by` or a reserved `collision:` token.

## Worked example

A first draft groups nine accounting changes under `accounting`. Inspection
shows that three modify the importer, three modify the VAT calculator, and
three only change documentation. Design three lanes instead: `importer`,
`vat`, and `docs`. Each has one possible head, so three workers can proceed
when their heads are runnable. If an importer change and a VAT change both
need `shared.rs`, keep their lanes separate and give those two issues
`collision: [shared.rs]`. If `shared.rs` becomes the repeated bottleneck,
split it so later work no longer needs that collision. Keep executable
findings gated in a lane, mark reviewed independent work as `unlaned`, and
explicitly exclude non-executable records. An issue with no lane or exclusion
still needs classification.

## Dogfood lesson

`issuectl` itself once ran six `cli-fixes` issues in one lane. They were all
serial slices, so that theme-shaped lane serialized the work session. Splitting
future work by independently mergeable conflict boundaries gives the same plan
more usable parallelism without weakening collision safety.
