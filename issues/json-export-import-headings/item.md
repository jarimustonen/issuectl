---
created: 2026-09-02
updated: 2026-09-03
type: bug
status: fixed
priority: normal
provenance: ai-review
source_ref: taskfleet:01m1gc62273xn52kzkgtpd1p73/review-finding:sha1:a4c841fb4444af33cd2a67a4effce804ba6e6854
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
commits:
- hash: 4310b37
  summary: preserve structured JSON bodies
- hash: e34e192
  summary: validate JSON body semantics after review
- hash: 9c03356
  summary: preserve structured JSON bodies (rebased)
- hash: 3c82522
  summary: validate JSON body semantics after review (rebased)
closed: 2026-09-03
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

## Acceptance Criteria

- [x] issuectl JSON export/import preserves one title H1 and one structured Description section.
- [x] Foreign description, GitHub body, and legacy plain-body imports retain free-text rendering.
- [x] Focused regressions, multi-model review, finding assessment, and the full green gate pass.


## Resolution

### 2026-09-03T12:57:58Z · @issuectl

Implemented explicit structured-body versus free-text import decoding, preserved legacy plain-body and GitHub semantics, rejected ambiguous body/description records, and verified exact export/import rendering. Multi-model review findings were assessed and all confirmed localized fixes applied. Full repository green gate passed: fmt, clippy, tests, build, and rustdoc.
