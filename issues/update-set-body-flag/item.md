---
created: 2026-08-14
updated: 2026-08-14
type: feature
status: done
priority: normal
closed: 2026-08-14
commits:
- hash: 25e7d45
  summary: 'feat(update): add --body-file/--description to set/replace issue body'
---

# issuectl update: add --body-file/--description to set or replace an existing issue body

## Description

**Observed:** `issuectl new` accepts `--description` / `--body` / `--body-file` to set the initial body, but `issuectl update <slug>` has no equivalent. Passing `--description-file` to `update` hard-fails:

    $ issuectl update vat-policy-full-rate-history --description-file /tmp/desc.md
    error: unexpected argument '--description-file' found
    tip: to pass '--description-file' as a value, use '-- --description-file'

There appears to be no flag on `update` to set/replace the body at all.

**Expected:** `update` should offer a symmetric way to set/replace the issue body from the CLI, e.g. `--description`/`--body` and `--body-file -` (stdin), matching `new`.

**Impact / workaround:** to give a freshly-created issue a real body I had to append Markdown directly to `issues/<slug>/item.md` by hand, bypassing the CLI. Minor but a real asymmetry — agents naturally reach for `update --body-file` after `new`.

**Note on flag naming:** `new` uses `--body-file`; I instinctively tried `--description-file` on `update`. Whatever `update` gains, matching `new`'s exact flag names (`--body-file`, `--description`/`--body`) would avoid the confusion.

## Resolution

### 2026-08-14T10:07:53Z · @issuectl

Landed on main (25e7d45/9cb71fc): update gains --description/--body/--body-file (with - = stdin) mirroring new, routed through UpdateIssueRequest.set_body under flock. Worker merged without closing; verified on main.
