---
created: 2026-05-06
updated: 2026-05-06
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [cli, qol]
---

# QoL bundle: triage inbox + fuzzy picker + slug prefix matching + shell completions + scan-todos

_Source: src/cli/{pick,scan_todos,completions}.rs (new), src/repo.rs (slug prefix resolution), issues/inbox/_

## Description

Five small but high-leverage UX wins, bundled because each is small. (1) issues/inbox/<slug>/item.md as a low-friction landing zone for half-baked drafts; triage moves them to issues/open. (2) 'issuectl pick' fuzzy interactive picker over slug + title + labels for piping into other commands. (3) Slug prefix matching: 'issuectl show extremely-quiet' resolves uniquely; ambiguous prefixes print candidates. Mitigates the random-slug usability tax. (4) 'issuectl completions {zsh,bash,fish}' generating shell completions for slugs, statuses, labels, users. (5) 'issuectl scan-todos' greps source for '// TODO(issue: slug)' / 'TODO(issue:)' patterns; reports stale (issue closed) and untracked (no slug); can create inbox issues. Repository-local, no external index.
