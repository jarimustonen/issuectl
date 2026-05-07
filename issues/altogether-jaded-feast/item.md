---
created: 2026-05-06
updated: 2026-05-06
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [schema, v0.6.0-candidate, workflow]
---

# Reviewer field + review_status frontmatter

## Description

Optional 'reviewer: <user>' and 'review_status: requested|in-review|approved|changes-requested' fields. Useful for teams that review through git/PRs but want issue-level review visibility. Doctor validates reviewer is a known user; queries support 'reviewer:me'.
