---
created: 2026-05-09
updated: 2026-05-09
type: bug
reporter: jari
status: in-progress
priority: normal
epic: exorbitantly-ill-apples
related: ['@remarkably-chivalrous-discovery']
labels: [web-edit-sync, canonical-hash]
---

# Add title to canonical_frontmatter_value (concurrency-control gap)

## Description

The flat-layout coherence rewrite of docs/design/web-edit-sync.md (issue @remarkably-chivalrous-discovery) confirmed via multi-LLM review that `Issue.title` is a frontmatter field but is omitted from `canonical_frontmatter_value` in src/canonical.rs:45+.

Effect: two writers concurrently changing `title` (one via `issuectl new`, the other via a hand-edit to item.md) can clobber each other without 409.

Spec is correct in the rewritten doc (§3.2 now includes `m.insert("title", ...)` in the pseudocode and explains why). Code needs to match.

Notes:
- This is a *breaking* version-token change. Every existing `version` will recompute differently. Bake into M1 with the `mutate.rs` refactor or roll out atomically.
- Update `canonical_hash_changes_when_*` test coverage to include title-mutation.
- No DTO change required: title remains immutable post-creation; the hash is only there to catch hand-edits and `new`-time clobbers.
