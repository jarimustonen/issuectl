---
created: 2026-05-09
updated: 2026-05-09
type: chore
status: open
priority: normal
epic: exorbitantly-ill-apples
---

# server: cache schema + transitions config with mtime invalidation

## Description

Today every PATCH/POST loads issues/.schema.yaml and .issuectl/transitions.yaml from disk. In server mode this is one extra stat+read per write request. Cache them behind an Arc<RwLock<RepoConfig>> with mtime-based invalidation, similar to other on-disk repo state.

Spun off from review of vastly-lyrical-police (transition rules + body section linting). See history/review-transition-rules-linting.md M4.
