---
created: 2026-05-06
updated: 2026-05-06
type: task
reporter: jari
assignee: jari
status: open
priority: normal
labels: [kanban, web-ui, research]
related: ['@fiercely-juicy-kettle']
epic: hugely-exciting-spiders
---

# Investigate launching Claude Code with a prompt for 'start implementation' button on cards

_Source: src/web/ (issue detail view), research only_

## Description

Spike: investigate whether the kanban can launch Claude Code (or another agent) with a pre-filled prompt derived from the issue body. The end goal is a card button like 'Start implementation' that opens Claude Code in the right repo/worktree with the issue as the prompt. Areas to explore: claude CLI invocation modes (stdin prompt, --prompt flag, file-based), URL handlers (claude:// scheme?), terminal/IDE integration, and how the web board (a browser process) can hand off to a local CLI safely. Output: a short design note in the issue with options + recommendation.
