---
created: 2026-05-06
updated: 2026-05-08
type: task
status: in-progress
priority: high
reporter: jari
epic: exorbitantly-ill-apples
---

# Preserve unknown frontmatter fields in canonical hash

## Description

M1 review surfaced a real concurrency-safety gap: today's Frontmatter struct drops user-added YAML keys at parse time, so canonical_hash() does not include them. An issue with custom keys (triage:, reviewer:, etc.) can be silently overwritten by a writer that doesn't touch the custom fields, because the version hash matches.

Design doc §3.2 explicitly requires unknown keys to participate in the hash. M0 documented this as a known limitation deferred to M1; M1 left it.

Scope (target: 0.5.0):
- Add unknown: BTreeMap<String, serde_yaml::Value> to Frontmatter (parser preserves)
- Add unknown to Issue (or a wrapper) so canonical_hash can read it
- Project unknown keys (sorted) into canonical_frontmatter_value
- Round-trip via write_item (serialize unknown keys back)
- Tests: concurrent writes targeting different keys both succeed; concurrent writes targeting the same custom key produce 409

This is non-trivial because Issue is part of the public JSON API for the web board.
