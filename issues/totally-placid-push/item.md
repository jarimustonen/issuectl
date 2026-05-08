---
created: 2026-05-08
updated: 2026-05-08
type: feature
reporter: jari
status: open
priority: normal
epic: exorbitantly-ill-apples
labels: [agent-friendly, v0.6.0-candidate]
---

# body_sections::parse_section: richer return type with diagnostics

## Description

Spin-off from second-round review of @overly-dreary-yak (O7). Currently 'parse_section' returns 'Vec<Block>' which collapses several distinct outcomes — missing section, empty section, all-malformed-headings, swallowed-by-unclosed-fence, duplicate sections present — into 'empty vec'. Sister tickets ('decide', 'agent-run') will infer wrong things from this. Switch to 'Result<ParsedSection>' or 'ParsedSection { found: bool, blocks: Vec<Block>, warnings: Vec<ParseWarning>, duplicate_sections: usize }' with structured warnings (MalformedBlockHeading, UnclosedFence, DuplicateSection).
