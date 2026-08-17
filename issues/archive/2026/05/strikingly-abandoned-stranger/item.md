---
created: 2026-05-08
updated: 2026-05-09
type: task
reporter: jari
status: done
priority: normal
epic: exorbitantly-ill-apples
commits:
- hash: fe6761c
  summary: add two-thread seq-order regression tests
- hash: b423e99
  summary: revert two-thread tests after /llm-review found structural flaw
closed: 2026-05-09
---

# Two-thread regression test for seq order under concurrent mutations

_Source: src/mutate.rs tests_

## Description

The new_issue_publishes_before_releasing_flock test asserts the proxy invariant ('flock held when publish runs') in a single-threaded test with no other publishers. The actual production failure mode the C3 fix prevents — a fast PATCH landing seq=N+1 before a just-released-lock POST publishes seq=N (UI flicker on kanban: card briefly flips to old state then catches up) — is not exercised at all. The current test is necessary but not sufficient. Build a two-thread test that uses a barrier to choreograph the second mutation to start the moment the first releases its lock, runs many iterations for flake-resistance, and asserts published seq order matches on-disk write order. Decide which mutation pairs to cover (POST+PATCH at minimum; PATCH+PATCH on the same slug is the original incredibly-real-hour-style scenario).
