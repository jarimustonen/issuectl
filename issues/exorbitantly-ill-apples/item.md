---
created: 2026-05-06
updated: 2026-05-07
type: epic
status: in-progress
priority: high
owner: jari
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
- [x] @awfully-faint-sound — Migrate to flat layout `issues/<slug>/item.md`
  (status only in YAML). Landed in `2809ded` + review fixes `501a1bd`;
  layout migration folded into `doctor --fix` (`ea8250a`). Closed.

### M1 — web edit/sync

The web-edit-sync design doc (`docs/design/web-edit-sync.md`) phases the
work internally as design-M0 → design-M3. Tracking those sub-phases here
to keep the epic in sync with the worktree:

- [x] **design-M0: live read-side updates** (worktree `web-edit-sync-design`,
  commits `ed9094d` + cluster fixes `2806f3f` … `cf632b0`).
  EventHub (parking_lot mutex covering seq+ring), notify-debouncer-full
  watcher with consecutive-failure backoff and notify-error
  classification, `/events` SSE with race-free subscribe-since handoff,
  Last-Event-ID + omit-id-on-synthetic + scan-stream-on-lagged
  semantics, `Arc<BoardEvent>` ring/broadcast for cheap M1 critical
  section, `canonical_hash` shared module so M0 watcher and M1
  `mutate.rs` produce identical version strings. 25 fixes from
  `/llm-review` + `/assess-findings` (report:
  `history/review-m0-implementation.md`, gitignored). 136/136 tests,
  end-to-end SSE smoke verified.
- [x] **design-M1: writes** — `28e15bf` + review fixes `b094cdb`.
  `mutate.rs` shared by CLI and server, flock on `.issuectl/write.lock`,
  PATCH/PUT/POST routes, CSRF + Host-header validation,
  `expected_version` optimistic concurrency, CLI `--expected-version`
  requirement on `--json`.
- [x] **design-M2: body editor** — `0b3b88d` + review fixes `832c363`.
  `PUT /body`, textarea + preview, localStorage draft, conflict UX.
- [x] **design-M3: robustness** — `b1a4910` + review fixes `565e7ac`.
  `--watch-poll-ms`, `Degraded` banner, three-way merge UI.
- [x] @needlessly-fluffy-decision — Drag-and-drop write-back on the
  kanban. Landed `32802d0` + review rounds `88dfb85` and `1ddd09b`.
  Closed.
- [ ] @amazingly-scattered-month — Startup reconciliation
  (subsumed by @slightly-finicky-heart; close when M2 lands).

### M2 — agent-safe foundation
- [ ] @slightly-finicky-heart — `issuectl doctor`: full validation suite +
  installable git hooks. Subsumes @amazingly-scattered-month.
- [ ] @peculiarly-political-interest — Agent-safe mutation CLI
  (`set` / `note` / `check` / `label` / `apply` + `--dry-run`).
- [x] @outright-homely-calendar — `issuectl fmt` + optional YAML merge
  driver. Landed `230974d` + review fixes `49e7c0f`. Closed.
- [ ] @vastly-lyrical-police — Declarative status transition rules +
  per-type body section linting. Small extension of doctor.
- [x] @overly-dreary-yak — Standardized markdown body sections (comments,
  decisions, agent runs, reopen notes). Landed `39cabc5` + review fixes
  (`6f4485f`, `122bec7`, `010bc3c`, `73c1a66`). Spin-offs:
  @virtually-dull-regret (note --stdin), @totally-placid-push
  (parse_section diagnostics).

### M3 — discoverability & agent integration
- [x] @unusually-elegant-rule — Shared query engine (CLI + web + automation)
  with `--json` and full-text search. Landed `5af9f94` + review fixes
  (`a8a643c`, `5ae510b`).
- [x] @profoundly-domineering-wound — Agent context bundle
  (`issuectl context <slug>`) + repo-local prompt templates. Landed
  `a1fe3cb` + review fixes `d299672`.
- [x] @singularly-hulking-crown — Issues schema file (required + optional
  fields). Landed `62bcf00`.
- [ ] @markedly-terrific-angle — `.issuectl/AGENTS.md` agent policy file.
  Agents read it automatically; small change, huge leverage now that
  M2 introduces a new CLI surface.

### Bugs / polish (small, dropped into v0.5.0 because they hurt daily use)
- [x] @peculiarly-truncated-title — `issuectl ls` drops the first
  character of the H1 title in CLI display. Fixed in `6549ce6`.
- [ ] @astoundingly-harsh-nest — `do_new_locked` returns `anyhow::Error`
  and `mutate::new_issue` reverse-engineers the typed `MutateError`
  variant by string-matching the formatted message. Brittle; fix by
  returning a typed enum. Spin-off from @especially-stingy-powder
  /llm-review.
- [ ] @strikingly-abandoned-stranger — Add a two-thread regression test
  that exercises the actual seq-inversion failure mode the C3 fix
  prevents (the existing `new_issue_publishes_before_releasing_flock`
  test only covers the proxy invariant in a single-threaded setup).
  Spin-off from @especially-stingy-powder /llm-review.

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
  @supremely-accurate-body) come in via the design-M0 merge. Link with
  `--epic` once that worktree lands. @amazingly-scattered-month
  remains a spin-off; @supremely-accurate-body is M2-conditional.
- The v0.6.0 candidate pool (@hugely-exciting-spiders) holds the
  deferred items: dependencies, DoD validation, commit trailers, activity
  reports, QoL bundle, kanban UX polish, multi-board, Claude-launch
  investigation. Re-rank when v0.6.0 starts.
