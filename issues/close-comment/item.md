---
created: 2026-08-05
updated: 2026-08-06
type: feature
status: done
priority: low
commits:
- hash: 26e75b0535ea614d7a92c1258e168fddb06d1feb
  summary: close --comment/--note appends Resolution note
closed: 2026-08-06
---

# issuectl close: accept a --comment/--note to record closing rationale

## Description

## Problem

`issuectl close <slug>` takes only `--status` (plus `--commit`, `--expected-version`). There is no way to record WHY/HOW an issue was closed in the same step — no `--comment`/`--note`.

## Observed

Closing an issue with a resolution note required a manual two-step: `issuectl close <slug> --status done`, then hand-append a "## Resolution" block to `item.md` and commit it separately. (Hit while closing homebrew-tap-distribution / linux-release-binaries.)

## Ask

Add `issuectl close --comment "<text>"` (alias `--note`) that appends a timestamped closing note to the issue body (or a `resolution:` field), so the disposition is captured in one command. Low priority — the manual append works, just clunky.
