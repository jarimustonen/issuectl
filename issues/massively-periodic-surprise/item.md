---
created: 2026-05-06
updated: 2026-08-04
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, visualization, deferred]
---

# Issue graph view — multiple lenses on one tool

## Description

Single underlying graph engine (`issuectl graph`), multiple lenses
on top of it. The graph itself is built from frontmatter
relationships (`blocked_by`, `related`, `epic`); each lens
projects/filters/colors that graph for a different purpose.

## Lenses

### 1. Dependency graph (the original ask)

What blocks what. Renders `blocked_by` as directed edges,
`related` as dashed undirected edges, `epic` membership as
clusters. Output formats: `--format mermaid|dot|svg`. Mermaid is
paste-ready for markdown docs. Builds on `@uncommonly-cooing-badge`
(canonical `blocked_by`).

In the web board: clicking a blocked card opens a mini dependency
tree centred on that issue.

### 2. Worktree-planning view

Goal: answer "what can I work on in parallel right now?" — the
lens we actually used during the v0.5.0 push, where the question
"voiko nämä tehdä rinnakkain?" came up for almost every worktree
spawn.

Inputs beyond the dep edges:
- **Status filter** — only `open` / `in-progress` participate.
- **Conflict-pinta heuristic** — parse a `Source: …` line (or
  similar convention) from each `item.md` to estimate the file/
  module footprint. Issues whose footprints overlap are flagged
  as "likely conflict if run in parallel"; disjoint footprints
  cluster into "parallel-safe" sets.
- **Soft-order constraints** — pull free-text "land before X"
  / "after X" hints from epic bodies (e.g. the existing
  `@outright-homely-calendar` "land before drag-and-drop" line)
  and surface them as advisory edges.

Output: a coloured graph with each issue tagged *blocked* /
*ready-now* / *needs-rebase-after-X*, plus a top-level
"parallel-safe sets" summary so the user can pick N worktrees to
spawn.

### 3. Epic / milestone roll-up

Project the graph filtered to one epic. Useful for release
readiness: at a glance, what is left in v0.5.0, which pieces are
unblocked, what depends on what.

## Tool design

One CLI surface — `issuectl graph` — with `--lens` selecting the
projection (`deps` (default) / `worktree` / `epic <slug>`).
`--format` is orthogonal (mermaid/dot/svg/json). Web board
embeds the same tool; clicking a card surfaces the deps lens
centred on it. The graph data model is the source of truth; the
lenses are pure transforms.

## Conventions this requires

- Canonical `blocked_by` field (`@uncommonly-cooing-badge`).
- Convention for `Source:` lines so the worktree-planning lens
  has reliable footprint data. Many existing issues already use
  the pattern (`_Source: src/repo.rs, …_`). Formalising it +
  doctor validation comes via `@vastly-lyrical-police` /
  `@singularly-hulking-crown`.
- "Land before X" / "after X" hint syntax in epic bodies. Free
  text is fine for v1; can tighten later if real disagreement
  emerges.

## Out of scope

- Live editing of dep edges from the graph view (drag to add
  `blocked_by`). Could be a v0.7+ idea once the dep model is
  settled.
- Critical-path analysis. Not useful for a few-dozen-issue repo;
  reconsider only if multi-team scale appears.
