---
created: 2026-05-06
updated: 2026-07-26
type: task
status: obsolete
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
related: ['@fiercely-juicy-kettle', '@entirely-cowardly-aftermath']
labels: [kanban, research, web-ui, backlog]
closed: 2026-07-26
---

# Investigate launching Claude Code with a prompt for 'start implementation' button on cards

_Source: src/web/ (issue detail view), research only_

## Description

Spike: investigate whether the kanban can launch Claude Code (or another agent) with a pre-filled prompt derived from the issue body. The end goal is a card button like 'Start implementation' that opens Claude Code in the right repo/worktree with the issue as the prompt. Areas to explore: claude CLI invocation modes (stdin prompt, --prompt flag, file-based), URL handlers (claude:// scheme?), terminal/IDE integration, and how the web board (a browser process) can hand off to a local CLI safely. Output: a short design note in the issue with options + recommendation.

## Comments

### 2026-05-08T13:50:15Z · @jari

Superseded in scope by the wider design spike
`docs/design/web-control-surface.md` — same problem framed across
all three layered mechanisms (agent-trigger, worktree-spawn,
schema-defined actions) plus five additional candidates evaluated
against a shared rubric.

This issue stays **open**. The recommended v0.6.0 first slice is
the schema-defined-actions architecture with a built-in `worktree`
action kind (§9 of the design note). When the implementation issue
for the `worktree` action kind is filed, it should reference and
close this ticket with a `Source:` line.

### 2026-05-09T04:37:57Z · @jari

Spike work is now tracked under @entirely-cowardly-aftermath in
the v0.6.0 epic (@hugely-exciting-spiders). The design note has
been through two multi-LLM review rounds and an empirical
`/assess-findings` pass. Implementation ticket comes next per
§12 of the design note; that ticket will be the one that finally
closes this issue with a `Source:` line.

### 2026-07-26T08:06:05Z · @jari

Suljettu obsolete: tutkimuksen scope korvattu valmistuneella @entirely-cowardly-aftermath -designilla (docs/design/web-control-surface.md).

