---
created: 2026-05-10
updated: 2026-05-10
type: improvement
status: wontfix
priority: normal
epic: exorbitantly-ill-apples
related: ['@amazingly-ready-pancake']
closed: 2026-05-10
---

# Doctor: outcome.notes_renamed reports pre-NN-rename dir name instead of canonical slug

## Description

Spin-off from /llm-review of @amazingly-ready-pancake.

## Problem

For the numbered-legacy + Notes input — both pre-flat (issues/open/3-foo-bar/item.md with `## Notes`) and already-flat (issues/3-foo-bar/) — `doctor::apply` records the pre-NN-rename directory name in `outcome.notes_renamed` while `outcome.legacy_dirs_migrated[].new_slug` records the canonical slug NN-rename produced. Two adjacent fields in the same JSON envelope reference the same physical issue under different names.

Identified by Gemini, GPT-5.5, and Anthropic in /llm-review of @amazingly-ready-pancake. Pre-existing skew — @amazingly-ready-pancake widened the input space (added the legacy-folder→flat path) but did not introduce the bug class.

## Why it matters

`--json --fix` is the scripting interface per AGENTS.md. Consumers correlating `notes_renamed` against `legacy_dirs_migrated[].new_slug` see two seemingly different issues and break.

## Fix options

(a) Move the second `rename_notes_to_comments` call after the NN-rename loop. NN-rename only touches frontmatter and `#NN`/path refs, never `## Notes`/`## Comments` headings — safe.
(b) Post-process `outcome.notes_renamed` at the end of `apply` via the `LegacyMigration::old_dir_name → new_slug` mapping built during NN-rename.

Per Gemini, the same skew may exist for `outcome.notes_conflicts_at_apply` and `outcome.status_reconciled`. Audit during fix.

## Acceptance criteria

- Test: numbered-legacy + Notes (issues/open/3-foo-bar/ with `## Notes`) → after one `--fix`, `outcome.notes_renamed == [canonical_slug]` (not `3-foo-bar`).
- Same for already-flat numbered-legacy.
- `outcome.notes_conflicts_at_apply` and `outcome.status_reconciled` audited and remapped if affected.

## Source

- history/review-notes-rename-after-migrate.md finding #1
- @amazingly-ready-pancake
