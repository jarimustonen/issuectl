---
created: 2026-08-17
updated: 2026-08-17
type: chore
status: open
priority: normal
related: ['@cli-verb-surface']
lane: verb-surface
lane_seq: 10
blocked_by: ['@cli-verb-surface']
---

# Deprecate issuectl triage and the inbox landing zone

## Description

## Problem

`issuectl triage` + the `issues/inbox/<slug>/` drafts landing zone is a second,
mostly-unused reception mechanism running in parallel with the standard intake flow
(`intake file` / `intake queue` / `intake accept|defer|reject`).

Evidence (2026-08-17 assessment):

- This repo has no `issues/inbox/` at all; reception happens exclusively through
  `intake file`.
- The only producer of inbox drafts is `scan-todos --create-inbox`.
- The skill templates never mention inbox or `triage` — consumer agents are taught
  the intake flow only.
- `docs/design/intake-flow.md` already flags the seam itself: OD-1 (reception layout:
  flat `status: untriaged` vs `inbox/` draft) and OD-8 (the `triage` command name
  colliding with the intake concept).
- Blast radius of the mechanism in core: `repo.rs` layout discovery knows the inbox
  zone (`INBOX_DIR`, `push_inbox_issue`), `do_new` carries an `inbox: bool` option
  (all programmatic callers pass `false`), `cmd_triage` promotes.

## Proposal

Deprecate `issuectl triage` and the inbox landing zone; route the one remaining
producer through the standard flow:

1. `scan-todos --create-inbox` files via `intake file` (provenance `scan-todos`)
   instead of writing `issues/inbox/` drafts. The flag becomes `--file` (or similar);
   the old name stays as a deprecated alias for a window.
2. `triage` prints a deprecation notice pointing at `intake queue` / `intake accept`,
   then is removed after the window.
3. `repo.rs` inbox discovery + `do_new`'s `inbox` option are removed once nothing
   produces inbox drafts. `doctor` gains a one-shot migration: an existing
   `issues/inbox/<slug>/` draft is promoted (exactly what `triage` did) so no repo is
   stranded.

## Blocked by

The verb-surface ADR (@cli-verb-surface) should ratify this as part of the overall
surface decision — this issue is the worked example and first implementation slice.

## Acceptance

- One reception pipeline: every new item enters through `create` or `intake file`.
- `triage` and inbox-specific code paths removed (after the deprecation window), with
  the doctor migration covering stranded drafts.
- Skill templates untouched or updated in the same commit if any documented surface
  changes (AGENTS.md critical rule).

## Comments

### 2026-08-17T08:49:04Z · @agent-decision

ADR 0004 ratified this deprecation direction. Scope is clarified: deprecation waits for @intake-queue-legacy-mismatch, scan-todos receives an explicit intake-filing flag rather than ambiguous --file, and doctor --fix migrates stranded inbox drafts throughout the transition.
