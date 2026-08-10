---
created: 2026-05-06
updated: 2026-08-10
type: feature
status: obsolete
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [git-native, v0.6.0-candidate, web-ui, deferred]
closed: 2026-08-10
---

# Web board: uncommitted-state indicator on cards

## Description

Web board reads local files; when an issue's item.md has uncommitted git changes (modified, untracked, or staged but not committed), show a marker (asterisk / dot / yellow border) on the card. Reminds the user/agent that the source of truth isn't synced to git yet. Trivial to detect via 'git status --porcelain issues/'.

## Resolution

### 2026-08-10T10:03:40Z · @issuectl

Web/browser UI is being removed from issuectl (product decision 2026-08-10). This is a web-board enhancement, so it is obsolete. See @remove-web-ui.
