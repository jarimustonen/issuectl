---
created: 2026-08-15
updated: 2026-08-15
type: improvement
assignee: ai
status: open
priority: normal
labels: [sync-commits]
lane: cli-fixes
lane_seq: 60
collision: [crates/issuectl/src/main.rs]
---

# sync-commits should warn when default range is empty on main

## Description

## Description

`issuectl sync-commits` can be surprising when run directly on `main`: its default range is documented as `<merge-base of HEAD and main/master>..HEAD`. On `main`, the merge-base is often `HEAD`, so the default becomes effectively `HEAD..HEAD` and scans no commits.

Observed in homebase after merging a spinoff commit whose message contained a valid trailer:

```text
Refs-Issue: @light-llm-server
```

Running:

```bash
issuectl --json sync-commits
```

reported an empty plan and a range equivalent to the current HEAD boundary, so the just-landed commit was not recorded. The workaround was to add the commit manually, or to run an explicit inclusive-enough range such as:

```bash
issuectl sync-commits --range HEAD~1..HEAD
# or before push:
issuectl sync-commits --range origin/main..HEAD
```

The underlying Git range behaviour is correct: `A..B` excludes `A`. The issue is that the default range silently becomes empty in a common agent workflow: commit or merge on `main`, then run `sync-commits`.

## Expected behaviour

Make the empty-default-range case visible or safer. Recommended behaviour:

- If the computed default range contains zero commits, emit a warning in JSON and text output explaining that the default range is empty and suggesting `--range HEAD~1..HEAD` or `--range origin/main..HEAD`.
- Optionally, when on a branch with an upstream, consider defaulting to `@{u}..HEAD` instead of `merge-base(HEAD, main)..HEAD`, if that is more aligned with the intended “record commits I just made before pushing” workflow.

## Acceptance Criteria

- [ ] Running `sync-commits` on `main` with an empty computed default range does not silently look successful without explaining that no commits were scanned.
- [ ] `--json` output carries a machine-readable warning or metadata that agents can surface.
- [ ] The `/issue` skill docs or command help clarify the correct explicit range for “record the last commit on main”.
