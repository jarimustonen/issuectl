---
created: 2026-08-28
updated: 2026-08-28
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:aggountant-wrapup
source_ref: agent:aggountant-wrapup/reporter:jari/id:aggountant-2026-08-28-issuectl-doctor-owned-fields
---

# doctor warns about issuectl-owned intake fields

## Description

doctor warns about issuectl-owned intake fields

Running `issuectl doctor` in aggountant reports unknown frontmatter keys that issuectl's own intake/review workflow wrote.

Exact command:

    issuectl doctor

Observed examples:

    Unknown frontmatter keys (not declared by schema):
      acceptably-sharp-beetle: originating_run
      acceptably-sharp-beetle: originating_run_kind
      essentially-mindless-attraction: review_source
      essentially-mindless-attraction: originating_run
      essentially-mindless-attraction: originating_run_kind
      essentially-mindless-attraction: review_target
      essentially-mindless-attraction: assessment_classification
      essentially-mindless-attraction: assessment_outcome
      essentially-mindless-attraction: review_severity

These fields were produced by the issuectl-managed intake/review path rather than hand-authored as arbitrary extension fields.

Expected: issuectl's standard schema and doctor agree about fields written by issuectl-owned workflows. Doctor should not warn about its own valid output; alternatively the writer must declare/install the needed schema fields when it writes them.
