---
created: 2026-05-07
updated: 2026-05-10
type: improvement
status: in-progress
priority: normal
---

# Add timeout / AbortController to all client write paths

_Source: src/server/client/board.js_

## Description

Both body PUT and drag-and-drop PATCH fetches lack timeouts. A hung server stalls pending_writes[slug] > 0 forever, causing same-slug SSE events to accumulate in deferred_events[slug] indefinitely until page reload.

Wrap fetch calls with AbortController + ~30s timeout, always run finishPendingWrite in finally, and surface a toast/error when the timeout fires.

Spin-off from drag-and-drop write-back round-2 review (history/review-needlessly-fluffy-decision-dnd.md follow-up, finding #H).
