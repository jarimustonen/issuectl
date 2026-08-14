---
created: 2026-08-14
updated: 2026-08-14
type: bug
status: open
priority: normal
labels: [cli, dag]
---

# issuectl dag must not exclude in-progress from spawnable

## Description

## Comments

### 2026-08-14T03:42:13Z · @jari

Design correction (decided 2026-08-13 with @jari). `issuectl dag` currently excludes in-progress from `spawnable` (dag.rs:80 const IN_PROGRESS, dag.rs:466 let underway = i.status==IN_PROGRESS, dag.rs:470 && !underway). That is the WRONG model.

MODEL: in-progress ≠ 'being worked right now'. It means STARTED, not done. `issuectl dag` is consulted ONLY when nothing is actively running ('what's next?'). Under that invariant an in-progress issue is by definition one nobody is currently on → an unfinished, resumable candidate. So dag must SURFACE in-progress issues as spawnable — aggressively — because they are half-done and idle. Preventing two workers on the same issue is the CALLER's reservation/claim responsibility (dag.rs:52-55 TOCTOU note already says 'the caller must claim'), NOT dag's.

FIX: remove the `!underway` exclusion and the IN_PROGRESS const; in-progress heads report spawnable. Update the module docstring (dag.rs:44-50, which currently asserts 'an issue whose work is already underway must never read spawnable'). Add a test: an in-progress head is spawnable. Supersedes the closed dag-inprogress-schema-aware (that wanted schema-aware RENAMING of in-progress; we instead remove the underway check entirely — in-progress stays canonical).
