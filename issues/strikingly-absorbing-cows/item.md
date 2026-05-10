---
created: 2026-05-06
updated: 2026-05-10
type: feature
status: done
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [automation, git-native]
commits:
- hash: d59c1e7
  summary: 'feat(sync-commits): trailer-driven commit linking'
- hash: d4fa6cbc101453558f0049430674663ae7eb545d
  summary: 'chore(issues): record trailer-feature commit via sync-commits'
- hash: 8118c968f91ba669b98336ef0b063a8ad2ce11c8
  summary: 'fix(sync-commits): apply review findings'
closed: 2026-05-10
---

# Git-native commit linking: trailers (Refs-Issue / Fixes-Issue) + sync-commits + branch-name detection

_Source: src/cli/{sync_commits,hooks}.rs (new), .githooks/post-commit_

## Description

Parse standard git trailers in commit messages: 'Refs-Issue: <slug>' (link without status change) and 'Fixes-Issue: <slug>' (suggest closing). 'issuectl sync-commits [--since <ref>]' walks git log, updates each issue's commits: array, optionally proposes status changes. Optional post-commit hook auto-runs sync-commits for the latest commit. Branch-name detection: 'issue/<slug>' / 'feat/<slug>' branches imply linkage. Reduces manual --add-commit calls and aligns the board with what shipped.
