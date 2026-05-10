---
created: 2026-05-10
updated: 2026-05-10
type: chore
status: wontfix
priority: normal
epic: exorbitantly-ill-apples
related: ['@amazingly-ready-pancake', '@dreadfully-combative-stomach']
closed: 2026-05-10
---

# Doctor: walk all folders in populate_notes_migration so notes-rename can run once before flat-layout migration

## Description

Spin-off from /llm-review of @amazingly-ready-pancake (Anthropic finding).

## Problem

`populate_notes_migration` is "flat-only" by design — it skips any `s.folder != "flat"`. The fix in @amazingly-ready-pancake paid for this with a dual `rename_notes_to_comments` call inside `apply` (one before phase 5, one after the post-migration fresh re-scan).

## Cleaner architecture

Make `populate_notes_migration` walk `open`/`closed` folders too. Then a single rename pass before flat-layout migration suffices — the `fs::rename` in phase 5 carries the rewritten body along automatically.

Eliminates:
- The dual-call pattern in `apply`.
- The `actions.notes_to_rename = fresh.notes_to_rename` mid-pipeline mutation that Anthropic and OpenAI both flagged as a smell.
- The widening of @dreadfully-combative-stomach's slug-identity skew surface (the post-migration call is what extended the skew to legacy-folder issues).

Also aligns with the symmetric "no flat-only lints" principle Anthropic raised: any other lint sharing the same flat-only assumption may be invisible to pre-migration scan in the same way.

## Risk / scope

Audit other lints in `scan()` / `populate_*` for hidden `folder == "flat"` assumptions. Building the right path for legacy-folder issues in the rename writer needs care (currently `issues.join(slug)` — must instead use `s.dir_path`). Cross-cutting refactor — needs its own design before implementation.

## Source

- history/review-notes-rename-after-migrate.md finding #6
- @amazingly-ready-pancake
- @dreadfully-combative-stomach
