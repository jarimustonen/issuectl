---
created: 2026-07-23
updated: 2026-07-23
type: bug
reporter: jari
assignee: jari
status: open
priority: normal
---

# rename leaves the renamed issue's own `slug:` frontmatter field stale

_Source: crates/issuectl-core/src/repo.rs rename_issue_

## Description

## Summary

`issuectl rename <old> <new>` renames the directory and rewrites cross-references
(`epic:` / `related:` / `blocked_by:` and `@slug` body mentions), but it does
**not** update the renamed issue's own `slug:` frontmatter field. For any issue
that carries a `slug:` field, the result is `issues/<new>/item.md` still holding
`slug: <old>`. `doctor` does not flag the directory-vs-field mismatch.

## Why it matters (not purely latent)

Freshly `new`-created issues carry no `slug:` field, so the bug is invisible
there. But **`doctor --fix` stamps a `slug:` field onto every issue** during
migration. So the exact sequence a repo hits when adopting issuectl —
`doctor --fix` (legacy → flat) then renaming the auto-generated random slugs to
descriptive ones — leaves every renamed issue with a stale self `slug:`, silently.

## Repro

1. `issuectl doctor --fix` on a repo (stamps `slug:` on every item.md), or hand-add
   `slug: old-foo` to an issue at `issues/old-foo/item.md`.
2. `issuectl rename old-foo new-bar`
3. `grep '^slug:' issues/new-bar/item.md` → prints `slug: old-foo` (stale).
4. `issuectl doctor` → "Repository OK" (mismatch not detected).

Observed 61/61 renamed issues carried a stale self `slug:` after a real migration
(issuectl 0.6.4).

## Root cause

- `rename_issue` (`crates/issuectl-core/src/repo.rs:940`) rewrites references for
  every issue via `rewrite_frontmatter_refs` + `rewrite_body_refs`, then moves the
  directory.
- `rewrite_frontmatter_refs` (`crates/issuectl-core/src/repo.rs:1099`) only touches
  `epic`, `related`, `blocked_by`. It never rewrites the `slug` key. So when the
  loop processes the renamed issue itself (`slug == old`), its self `slug:` field
  is left untouched.

## Suggested fix

- In `rename_issue`, when handling the renamed issue's own file, also set its
  frontmatter `slug` field to `new` (only if present), so the field tracks the dir.
- Defense-in-depth: have `doctor` detect (and `--fix` repair) a `slug:` field that
  disagrees with the directory name — this class of drift is currently invisible.

## Environment

issuectl 0.6.4 (Homebrew). Found while migrating the `glasspad` repo from the
legacy numbered layout and renaming 61 auto-generated slugs to descriptive ones.
