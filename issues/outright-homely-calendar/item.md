---
created: 2026-05-06
updated: 2026-05-06
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: exorbitantly-ill-apples
labels: [format, git-native]
---

# issuectl fmt + optional YAML merge driver for item.md

_Source: src/cli/fmt.rs (new), src/merge_driver.rs (new), .gitattributes_

## Description

Two pieces, one issue: (1) 'issuectl fmt' normalizes frontmatter key order, array sorting, timestamp format, blank-line policy, markdown heading style. Idempotent. Reduces YAML churn and makes agent edits reviewable. (2) 'issuectl merge-driver' as a git custom merge driver (configured in .gitattributes for issues/**/*.md) that union-merges array fields (labels, related, blocked_by, commits) and picks the newest 'updated:'. Mitigates the #1 reason file-based trackers break for small teams. Important to land BEFORE drag-and-drop write-back (@needlessly-fluffy-decision) ships, since web mutations multiply YAML churn.
