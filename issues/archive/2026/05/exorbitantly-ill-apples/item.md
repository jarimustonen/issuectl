---
created: 2026-05-06
updated: 2026-05-10
type: epic
status: done
priority: high
owner: jari
labels: [release-v0.5.0]
closed: 2026-05-10
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
- [x] @slightly-finicky-heart — `issuectl doctor`: full validation suite +
  installable git hooks. Subsumes @amazingly-scattered-month. Landed
  `8afb683` + review fixes (`8568585`, `f572e8a`).
- [x] @peculiarly-political-interest — Agent-safe mutation CLI
  (`set` / `note` / `check` / `label` / `apply` + `--dry-run`).
  Landed `d5943fb` + review fixes (`e67503be`, `505c185`).
  Spin-off: @massively-regular-market (body ops in `apply`).
- [x] @outright-homely-calendar — `issuectl fmt` + optional YAML merge
  driver. Landed `230974d` + review fixes `49e7c0f`. Closed.
- [x] @vastly-lyrical-police — Declarative status transition rules +
  per-type body section linting. Landed `3f9afba`. Spin-offs:
  @deeply-wistful-beam (cache schema/transitions),
  @fully-economic-grade (update --type body scaffolding),
  @incredibly-mellow-owner (doctor single-pass scanner).
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
- [x] @markedly-terrific-angle — `.issuectl/AGENTS.md` agent policy file.
  Landed `cac12ca` + review fixes (`2786f32`, `fd7d1ff`).

### Concurrency / correctness (originally surfaced in M1 and M2 reviews)
- [x] @painfully-endurable-steel — Preserve unknown frontmatter keys in
  canonical hash. Landed `f20f443` + review fixes (`8845bc7`,
  `3d3adbd`, `a6cdd4b`).
- [x] @especially-stingy-powder — `mutate::new_issue` must publish
  before releasing flock. Landed `a6d6755` + review fixes `4f319d6`.

### Bugs / polish (small, dropped into v0.5.0 because they hurt daily use)
- [x] @peculiarly-truncated-title — `issuectl ls` drops the first
  character of the H1 title in CLI display. Fixed in `6549ce6`.
- [x] @astoundingly-harsh-nest — Typed `DoNewError` enum at
  `do_new_locked` boundary. Landed `c272404` + review fixes `bcdb9e9`.
  Spin-offs: @partially-ahead-button (extract domain code from main.rs),
  @relatively-entertaining-ticket (CLI golden-test harness).
- [x] @strikingly-abandoned-stranger — Two-thread seq-order regression
  tests landed and reverted (`47be957`, `5e6baef`) — kept the existing
  proxy-invariant test; harness deemed flaky. See issue body for
  rationale.
- [x] @tolerably-beautiful-war — `custom_fields` on `UpdateIssueRequest`.
  Landed `4f62aef` + review fixes `149393c`.
- [x] @partially-ahead-button — Extract `do_new_locked` + `NewArgs`
  out of `src/main.rs` into a domain module. Landed `6b99007` + `7ba0977`
  + review fixes (`ef1af2f`, `16cba25`). Spin-offs:
  @ridiculously-outgoing-brass (constants relocation + lib.rs split),
  @genuinely-magical-canvas (centralize custom-field validation).
- [x] @relatively-entertaining-ticket — CLI golden-test harness for
  `cmd_new` error output. Landed `41df83d` + review fixes `96483be`.
- [x] @ridiculously-outgoing-brass — `issuectl-core` workspace split +
  constants relocation. Landed `6d4419d`. Spin-offs:
  @greatly-flat-sleet (doctor apply-pipeline refactor),
  @quite-rigid-horses (derive lifecycle from schema).
- [x] @genuinely-magical-canvas — Centralize custom-field-key validation.
  Landed `1a93b86` + review fixes `86295dc`.
- [x] @incredibly-mellow-owner — `doctor` single-pass scanner. Landed
  `891e2e7` + review fixes `711087f`.
- [x] @deeply-wistful-beam — Server: cache schema + transitions config.
  Landed `4c7c440` + review fixes (`7d26181`, `588f640`, `6c3ec18`).
- [x] @fully-economic-grade — `issuectl update --type`: scaffold + reject
  on missing required sections. Landed `79b6242` + review fixes `86b3dd1`.
- [x] @massively-regular-market — `apply` transactional body ops. Landed
  `fea2072` + review fixes (`3de9c62`, `fc790bd`).
- [x] @remarkably-chivalrous-discovery — Coherent rewrite of
  `web-edit-sync.md`. Landed `f90d345` + review fixes (`d0141c7`,
  `39d4777`). Spin-off: @especially-unruly-crate (title in
  canonical_hash).
- [x] @especially-unruly-crate — Add `title` to
  `canonical_frontmatter_value`. Landed `81712f1` + review fixes
  `540a30b`.
- [x] @greatly-flat-sleet — Doctor: structural Findings + Actions +
  ApplyOutcome pipeline. Landed `a760a9b` + review fixes `e10b21b`.
  Spin-offs: @completely-hilarious-kitty (legacy_number_from_mapping
  data-loss bug), @slightly-hellish-airport (post-flat-layout
  critical-blockers re-check).
- [x] @quite-rigid-horses — Derive lifecycle status classification
  from schema/transitions. Landed `3280f7b` + review fixes `313aac6`.
- [x] @completely-hilarious-kitty — Doctor: fix `legacy_number_from_mapping`
  data-loss when `number` and `slug` coexist. Resolved by `f371b30`
  (superseded — closed via `4d4157e`).
- [x] @slightly-hellish-airport — Doctor: post-flat-layout migration
  re-checks `critical_blockers` before NN-rename. Landed `1d762c8` +
  review fixes `f371b30`. Spin-offs: @amazingly-ready-pancake
  (rename_notes ordering), @deeply-madly-thought (`blocked_by`
  ref-rewrite gap), @nearly-aware-chain (Err-discards-partial-outcome),
  @rather-abhorrent-edge (split blockers preflight + post-apply).

### Doctor follow-ups (open, last v0.5.0 batch)
- [ ] @amazingly-ready-pancake — Run `rename_notes_to_comments` AFTER
  flat-layout migration so freshly-lifted dirs get the one-shot
  Notes rename.
- [ ] @deeply-madly-thought — `rewrite_item_frontmatter` must rewrite
  `blocked_by: ["#NN"]` legacy refs alongside `epic` and `related`.
- [ ] @nearly-aware-chain — `execute_migrate_layout_plan` Err path
  must preserve partial `flat_layout_migrated` in the outcome.
- [ ] @rather-abhorrent-edge — Split `ApplyOutcome.blockers` into
  `preflight_blockers` + `post_apply_blockers`; update JSON envelope
  + skill templates to match.

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
