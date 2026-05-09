---
created: 2026-05-09
updated: 2026-05-09
type: chore
status: open
priority: normal
epic: exorbitantly-ill-apples
---

# doctor: single-pass issue scanner

## Description

Doctor currently re-walks issues/ multiple times (schema validation, transition warnings, body-section linting, orphan epic refs, etc.). For repos with many issues this doubles+ the parse cost. Refactor to a single canonical scan_issues(repo_root) -> Vec<ScannedIssue> consumed by every doctor check.

Spun off from review of vastly-lyrical-police (transition rules + body section linting). See history/review-transition-rules-linting.md C4 and the moderator notes.
