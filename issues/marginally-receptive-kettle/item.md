---
created: 2026-05-10
updated: 2026-05-10
type: bug
reporter: jari
status: fixed
priority: high
epic: hugely-exciting-spiders
labels: [from-3dbear-0.5.1-feedback]
closed: 2026-05-10
commits:
- hash: f648e2f6
  summary: 'fix(list): truncate titles on char boundaries'
---

# issuectl list panics on non-ASCII titles (UTF-8 byte-vs-char boundary)

## Description

Regression of 0.3.1 Finding 1. Panic moved from src/main.rs:1162 to crates/issuectl/src/main.rs:1999 — table renderer was refactored but byte-index slicing remains. Repro: any title with ä/ö in Finnish-language repo. Fix: use s.chars().take(N) or unicode-width instead of byte-index slicing. See @intensely-ill-garden for full feedback context (3DBear monorepo 0.3.1 → 0.5.1 migration, 2026-05-10).
