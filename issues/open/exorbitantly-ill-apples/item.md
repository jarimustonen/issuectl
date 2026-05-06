---
created: 2026-05-06
updated: 2026-05-06
type: epic
owner: jari
status: open
priority: high
labels: [release-v0.5.0]
---

# v0.5.0: Writable, agent-safe kanban board

## Goal

Ship the writable web kanban with bidirectional file ↔ web sync, on top of
foundational tooling that makes the file-based model durable when humans
and AI agents both edit it. After v0.5.0:

- Every web action is reachable from the CLI; agents and humans share the
  same surface and never need to touch raw YAML.
- The repo is self-validating (`doctor` + git hooks); broken files surface
  loudly, not silently.
- The format is stable enough (`fmt` + merge driver) to survive concurrent
  edits via web, CLI, `$EDITOR`, and `git pull`.
- Issues are discoverable via a single shared query language (CLI + web).
- Agents can pull deterministic context bundles and obey a committed
  agent policy (`.issuectl/AGENTS.md`).

UI polish, dependencies, reporting, commit-trailer automation, and
quality-of-life improvements are deferred to v0.6.0.

## Scope (11 issues)

### M0 — architectural prerequisite (must land first)
- [ ] @awfully-faint-sound — Migrate to flat layout `issues/<slug>/item.md`
  (status only in YAML). Eliminates §3.4 of the web-edit-sync design.

### M1 — web edit/sync
- [ ] (worktree `web-edit-sync-design`) — Implement bidirectional web ↔ file
  sync per `docs/design/web-edit-sync.md`. M1 user-facing surface = drag-and-drop:
- [ ] @needlessly-fluffy-decision — Drag-and-drop write-back on the kanban.
- [ ] (worktree) @amazingly-scattered-month — Startup reconciliation
  (subsumed by @slightly-finicky-heart; link when worktree merges).

### M2 — agent-safe foundation
- [ ] @slightly-finicky-heart — `issuectl doctor`: full validation suite +
  installable git hooks. Subsumes @amazingly-scattered-month.
- [ ] @peculiarly-political-interest — Agent-safe mutation CLI
  (`set` / `note` / `check` / `label` / `apply` + `--dry-run`).
- [ ] @outright-homely-calendar — `issuectl fmt` + optional YAML merge
  driver. Land before drag-and-drop ships.
- [ ] @vastly-lyrical-police — Declarative status transition rules +
  per-type body section linting. Small extension of doctor.
- [ ] @overly-dreary-yak — Standardized markdown body sections (comments,
  decisions, agent runs, reopen notes). Defines the conventions the
  mutation CLI writes against.

### M3 — discoverability & agent integration
- [ ] @unusually-elegant-rule — Shared query engine (CLI + web + automation)
  with `--json` and full-text search.
- [ ] @profoundly-domineering-wound — Agent context bundle
  (`issuectl context <slug>`) + repo-local prompt templates.
- [ ] @singularly-hulking-crown — Issues schema file (required + optional
  fields). Powers doctor validation and schema-aware tooling.
- [ ] @markedly-terrific-angle — `.issuectl/AGENTS.md` agent policy file.
  Agents read it automatically; small change, huge leverage now that
  M2 introduces a new CLI surface.

## Phases

1. **M0 flat layout** — must merge first; simplifies M1.
2. **M1 web edit/sync** — implementation per the worktree design doc.
3. **M2 agent-safe foundation** — doctor, mutation CLI, fmt + merge driver,
   transition rules, body conventions.
4. **M3 query + context + schema + AGENTS.md** — substrates that compound.

## Notes

- Brainstorm synthesis: `history/plan-feature-brainstorm.md` (gitignored).
- Architectural decisions established by 3-LLM panel
  (gemini-3.1-pro-preview, gpt-5.5, deepseek-v4-pro), unanimous on each:
  - Status only in YAML, never in path.
  - Canonical `blocked_by` only; derive `blocks` at runtime (deferred to v0.6.0).
  - Recurring issues materialize a new file per occurrence (v0.6.0).
  - No SQLite cache for v1; if added, gitignored.
  - Agent artefacts: stdout default; gitignored if written; only durable
    `plan.md` / `handoff.md` committed beside the issue.
- The web-edit-sync worktree spin-offs (@amazingly-scattered-month,
  @supremely-accurate-body) live on branch `web-edit-sync-design` and are
  not yet linked to this epic. Link with `--epic` after the worktree
  merges, or amend the worktree branch.
- The v0.6.0 candidate pool (@hugely-exciting-spiders) holds the
  deferred items: dependencies, DoD validation, commit trailers, activity
  reports, QoL bundle, kanban UX polish, multi-board, Claude-launch
  investigation. Re-rank when v0.6.0 starts.
