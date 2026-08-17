---
created: 2026-05-07
updated: 2026-05-08
type: bug
status: duplicate
priority: normal
epic: exorbitantly-ill-apples
related: ['@painfully-endurable-steel']
closed: 2026-05-08
---

# canonical_hash drops unknown frontmatter fields

## Description

Pre-existing limitation, not introduced by awfully-faint-sound. Design web-edit-sync.md §3.2 mandates that unknown frontmatter fields participate in canonical_hash so external editors that add custom keys get conflict detection.\n\nThe parser's Frontmatter struct does not currently preserve unknowns. Concurrent updates to a custom field can be silently lost because the version token doesn't change.\n\nFix: extend parser::Frontmatter with BTreeMap<String, serde_yaml::Value> for unknowns, thread through Issue and canonical_frontmatter_value. Both the watcher (parse path) and mutate.rs (write path) consume the same canonical_hash, so the change is consistent by construction.\n\nSee history/review-flat-layout.md D4 (gpt-5.5, deepseek consensus; claude correctly classified as out-of-scope follow-up).
