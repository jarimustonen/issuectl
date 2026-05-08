---
created: 2026-05-07
updated: 2026-05-08
type: bug
status: in-progress
priority: normal
epic: exorbitantly-ill-apples
labels: [release-v0.5.0]
commits:
- hash: 6390ced
  summary: 'test(mutate): regression test for new_issue publish-before-flock-release'
- hash: 9c9d026
  summary: 'test(mutate): apply review fixes (WouldBlock match, 4 mutation paths, error-path)'
---

# mutate::new_issue publishes after releasing flock

## Description

M2 review F12 (review-m2-implementation.md). `mutate::new_issue` calls
`crate::do_new`, which acquires the repo `flock`, writes the new issue
file, and drops the lock before returning. `new_issue` then re-parses
the file, computes the canonical hash, and calls `hub.publish` —
**outside any lock**.

This violates the design contract in `docs/design/web-edit-sync.md`
§3.1 step 8 ("publish before releasing flock") and the global
seq-ordering invariant the rest of the M0/M1/M2 protocol depends on.
A concurrent writer (CLI, agent, another HTTP request) can modify the
file between `do_new`'s write and `new_issue`'s re-parse, in which
case the published `IssueUpserted` event carries a version that does
not correspond to the mutation it claims to describe.

Practically: creation races usually generate distinct slugs, so
double-409s are unlikely. The observable failure mode is event
reorder relative to a fast-following PATCH on the same slug, which
breaks dedupe-by-version on the client.

## Fix sketch

Two reasonable options:

1. Move the `do_new` body into `mutate.rs` so a single \`WriteLock\`
   spans \`create → parse → hash → publish → release\`. \`do_new\` becomes
   a thin wrapper around the new \`mutate\` entry point.
2. Refactor \`do_new\` to return its \`WriteLock\` guard alongside the
   outcome, so the caller can hold the lock across the publish step
   and drop it explicitly afterwards.

Option 1 is cleaner; option 2 minimises diff but smears the lock
contract across two call sites.

## Reviewers who flagged this

- DeepSeek-v4-pro (independent + cross-review)
- GPT-5.5 (independent + cross-review)
- Claude-Opus-4.7 (cross-review, with severity caveat)

All agreed the fix belongs in v0.5.0 but not as part of M2; it's an
M1 carry-over.

## Definition of done

- \`mutate::new_issue\` (or its replacement) holds the repo \`flock\`
  through the publish call.
- A regression test demonstrates the publish-before-release order
  for create, mirroring the existing M1 update tests.
- 173+ tests pass.
