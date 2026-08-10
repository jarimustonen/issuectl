---
created: 2026-05-11
updated: 2026-08-10
type: chore
status: obsolete
priority: normal
labels: [monitoring, web-edit-sync, deferred]
related: ['@incredibly-real-hour']
closed: 2026-08-10
---

# Add per-slug version dedup / observability log for watcher race

## Description

Spin-off from /llm-review of wt-cleanup-internals (referenced in @incredibly-real-hour). 

The watcher race documented in parse_slug_state's doc comment (concurrent PATCH bursts emitting V1 at a higher seq than V2) is currently "monitor rather than fix." The decision is defensible because the window is narrow, but "monitor" is meaningless without an observability hook — the doc says "reopen if it surfaces in real use" yet there's no signal that distinguishes the race from any other transient UI desync. Bug reports won't mention it because the SPA self-heals via the 409 conflict-recovery path on the user's next action.

## Goal

Add hub-level per-slug last-version tracking so we can either:

(a) Log when parse_slug_state publishes a hash that's older than the last published hash for the same slug (rate-limited debug-level), or

(b) Dedup outright at hub.publish() — drop the older event before it reaches subscribers (this is the cheap fix the review thread proposed).

(a) is non-controversial (pure observability); (b) is a semantics change worth its own review because the hub's current contract is "publish everything in seq order."

## Why this is a separate issue

The work touches EventHub state (new per-slug LRU or BTreeMap), is a behaviour change at the publish boundary, and the dedup vs log-only choice is itself a design decision. Bundling it into wt-cleanup-internals would have inflated that branch beyond the "internals cleanup" framing.

## Resolution

### 2026-08-10T10:03:40Z · @issuectl

Web/browser UI is being removed from issuectl (product decision 2026-08-10). This is a web-board enhancement, so it is obsolete. See @remove-web-ui.
