---
created: 2026-08-14
updated: 2026-08-14
type: bug
status: fixed
priority: normal
labels: [cli, dag]
closed: 2026-08-14
closed_by: agent-dag
---

# issuectl dag must not exclude in-progress from spawnable

## Description

## Comments

### 2026-08-14T03:42:13Z · @jari

Design correction (decided 2026-08-13 with @jari). `issuectl dag` currently excludes in-progress from `spawnable` (dag.rs:80 const IN_PROGRESS, dag.rs:466 let underway = i.status==IN_PROGRESS, dag.rs:470 && !underway). That is the WRONG model.

MODEL: in-progress ≠ 'being worked right now'. It means STARTED, not done. `issuectl dag` is consulted ONLY when nothing is actively running ('what's next?'). Under that invariant an in-progress issue is by definition one nobody is currently on → an unfinished, resumable candidate. So dag must SURFACE in-progress issues as spawnable — aggressively — because they are half-done and idle. Preventing two workers on the same issue is the CALLER's reservation/claim responsibility (dag.rs:52-55 TOCTOU note already says 'the caller must claim'), NOT dag's.

FIX: remove the `!underway` exclusion and the IN_PROGRESS const; in-progress heads report spawnable. Update the module docstring (dag.rs:44-50, which currently asserts 'an issue whose work is already underway must never read spawnable'). Add a test: an in-progress head is spawnable. Supersedes the closed dag-inprogress-schema-aware (that wanted schema-aware RENAMING of in-progress; we instead remove the underway check entirely — in-progress stays canonical).

## Resolution

### 2026-08-14T05:35:19Z · @agent-dag

Fixed on branch wt/01kzzatfz5: dag no longer excludes in-progress from spawnable. in-progress = started-not-done → resumable candidate that must surface; caller owns double-work prevention via --reservations. Removed !underway + IN_PROGRESS const, rewrote module+make_issue docstrings + AGENTS + CHANGELOG (Changed, supersedes unreleased !underway), flipped 3 in-progress tests to assert spawnable + added in_progress_head_still_reserved_by_lane. cargo test 1027 pass, clippy no new warnings, cargo fmt clean. Reviewed via /llm-review (4/4 consensus — mechanics confirmed correct; revert recommendation declined as it contradicts the decided ISO LINJAUS) + /assess-findings. Two follow-ups staged (not filed): per-issue slug reservation for unreservable unlaned in-progress issues; resumable:bool field.
