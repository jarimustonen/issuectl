---
created: 2026-08-14
updated: 2026-08-14
type: bug
status: open
priority: high
labels: [release, changelog]
---

# Changelog trailers never injected → trailer-driven changelog compiles empty

## Description

The changelog is trailer-driven: `issuectl changelog <range>` compiles release notes by walking git-log for `Refs-Issue:`/`Fixes-Issue:` trailers and grouping by issue type. But NOTHING injects those trailers — `git_trailers.rs` only PARSES them; there is no commit-msg hook, no `worktree-merge` trailer step, and CONTRIBUTING does not document the convention. Result: since v0.10.0, 1 of 63 commits carries a trailer, so `issuectl changelog v0.10.0..HEAD` yields a near-empty list that does NOT reflect the ~15 units of real unreleased work. This blocks a quality 0.11.0 release (AGENTS release bar = green gate PLUS a complete CHANGELOG [Unreleased]).

Discovered during the 2026-08-14 stint when the DAG drained and the release step found the [Unreleased] section empty and the trailer compile near-empty.

Fix options (pick during design):
1. Auto-inject `Fixes-Issue: <slug>` when a run/worktree closes an issue (orchestratectl `run merge` and/or `issuectl close --commit` could stamp the trailer into the landing commit), so the changelog compiles correctly with zero human discipline.
2. A commit-msg hook that maps a branch/issue context to a trailer.
3. Document the trailer convention in CONTRIBUTING and rely on committers (weakest — this round shows agents don't add them).

Until fixed, releases need a manually-curated CHANGELOG [Unreleased] section. Preferred long-term: option 1 (close/merge stamps the trailer).
