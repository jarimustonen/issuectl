---
created: 2026-09-02
updated: 2026-09-03
type: bug
status: in-progress
priority: normal
provenance: ai-review
source_ref: orchestratectl:01m1gc62273xn52kzkgtpd1p73/review-finding:sha1:a4c841fb4444af33cd2a67a4effce804ba6e6854
review_source: ai-review
originating_run: 01m1gc62273xn52kzkgtpd1p73
originating_run_kind: spinoff
assessment_classification: CONFIRMED
assessment_outcome: SPIN_OFF
review_confidence: HIGH
review_target: main..HEAD create --body-file production diff
labels:
- ai-review-model:gemini-3.1-pro-preview
- ai-review-model:gpt-5.6-sol
- ai-review-model:claude-fable-5
- ai-review-model:deepseek-v4-pro
lane: transfer
collision: [crates/issuectl-core/src/transfer.rs]
---

# JSON export-to-import duplicates structured body headings

## Description

## Observed

`issuectl export json --json` emits each issue's complete structured Markdown body in the `body` field. `issuectl import json` deserializes that field through `ImportRecord.description` and creates it as free text, adding a generated `## Description` wrapper.

A direct round-trip on the current tree produced:

```markdown
# Round trip body

## Description

# Round trip body

## Description

Something broke.

## Expected

Something works.
```

The imported issue therefore contains a nested exported H1 and duplicate Description heading.

## Expected

Issuectl's documented own-JSON export/import path should preserve the exported body as structured content, or explicitly transform it without adding duplicate structural headings. Foreign `description` inputs and existing GitHub import semantics must remain free text.

## Triage context

This was independently reproduced during the create-body production diff review. The responsible path is `crates/issuectl-core/src/transfer.rs`: `ImportRecord.description` aliases `body`, while `ImportRecord::into_new_args` selects free-text rendering. The existing `export_json_round_trips_through_import` test only checks export-to-parse and never renders the resulting record. Fixing this needs an explicit design for distinguishing structured `body` from free-text `description`, plus an end-to-end export → parse → create regression.
