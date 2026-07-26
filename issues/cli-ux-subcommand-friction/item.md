---
created: 2026-07-26
updated: 2026-07-26
type: feature
status: open
priority: normal
---

## Context

Real friction hit while driving `issuectl` from an agent workflow (deutschpad `/stint`,
2026-07-26). Each of these cost a failed call + a `--help` round-trip. None are blockers,
but together they add avoidable turns for both humans and agents who reach for the
"obvious" invocation first.

## Observed vs. expected

1. **`issuectl create` does not exist** — the intuitive verb. Actual is `new`.
   - Observed: `error: unrecognized subcommand 'create'` (tip suggests `ready`/`rename`, not `new`).
   - Expected: either accept `create` as an alias for `new`, or have the "did you mean"
     tip point at `new`.

2. **`issuectl new` rejects a positional title** — `issuectl new "Some title" --type feature`
   fails; you must pass `--type` AND `--title`.
   - Expected: accept a positional title (`issuectl new "Title" --type feature`), matching
     how `note`/`search` take positional text.

3. **`issuectl set <slug> related <slug>` gives a cryptic error for built-in *list* fields.**
   - Observed: `Error: validation: custom field "related" is built-in: --related (repeatable)`.
   - Expected: route to the right path (or a clear "use `update --add-related` / `--remove-related`").
     The message names `--related` but `update` actually exposes `--add-related`/`--remove-related`,
     so following the hint verbatim still fails.

4. **`issuectl body --set-file` / `--from-file` at the top level doesn't exist** — the flag
   lives under the `body set` subcommand (`issuectl body set <slug> --from-file PATH`).
   - Observed: `issuectl body <slug> --set-file PATH` → usage error.
   - Expected: clearer help surfacing `body set --from-file`, or accept the flag at `body` level.

5. **`issuectl note --decision`** — flag/positional ordering + required `--as` tripped the
   first attempt (`issuectl note <slug> "..." --decision` failed until reordered to
   `issuectl note --as <author> --decision <slug> "..."`).
   - Expected: order-insensitive flags, or a clearer usage error pointing at the missing `--as`.

## Suggested fix (pick per maintainer taste)

- Add verb aliases (`create`→`new`) and/or improve the "did you mean" suggestions to point
  at the real subcommand.
- Accept positional title in `new`.
- For built-in list fields passed to `set`, emit an actionable message naming the exact
  `update --add-<field>` / `--remove-<field>` flags (and make the flag name in the error match).
- Ensure `--as` omission on `note` produces a targeted error.

## Notes

Environment: installed `issuectl` (deutschpad has `.issuectl/`), `orchestratectl 0.1.0` era.
Reported from an agent session; low severity, discoverability/UX only.
