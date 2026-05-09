---
created: 2026-05-09
updated: 2026-05-09
type: task
reporter: jari
assignee: jari
status: in-progress
priority: normal
epic: hugely-exciting-spiders
related: ['@excessively-beneficial-owner', '@profoundly-domineering-wound', '@markedly-terrific-angle']
labels: [kanban, web-ui, design-spike, v0.6.0-candidate]
commits:
- hash: 6dee67c
  summary: 'docs(design): web control surface design spike'
- hash: 31c933f
  summary: 'docs(design): drop content-hash trust gate; auto-trust manifests'
- hash: '77418e1'
  summary: 'docs(design): revise web control surface for workmux + integrate review'
- hash: b6ba2f5
  summary: 'docs(design): integrate round-2 review findings (R1-R19)'
---

# Design web control surface for triggering issue work from kanban

_Source: docs/design/web-control-surface.md_

## Description

Design spike: how the read-only kanban becomes a control surface for triggering agent work on issues.

## Outcome

Design note shipped at `docs/design/web-control-surface.md` covering:

- 8 mechanism candidates rated against a security/UX/portability rubric
- Recommended v0.6.0 architecture: action manifest + filesystem run queue + embedded runner, with three action kinds (`workmux`, `workmux-send`, `exec`)
- Hard dependency on `workmux` for terminal-backed actions
- No content-hash trust gate (manifest treated as authored code, same trust model as Makefile / npm scripts / git hooks)
- `manifest_digest` binding on POST as stale-preview defence (not a trust gate)

## Process

Two multi-LLM review rounds (4 + 3 reviewers via `/llm-review`) plus formal `/assess-findings` passes integrated into the design. Round 1 produced ~25 findings; round 2 added 19 (R1–R19) focused on the workmux-integration and trust-removal sections specifically.

## Review traces (gitignored, local only)

- `history/review-web-control-surface-raw.md` — round 1 raw
- `history/review-web-control-surface.md` — round 1 assessed
- `history/review-web-control-surface-r2-raw.md` — round 2 raw
- `history/review-web-control-surface-r2.md` — round 2 assessed (with empirical verification per `/assess-findings` Phase 2a)

## Supersedes (in scope, not closure)

@excessively-beneficial-owner — narrow precursor ("start implementation" button research task). Stays open for the implementation ticket whoever picks it up first will write.

## Spin-offs to file when implementation begins

See §12 of the design note. Top items:

1. Action manifest + run queue + embedded runner (the atomic primitive)
2. `kind: workmux` + `kind: workmux-send` action kinds — supersedes @excessively-beneficial-owner
3. CLI parity (`issuectl run` + `issuectl runs ...`)
4. Filesystem detection + `--unsafe-shared-fs`
5. Visibility surface (terminal banner + UI manifest-changed banner + modal argv preview)
6. Worktree/run GC story (was F21 SPIN-OFF in round 1)
7. External `issuectl runner` (v0.7.0+ deferred)

## Status

Design phase complete; commits 6dee67c, 31c933f, 77418e1, b6ba2f5 on branch `web-trigger-design-spike`. Awaiting `/worktree-merge` to land on main.
