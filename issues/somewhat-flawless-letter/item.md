---
created: 2026-05-06
updated: 2026-05-06
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [git-native, v0.6.0-candidate, web-ui]
---

# Web board: uncommitted-state indicator on cards

## Description

Web board reads local files; when an issue's item.md has uncommitted git changes (modified, untracked, or staged but not committed), show a marker (asterisk / dot / yellow border) on the card. Reminds the user/agent that the source of truth isn't synced to git yet. Trivial to detect via 'git status --porcelain issues/'.
