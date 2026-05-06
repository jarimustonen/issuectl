---
created: 2026-05-06
updated: 2026-05-06
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate, workflow, schema]
---

# Reviewer field + review_status frontmatter

## Description

Optional 'reviewer: <user>' and 'review_status: requested|in-review|approved|changes-requested' fields. Useful for teams that review through git/PRs but want issue-level review visibility. Doctor validates reviewer is a known user; queries support 'reviewer:me'.
