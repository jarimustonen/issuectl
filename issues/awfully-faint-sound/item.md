---
created: 2026-05-06
updated: 2026-05-07
type: improvement
reporter: jari
assignee: jari
status: testing
priority: high
epic: exorbitantly-ill-apples
labels: [breaking, foundation, migration]
commits:
- hash: 2809ded
  summary: 'refactor: flat layout (issues/<slug>/item.md, no open/closed split)'
- hash: 501a1bd
  summary: 'fix: apply review findings from awfully-faint-sound flat-layout'
---

# Migrate to flat layout: issues/<slug>/item.md (status only in YAML, not in path)

_Source: src/repo.rs, src/cli/{close,update,doctor,migrate}.rs, src/web/api.rs_

## Description

Today statuses are encoded in directory paths: issues/open/<slug>/ vs issues/closed/<slug>/. Drawbacks: (1) every status crossing causes a git rename/delete pair, breaking IDE tabs and markdown relative links; (2) the in-flight web-edit-sync design (docs/design/web-edit-sync.md §3.4, §6.2) carries significant complexity solely to handle this rename — flat layout eliminates it; (3) statuses like 'testing' / 'done' / 'wontfix' don't fit cleanly under 'open' or 'closed' anyway. Proposal: move to issues/<slug>/item.md with status strictly in YAML frontmatter. Path stable across status changes. issues/archive/YYYY/<slug>/ remains as cold storage (separate from status). Add 'issuectl migrate layout' that performs the move and a compat layer that reads both layouts during the transition. STRONG RECOMMENDATION: land this BEFORE the web-edit-sync M1 implementation, so M1 ships against the simpler model.
