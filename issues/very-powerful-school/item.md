---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, cli, maintenance]
---

# issuectl rename <old-slug> <new-slug> with reference updates

## Description

Safe rename that updates every reference: 'epic:', 'related:', 'blocked_by:' (once dependencies ship), '@<slug>' body refs, 'commits:' arrays, board configs, prompts. Without this, slugs become permanent — but sometimes a typo or a clearer name is worth fixing. Doctor must detect dangling refs after manual renames.
