---
created: 2026-05-06
updated: 2026-08-10
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, visualization, deferred]
---

# Issue graph view — multiple lenses on one tool (CLI)

> **Rescoped 2026-08-10 (CLI-only).** The browser/web UI is being removed from
> issuectl (see `@remove-web-ui`), so the former web-board embedding of this
> graph (mini dependency tree on card click) is dropped. What remains — and is
> the whole point — is the CLI `issuectl graph` engine and its lenses. Note that
> lens 2 (worktree-planning) partly shipped already as `issuectl dag`
> (`@dag-scheduling-view`); a future implementer should build on / reconcile
> with that rather than duplicate it.

## Description

Single underlying graph engine (`issuectl graph`), multiple lenses on top of it.
The graph itself is built from frontmatter relationships (`blocked_by`,
`related`, `epic`); each lens projects/filters/colors that graph for a different
purpose. Output is text/file (mermaid/dot/svg/json) — no server.

## Lenses

### 1. Dependency graph (the original ask)

What blocks what. Renders `blocked_by` as directed edges, `related` as dashed
undirected edges, `epic` membership as clusters. Output formats:
`--format mermaid|dot|svg`. Mermaid is paste-ready for markdown docs. Builds on
canonical `blocked_by` (already shipped).

### 2. Worktree-planning view

Goal: answer "what can I work on in parallel right now?" — the lens used during
the v0.5.0 push, where "voiko nämä tehdä rinnakkain?" came up for almost every
worktree spawn. **Partly shipped as `issuectl dag`** (lane/collision +
head-of-line/spawnability on read); reconcile with it before building more.

Inputs beyond the dep edges:
- **Status filter** — only `open` / `in-progress` participate.
- **Conflict-pinta heuristic** — parse a `Source: …` line (or similar
  convention) from each `item.md` to estimate the file/module footprint. Issues
  whose footprints overlap are flagged as "likely conflict if run in parallel";
  disjoint footprints cluster into "parallel-safe" sets. (Cf. the `collision:`
  field the DAG view already added.)
- **Soft-order constraints** — pull free-text "land before X" / "after X" hints
  and surface them as advisory edges.

Output: a coloured graph with each issue tagged *blocked* / *ready-now* /
*needs-rebase-after-X*, plus a top-level "parallel-safe sets" summary.

### 3. Epic / milestone roll-up

Project the graph filtered to one epic. Useful for release readiness: what is
left, which pieces are unblocked, what depends on what.

## Tool design

One CLI surface — `issuectl graph` — with `--lens` selecting the projection
(`deps` (default) / `worktree` / `epic <slug>`). `--format` is orthogonal
(mermaid/dot/svg/json). The graph data model is the source of truth; the lenses
are pure transforms.

## Conventions this requires

- Canonical `blocked_by` field (shipped).
- Convention for `Source:` lines so the worktree-planning lens has reliable
  footprint data. Many existing issues already use `_Source: src/repo.rs, …_`.
- "Land before X" / "after X" hint syntax. Free text is fine for v1.

## Out of scope

- Live editing of dep edges (drag to add `blocked_by`) — was a web idea, now moot.
- Critical-path analysis. Not useful at a few-dozen-issue repo scale.
