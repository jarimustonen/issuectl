---
created: 2026-08-02
updated: 2026-08-04
type: bug
reporter: jari
status: fixed
priority: normal
closed: 2026-08-04
---

# Refs-Issue hint false-fires when trailer supplied via git commit -F/stdin

_Source: commit-hook_

## Description

_Source: commit hook / Refs-Issue linting_

## Observed

When committing on a branch whose name matches an issue slug, `issuectl` prints:

```
issuectl: branch matches @tw-view-preview; consider adding `Refs-Issue: @tw-view-preview` to your commit message (or run `issuectl sync-commits`).
```

This fired on **both** commits of a session even though **both commit messages already contained** a `Refs-Issue: @tw-view-preview` trailer. The messages were supplied via `git commit -F -` (heredoc on stdin).

## Expected

The hint should be **suppressed when the commit message already contains a matching `Refs-Issue:`/`Fixes-Issue:` trailer** for the branch's issue. Firing when the trailer is present is a false positive and trains users to ignore the hint.

## Likely cause / notes

The check appears not to see the trailer when the message arrives via `-F -` / stdin (possibly it inspects a staged/`COMMIT_EDITMSG` path or the `-m` args only, not the `-F` payload). Worth confirming which message source the hook reads and making it read the final message that git will actually use.

## Impact

Cosmetic only — does not block or alter the commit. Low priority, but it's a repeated papercut in the normal `Refs-Issue`-trailer workflow.
