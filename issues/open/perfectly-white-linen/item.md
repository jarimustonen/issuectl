---
created: 2026-05-07
updated: 2026-05-07
type: feature
status: open
priority: normal
---

# Floating Undo for kanban drag-and-drop moves

_Source: src/server/client/board.js_

## Description

After a drag-and-drop status change lands, surface a small floating Undo button (e.g. bottom-centre toast or persistent corner pill) that reverts the move via a follow-up PATCH back to the previous status + version. Window of ~10s, dismissable.

Rationale: optimistic moves are immediate and silent on success; a misclick currently requires opening the detail dialog (or dragging back). Undo turns the operation reversible at one click.

Implementation notes:
- Capture {slug, prevStatus, prevVersion} at drop time.
- After 200, push a 'Move undone? Click to revert' affordance instead of (or alongside) the silent success.
- Undo click sends PATCH with expected_version = the version returned from the original write, status = prevStatus.
- If the user has since dragged the same card again, hide/disable the Undo (state.pending_writes guard).

Spin-off from the round-2 review smoke test of @needlessly-fluffy-decision.
