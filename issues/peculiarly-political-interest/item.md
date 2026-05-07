---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: high
epic: exorbitantly-ill-apples
labels: [cli, agent-friendly, foundation]
---

# Agent-safe mutation CLI: set / note / check / label / apply (+ --dry-run)

_Source: src/cli/{set,note,check,label,apply}.rs (new), src/mutate.rs (shared with web — see worktree)_

## Description

Stop agents and humans from sed-ing YAML. Add commands like 'issuectl set <slug> status testing', 'issuectl label add/remove', 'issuectl note <slug> "..."' (append-only), 'issuectl check <slug> "task"' (toggle markdown checklist), 'issuectl apply <patch.yaml>' (transactional multi-field). Every command supports --dry-run that prints a diff. Builds on the shared mutate.rs introduced in the web-edit-sync worktree (docs/design/web-edit-sync.md §3) — same flock + atomic-write protocol used by both CLI and the web write endpoints.
