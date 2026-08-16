---
created: 2026-08-16
updated: 2026-08-16
type: feature
reporter: jari
status: open
priority: normal
labels:
- via:agent-aggountant-wrapup
- needs-triage
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
