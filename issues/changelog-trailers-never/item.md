---
created: 2026-08-14
updated: 2026-08-15
type: bug
status: in-progress
priority: high
labels: [release, changelog]
lane: release
lane_seq: 10
collision: [crates/issuectl/src/main.rs]
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

## Comments

### 2026-08-15T12:16:58Z · @agent

DESIGN (option 1, issuectl side): add `issuectl close <slug> --stamp`. After the close mutation succeeds, issuectl amends the current HEAD commit's message to append `Fixes-Issue: @<slug>` — the EXACT format git_trailers::parse_trailers already accepts and report::changelog consumes. Byte-compat guaranteed by writing the trailer with git's own `git interpret-trailers --if-exists doNothing --trailer 'Fixes-Issue: @<slug>'` then `git commit --amend`, so it lands as a real trailer in the last paragraph and is idempotent (double-stamp = one trailer). Chose amend-HEAD over 'create a dedicated close-commit' because it stamps the REAL fix commit (best release-note fidelity) and matches the brief's 'amends a landing commit' wording. Opt-in flag (not default) since amend rewrites HEAD — unsafe as a silent default for a general tool; THIS repo gets zero-discipline by having its close/merge flow pass --stamp. Fail-SAFE guards (skip+warn, never fail the close): not a git repo / no commits / detached HEAD; in-progress rebase|cherry-pick|merge; HEAD is a merge commit; staged changes in the index. Requirement: run --stamp AFTER committing the fix (documented in help + skill template). Git logic in issuectl-core (git_trailers::stamp_fixes_trailer); cmd_close stays thin; mutate::close_issue stays fs-pure; --json echoes stamp outcome. Regression tests prove (a) exact-format trailer parses back and (b) report::changelog attributes the commit.
