---
created: 2026-05-06
updated: 2026-05-09
type: feature
status: in-progress
priority: normal
reporter: jari
assignee: jari
epic: exorbitantly-ill-apples
labels: [agent-friendly, v0.6.0-candidate]
commits:
- hash: 465a9c4
  summary: initial AGENTS.md policy + agents init + doctor drift check
- hash: ba060c2
  summary: harden against malformed sentinels, schema parse errors, atomic write+lock+symlink, format-version marker
- hash: a40a156
  summary: tighten parser (inner_span, fence-aware, Result-returning regenerate)
---

# .issuectl/AGENTS.md — committed agent policy file

## Description

Maintained by 'issuectl agents init' / kept in sync via doctor. Contains durable instructions for AI agents working in this repo: 'never edit frontmatter manually', 'use issuectl set for status changes', 'run issuectl ready before marking done', 'include Refs-Issue trailers in commits', plus repo-specific schema rules (custom fields, allowed labels). Different from prompt rendering — this is policy, not prompt. Read by Claude Code automatically (AGENTS.md convention).
