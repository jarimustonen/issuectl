---
created: 2026-08-15
updated: 2026-08-15
type: bug
reporter: jari
status: fixed
priority: normal
lane: cli-fixes
lane_seq: 20
collision: [crates/issuectl/src/main.rs]
closed: 2026-08-15
---

# list --status done returns 'No issues found' despite done issues existing

_Source: cli/list status filter_

## Description

**Observed (2026-08-15, running issuectl 0.10.0 in the aggountant repo):** closed several issues so their frontmatter reads `status: done` (verified by hand: `grep '^status:' issues/<slug>/item.md` → `status: done`). They also appear in bare `issuectl list` and in `issuectl dag`. But **`issuectl list --status done` prints `No issues found`** — the filter does not match issues whose status is `done`.

**Expected:** `list --status done` lists every issue whose `status:` is `done` (and likewise for other closing statuses like `fixed`/`wontfix`). 

**Repro:**
```
issuectl close <slug> --status done      # frontmatter now: status: done
issuectl list --status done              # → 'No issues found'  (bug)
grep '^status:' issues/<slug>/item.md    # → status: done
```

**Guess at cause:** `--status` may only be honoured against the open/active set, or it compares against a normalized value that closing statuses don't map to. Whatever the internal filtering, a status value that a closed item literally carries should be selectable via `--status`.
