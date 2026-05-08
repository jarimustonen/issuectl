---
created: 2026-05-08
updated: 2026-05-08
type: task
status: open
priority: normal
labels: [refactor, v0.6]
---

# Preserve unknown frontmatter fields on Issue (custom-field plumbing)

## Description

Currently src/context.rs::read_blocked_by re-reads item.md and re-parses YAML to extract blocked_by, because the typed Frontmatter struct doesn't carry unknown keys. This opens a TOCTOU window between repo::load_issues and the second read, and means every future custom field that needs to flow through to context/prompt bundles will repeat the same pattern. Fix: extend models::Issue to preserve unknown frontmatter fields (BTreeMap<String, serde_yaml::Value>), populate it in parser, and route blocked_by + future custom-field consumers through it. Drops the second file read in context.rs and lets the determinism contract hold under concurrent edits. Spun off from review of profoundly-domineering-wound.
