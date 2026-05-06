---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: exorbitantly-ill-apples
labels: [v0.6.0-candidate, validation]
---

# Declarative status transition rules + per-type body section linting

## Description

Two related ideas, one issue. (1) Transition rules in .issuectl/transitions.yaml: e.g. 'done requires checked acceptance criteria, requires assignee'. Enforced by safe-mutation CLI and doctor. (2) Per-type body linting: bug requires '## Steps to Reproduce', '## Expected', '## Actual'; feature requires '## Problem', '## Acceptance Criteria'; epic requires '## Goal', '## Issues'. Both extend doctor (@slightly-finicky-heart) without changing frontmatter schema.
