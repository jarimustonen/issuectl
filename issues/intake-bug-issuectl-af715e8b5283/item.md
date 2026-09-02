---
created: 2026-08-28
updated: 2026-09-02
type: bug
reporter: jari
status: fixed
priority: normal
provenance: agent:aggountant-wrapup
source_ref: agent:aggountant-wrapup/reporter:jari/id:aggountant-2026-08-28-issuectl-doctor-owned-fields
lane: doctor-schema
collision: [crates/issuectl-core/src/schema.rs]
commits:
- hash: 71eecca46dfefb69608148812e14ba23cec81b4f
  summary: recognize workflow-owned metadata in default schema
- hash: a900aa64ca420db0d116169b2e5825af0d5d44ea
  summary: apply review fixes and preserve review assessment
closed: 2026-09-02
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

## Resolution

### 2026-09-02T05:46:18Z · @issuectl

Registered the complete workflow metadata vocabulary in the layered default schema, retained exact unknown-key warnings, and verified schema installation plus Issue.extra projection. Full workspace green gate and multi-model review passed.
