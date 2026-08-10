---
created: 2026-08-10
updated: 2026-08-10
type: feature
status: done
priority: low
commits:
- hash: '44267e7'
  summary: 'warn at authoring time on reserved ## Notes section'
- hash: d03fc38
  summary: review fixes - raw-body scan, wording, tests
closed: 2026-08-10
---

# Warn at authoring time when an issue body uses the reserved '## Notes' section

## Description

`## Notes` is treated by `issuectl doctor` as a legacy alias that must be renamed/merged into `## Comments`. But `issuectl new --body-file` (and `issuectl body`) happily accept a body containing a `## Notes` heading with no warning. `issuectl note` then appends to `## Comments`, and if the issue also has a `## Notes` section, the pre-commit `doctor` hook blocks the commit with "Files with both `## Notes` and `## Comments` (manual merge required)".

## Friction observed (homebase, 2026-08-10)
1. `issuectl new ... --body-file -` with a body containing `## Notes` → created fine, no warning.
2. Later `issuectl note --as X <slug> ...` → appended to `## Comments`.
3. `git commit` → pre-commit `doctor` blocked: both sections present, manual merge required.

The reserved-name collision is only discovered at commit time, several steps after the section was introduced.

## Suggestions (any one)
- Warn (or reject with `--force` to override) at `issuectl new` / `issuectl body` time when the body contains a reserved legacy section name like `## Notes`.
- Let `doctor --fix` auto-merge `## Notes` into `## Comments` even when both exist (currently "manual merge required"), preserving order.
- Document the reserved section names in the `new`/`body` help text.

## Minor adjacent note
`issuectl note` requires `--as <AUTHOR>`; omitting it prints only "For more information, try '--help'." A more specific "the following required arguments were not provided: --as <AUTHOR>" (clap's standard) would help — worth confirming it isn't being suppressed.
