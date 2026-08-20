---
created: 2026-08-16
updated: 2026-08-20
type: feature
reporter: jari
status: done
priority: normal
closed: 2026-08-16
lane: cli-fixes
lane_seq: 40
provenance: agent-aggountant-wrapup
---

# update --type epic tells you to hand-edit the YAML instead of migrating…

## Description

update --type epic tells you to hand-edit the YAML instead of migrating reporter -> owner

Changing an issue's type to `epic` fails when the issue has a `reporter:` field, and the error instructs the user to edit issuectl's own data file by hand — there is no CLI flag that can clear it.

## Observed

    $ issuectl update phase4-imports --type epic
    Error: validation: type "epic" uses owner, not reporter/assignee; clear assignee/reporter
    from the frontmatter first (edit the YAML directly, or use the JSON API with
    `assignee: null` / `reporter: null`)

`issuectl update --help` offers `--owner`, `--assignee`, `--no-epic`, `--no-lane`, `--no-lane-seq`, `--clear-field` — but `--clear-field` is documented as being only for keys beyond the built-in set, and there is no `--no-reporter` / `--no-assignee`. So the only paths are hand-editing `item.md` or going around the CLI to the JSON API.

I hit this converting four roadmap issues to epics and had to edit four files by hand:

    issues/phase4-imports/item.md
    issues/phase5-financial-statements/item.md
    issues/phase6-payroll/item.md
    issues/phase7-tax-api/item.md

(replacing `reporter: jari` with `owner: jari`), after which `--type epic` succeeded on all four.

## Expected

One of:

1. `--type epic` migrates `reporter:` -> `owner:` itself (it is the same person in the same role — that is exactly what the manual fix does), or
2. `update` gains `--no-reporter` / `--no-assignee` so the documented prerequisite is reachable from the CLI, or
3. the error names an exact command to run rather than telling the user to edit the file.

A CLI whose error message instructs hand-editing its own database is the part that stands out; the validation rule itself seems right.

## Resolution

### 2026-08-16T20:09:02Z · @issuectl

Implemented all three parts: lone reporter now migrates to owner with a visible warning, update has --no-reporter and --no-assignee escape hatches, and ambiguous cases name runnable commands. Also added --no-owner so the non-epic remediation is genuinely runnable. Judgment: matching reporter and owner values are safe to collapse by removing the redundant reporter; a differing owner or any assignee remains an explicit user choice.
