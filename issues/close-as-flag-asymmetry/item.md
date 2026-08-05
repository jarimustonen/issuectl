---
created: 2026-08-04
updated: 2026-08-05
type: improvement
status: in-progress
priority: low
---

# `--as <author>` accepted by `note` but rejected by `close` (verb inconsistency)

_Source: /wrap-up, stint 2026-08-04._

## Observed

`issuectl note` **requires** `--as <AUTHOR>`:

    $ issuectl note some-slug "text"
    error: the following required arguments were not provided:
      --as <AUTHOR>

But `issuectl close` **rejects** it outright:

    $ issuectl close some-slug --status wontfix --as jari
    error: unexpected argument '--as' found
      tip: to pass '--as' as a value, use '-- --as'

So the two mutation verbs disagree on the `--as` flag: one mandates it, the
other refuses it. During a real session this cost a retry — the natural
"close and record who closed it" gesture fails.

## Expected

Consistent author handling across mutation verbs. Either:
- **A (preferred):** `close` accepts an optional `--as <author>` and records the
  closer (mirrors how `note` attributes an author), so lifecycle transitions can
  carry attribution; or
- **B (minimum):** if `close` deliberately takes no author, the flags are at
  least documented as intentionally different, and the rejection message points
  at the reason.

Today `close` only writes status frontmatter, so there is no author field to set
— which is exactly the gap: a disposition (wontfix / done) has no record of who
made the call, unlike a note.

## Severity

Low — ergonomic/consistency, not a correctness bug. Papercut for agents that
attribute lifecycle actions.
