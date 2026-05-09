---
created: 2026-05-09
updated: 2026-05-09
type: feature
status: in-progress
priority: normal
epic: exorbitantly-ill-apples
---

# issuectl update --type: scaffold or reject when required body sections are missing

## Description

cmd_new emits per-type required body sections as stubs at creation. update --type doesn't — changing an issue's type to one with stricter requirements leaves it doctor-failing without warning at the mutation site. Decide and implement: append-on-type-change, reject-on-type-change with required sections missing, or document the current 'doctor lints later' model.

Spun off from review of vastly-lyrical-police (transition rules + body section linting). See history/review-transition-rules-linting.md M11.
