---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, web-ui, git-native]
---

# Web board: uncommitted-state indicator on cards

## Description

Web board reads local files; when an issue's item.md has uncommitted git changes (modified, untracked, or staged but not committed), show a marker (asterisk / dot / yellow border) on the card. Reminds the user/agent that the source of truth isn't synced to git yet. Trivial to detect via 'git status --porcelain issues/'.
