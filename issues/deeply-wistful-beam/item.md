---
created: 2026-05-09
updated: 2026-05-09
type: chore
status: in-progress
priority: normal
epic: exorbitantly-ill-apples
commits:
- hash: 4db5f5c
  summary: initial server config cache implementation
- hash: 78c392c
  summary: tighten cache from llm-review feedback (Arc returns, mtime+len, !Send guard, server-level test, filetime)
- hash: 8c3efd8
  summary: file thread-local cache spin-off (@hugely-madly-haircut) under v0.6.0 backlog
- hash: ac87147
  summary: round-2 review fixes (NotFound-only stamp, re-stat under lock, !Send marker doc, root debug_assert, test pin assertions, 4 new tests)
---

# server: cache schema + transitions config with mtime invalidation

## Description

Today every PATCH/POST loads issues/.schema.yaml and .issuectl/transitions.yaml from disk. In server mode this is one extra stat+read per write request. Cache them behind an Arc<RwLock<RepoConfig>> with mtime-based invalidation, similar to other on-disk repo state.

Spun off from review of vastly-lyrical-police (transition rules + body section linting). See history/review-transition-rules-linting.md M4.
