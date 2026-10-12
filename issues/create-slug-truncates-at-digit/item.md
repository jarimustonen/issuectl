---
created: 2026-10-12
updated: 2026-10-12
type: bug
reporter: claude
status: wontfix
priority: normal
provenance: agent
provenance_detail: native-agent-host wrap-up 2026-10-12
closed: 2026-10-12
disposition_note: Title-derived slugs intentionally retain at most three significant words; digits are valid, create --slug already supports the longer requested form.
disposition_reason: by-design
---

# create truncates the derived slug at the first digit

## Description

`issuectl create --type task --title "Night stint 5–6 October 2026" --body-file … --json` in native-agent-host produced the slug `night-stint-5`, dropping the rest of the title. It was fixed afterwards with `issuectl rename night-stint-5 night-stint-2026-10-05`.

Expected: the derived slug keeps the meaningful words (for example `night-stint-5-6-october-2026`), or `create` accepts `--slug` as `intake file` already does.

## Comments

### 2026-10-12T11:07:03Z · @intake

Rejected (by-design): Title-derived slugs intentionally retain at most three significant words; digits are valid, create --slug already supports the longer requested form.
