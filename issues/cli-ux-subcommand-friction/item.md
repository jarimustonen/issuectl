---
created: 2026-07-26
updated: 2026-07-31
type: feature
status: open
priority: normal
---

## Context

Real friction hit while driving `issuectl` from an agent workflow (deutschpad `/stint`,
2026-07-26). Each cost a failed call + a `--help` round-trip. None are blockers, but
together they add avoidable turns for agents (and humans) who reach for the "obvious"
invocation first.

**Triaged 2026-07-31.** Items #1 and #4 of the original report were addressed by the
v0.6.5 CLI-alias work (`@verb-alias-discoverability`) — see **Resolved** below. The
remaining actionable scope is the three papercuts under **Open**.

## Open (remaining scope)

1. **`issuectl new` rejects a positional title.** `issuectl new "Some title" --type feature`
   fails; the title must be passed as `--title`.
   - Expected: accept a positional title (`issuectl new "Title" --type feature`), matching
     how `note` / `search` take positional text.

2. **`issuectl set <slug> related <slug>` gives a cryptic, self-contradicting error for
   built-in *list* fields.**
   - Observed: `Error: validation: custom field "related" is built-in: --related (repeatable)`.
   - The message names `--related`, but `update` actually exposes
     `--add-related` / `--remove-related`, so following the hint verbatim still fails.
   - Expected: route to the right path, or emit an actionable message naming the exact
     `update --add-<field>` / `--remove-<field>` flags — and make the named flag actually work.

3. **`issuectl note --decision` flag/positional ordering + required `--as`.** First attempt
   `issuectl note <slug> "..." --decision` failed until reordered to
   `issuectl note --as <author> --decision <slug> "..."`.
   - Expected: order-insensitive flags, or a targeted usage error pointing at the missing `--as`.

## Resolved (shipped in v0.6.5)

- **~~`issuectl create` does not exist~~** — DONE. `create` is now an alias for `new`
  (`@verb-alias-discoverability`, v0.6.5).
- **`body <slug> --set-file` / `--from-file` at the top level** — PARTIALLY addressed.
  The exact flag at `body` level still doesn't exist, but `issuectl body <slug>` now emits
  a hint pointing at `body set <slug>` (v0.6.5), which routes the caller. File a dedicated
  scope only if that hint proves insufficient in practice.

## Notes

Environment: installed `issuectl` (deutschpad has `.issuectl/`), `orchestratectl 0.1.0` era.
Discoverability/UX only; low severity. Trimmed 2026-07-31 after v0.6.5 shipped the overlap
(items #1 + #4); #2/#3/#5 remain.
