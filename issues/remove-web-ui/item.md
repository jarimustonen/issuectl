---
created: 2026-08-10
updated: 2026-08-12
type: chore
status: in-progress
priority: normal
commits:
- hash: 8a69c097522f75c851b7ab24c96f051540b33bd0
  summary: remove web UI + HTTP surface (serve, /api, kanban, watcher, RepoConfigCache, hub plumbing); prune deps; docs
---

# Remove browser/web UI from issuectl

## Description

## Decision

Remove **all browser/web-UI functionality** from issuectl. issuectl becomes a
pure AI-first CLI; the Trello-style web board and its HTTP surface go away.
(Product decision, 2026-08-10.)

## ⛔ Gate — do NOT implement yet (handled by hand)

Do not start the removal until the web functionality has been evaluated for the
**successor program**: a draft of that new program is being built in a separate
repo, and we first need to know to what extent these features can be carried
over there. When that draft is ready, this gate lifts and we do the removal.
There is **no dependency issue** tracking the gate — it is managed manually
(that is why this issue is `deferred`). Un-defer + start only after the go.

## Scope (what gets removed)

- `issuectl serve` — the local read-only web board command.
- The web server / HTTP layer and its `/api/*` endpoints (issue list, PATCH
  update paths, etc.).
- The kanban board frontend and all its assets.
- The file watcher / web-edit-sync machinery that only existed to keep the
  browser view live.
- Any config, schema fields, or docs that exist solely for the web board.

Keep everything CLI/domain: `issuectl-core`, all `cmd_*` CLI paths, the mutate
layer, schema, skills.

## Context — issues closed/rescoped alongside this decision (2026-08-10)

Closed `obsolete` (web-board enhancements, now moot): `almost-homely-decision`,
`fiercely-colossal-rabbits`, `fiercely-juicy-kettle`,
`genuinely-cloistered-current`, `intensely-teeny-ink`,
`needlessly-flimsy-scarecrow`, `needlessly-mysterious-volcano`,
`partially-nasty-sack`, `perfectly-white-linen`, `practically-truculent-music`,
`somewhat-flawless-letter`, `supremely-accurate-body`, `truly-somber-payment`.

Kept but rescoped **CLI-only** (their CLI value survives the web removal):
`issue-graph-view` (was massively-periodic-surprise — `issuectl graph`),
`epic-tree-view` (was needlessly-slippery-pan — `issuectl epic tree`).

## Acceptance Criteria

- [ ] Successor-program draft evaluated; migration extent of web features known.
- [ ] `issuectl serve` + web server / `/api` + kanban frontend removed.
- [ ] Web-only watcher / edit-sync removed.
- [ ] Web-only config/schema/docs removed; CLI paths untouched.
- [ ] Green gate passes (`cargo test`, `clippy`, `fmt --check`); skills synced.
