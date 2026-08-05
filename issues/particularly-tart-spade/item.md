---
created: 2026-08-05
updated: 2026-08-05
type: bug
status: open
priority: low
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
