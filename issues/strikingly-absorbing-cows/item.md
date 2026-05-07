---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [git-native, automation]
---

# Git-native commit linking: trailers (Refs-Issue / Fixes-Issue) + sync-commits + branch-name detection

_Source: src/cli/{sync_commits,hooks}.rs (new), .githooks/post-commit_

## Description

Parse standard git trailers in commit messages: 'Refs-Issue: <slug>' (link without status change) and 'Fixes-Issue: <slug>' (suggest closing). 'issuectl sync-commits [--since <ref>]' walks git log, updates each issue's commits: array, optionally proposes status changes. Optional post-commit hook auto-runs sync-commits for the latest commit. Branch-name detection: 'issue/<slug>' / 'feat/<slug>' branches imply linkage. Reduces manual --add-commit calls and aligns the board with what shipped.
