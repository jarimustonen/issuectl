---
created: 2026-05-06
updated: 2026-05-06
type: epic
status: open
priority: normal
owner: jari
labels: [backlog, release-v0.6.0]
---

# v0.6.0: Backlog candidates (rank when v0.6.0 starts)

## Goal

Candidate pool for v0.6.0. Holds (a) brainstorm ideas that did not make
the v0.5.0 top-10, and (b) v0.5.0 items deferred during re-scoping
because they were not foundational to the writable-board release. Each
child is a candidate, not a commitment.

When v0.6.0 planning starts:

1. Re-read the list with what v0.5.0 actually shipped in mind.
2. Drop anything subsumed or obsoleted by v0.5.0 work.
3. Rank the rest by value-to-cost.
4. Decide a cut line; move the tail to v0.7.0 or close as `wontfix`.

## Issues (28 candidates, grouped by theme)

### Workflow / planning
- [ ] @uncommonly-cooing-badge — Dependency tracking: canonical `blocked_by` + cycle detection + dependency-aware queries
- [ ] @seriously-wrathful-knife — Markdown DoD validation: parse acceptance criteria + block `done` until satisfied
- [ ] @starkly-jaded-baby — Recurring / scheduled issues (cron-driven, materialize per occurrence)
- [ ] @nearly-strong-canvas — Cycles / iterations (Linear-style)
- [ ] @painfully-stingy-nail — Lightweight estimates + workload reports
- [ ] @partially-nasty-sack — WIP-limit warnings per kanban column
- [ ] @altogether-jaded-feast — Reviewer field + review_status

### Validation / maintenance
- [ ] @ridiculously-decisive-hen — Stale issue detector + auto-archive
- [ ] @distinctly-melodic-balloon — Duplicate detection (heuristic, local)
- [ ] @very-powerful-school — `issuectl rename` with reference updates

### CLI / agent ergonomics
- [ ] @fairly-smart-building — Bulk operations (`issuectl bulk '<query>' --add-label ...`)
- [ ] @remarkably-juvenile-memory — `issuectl open <slug>` editor integration
- [ ] @wildly-common-bushes — QoL bundle: triage inbox + fuzzy picker + slug prefix matching + shell completions + scan-todos
- [ ] @fiercely-mature-cattle — Schema-driven agent instructions in context bundle
- [ ] @excessively-beneficial-owner — Investigate Claude Code launch button (research task)

### Git-native / reporting
- [ ] @strikingly-absorbing-cows — Git-native commit linking (trailers + `sync-commits` + branch-name detection)
- [ ] @considerably-wide-mass — Git-derived activity / timeline / changelog + lightweight metrics

### Kanban UX
- [ ] @truly-somber-payment — Sort kanban columns by priority
- [ ] @almost-homely-decision — Per-user kanban view config (last state)
- [ ] @fiercely-juicy-kettle — Copy-to-clipboard buttons on cards
- [ ] @genuinely-cloistered-current — Multiple named kanban boards
- [ ] @needlessly-flimsy-scarecrow — Customizable card fields + color coding
- [ ] @somewhat-flawless-letter — Uncommitted-state indicator on cards

### Visualization
- [ ] @massively-periodic-surprise — Dependency graph (Mermaid / SVG / web)
- [ ] @needlessly-slippery-pan — Epic tree view (CLI + web)

### Content / interop
- [ ] @terrifically-minor-quiver — Issue-local attachments + fixtures dirs
- [ ] @amazingly-certain-competition — Import / export (GitHub, JSON, CSV, markdown)

### Discuss / maybe-don't-build
- [ ] @somewhat-heady-zephyr — Per-issue `events.jsonl` log (only if v0.5.0 git-derived activity proves insufficient)

## Notes

- The v0.5.0 → v0.6.0 deferrals were made because v0.5.0 was overscoped;
  these items are good ideas but not foundational to "writable agent-safe
  board". Many will be cheaper or better-shaped after v0.5.0 lands
  (e.g. dependency tracking benefits from the doctor + query + schema
  layers shipping first).
- `@somewhat-heady-zephyr` is explicitly tagged `discuss` — only build if
  v0.5.0's git-derived metrics turn out to be unreliable.
- Several candidates explicitly build on v0.5.0 issues; their shape and
  feasibility depend on v0.5.0 outcomes — re-evaluate then.
- Brainstorm synthesis: `history/plan-feature-brainstorm.md` (gitignored).
