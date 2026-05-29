---
created: 2026-05-06
updated: 2026-05-29
type: feature
status: done
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [cli, maintenance, v0.6.0-candidate]
closed: 2026-05-29
---

# issuectl rename <old-slug> <new-slug> with reference updates

## Description

Safe rename that updates every reference: 'epic:', 'related:', 'blocked_by:' (once dependencies ship), '@<slug>' body refs, 'commits:' arrays, board configs, prompts. Without this, slugs become permanent — but sometimes a typo or a clearer name is worth fixing. Doctor must detect dangling refs after manual renames.
