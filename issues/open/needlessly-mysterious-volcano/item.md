---
created: 2026-05-07
updated: 2026-05-07
type: feature
status: open
priority: normal
---

# Keyboard accessibility for kanban status changes

_Source: src/server/client/board.js_

## Description

The drag-and-drop kanban write-back is mouse-only. Keyboard users cannot change a card's status from the board view (the detail dialog is the only path, if it exposes status editing at all).

Add a per-card 'Move to...' affordance (popover or keyboard shortcut) that triggers the same PATCH /api/issues/<slug> path used by drag-and-drop. Pairs naturally with the closed-column status picker (the picker can serve both the keyboard path and the drop-to-closed path).

Schedule for 0.6 or later. Spin-off from drag-and-drop write-back review (history/review-needlessly-fluffy-decision-dnd.md, finding #10).
