---
created: 2026-08-05
updated: 2026-08-05
type: bug
status: wontfix
priority: low
closed: 2026-08-05
closed_by: jari
---

# show --json omits closed_by though it is persisted

## Description

# `show --json` omits `closed_by` though it is persisted

_Found while dogfooding 0.7.0 right after release._

## Observed

`issuectl close <slug> --status done --as jari` correctly writes
`closed_by: jari` into the issue's frontmatter (verified in `item.md`), and the
`close` result object echoes `closed_by`. But `issuectl --json show <slug>`
returns `closed_by: null` for the same issue.

## Expected

`show --json` surfaces `closed_by` when present in frontmatter, so the
attribution added by `@close-as-flag-asymmetry` is readable, not write-only.

## Severity

Low — the data is persisted; only the `show` projection drops it. Likely a
missing field in the `show` serializer.

## Comments

### 2026-08-05T08:00:59Z · @jari

Not a bug: closed_by IS surfaced by show --json under .extra.closed_by (verified = 'jari'). I originally queried top-level .closed_by. The enhancement to promote it to a typed top-level field + human output is already tracked by intensely-blushing-galley.
