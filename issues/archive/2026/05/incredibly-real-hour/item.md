---
created: 2026-05-07
updated: 2026-05-10
type: chore
status: done
priority: normal
epic: hugely-exciting-spiders
labels: [monitoring]
commit: 71a440d
closed: 2026-05-10
---

# Investigate watcher race: stale snapshot after concurrent PATCH

## Description

Theoretical race surfaced by the awfully-faint-sound review (D1):\n\n1. PATCH 1 acquires flock → writes V1 → publishes seq=N V1 → unlock\n2. Watcher debouncer schedules a parse job from the rename event\n3. PATCH 2 acquires flock → writes V2 → publishes seq=N+1 V2 → unlock\n4. Watcher's parse job (started before PATCH 2's write) reads V1, publishes seq=N+2 V1 — a regression at higher seq\n\nUI symptom: card briefly flips back to old state, then catches up on next external trigger. No data loss; pure UI flicker.\n\nMonitoring task: keep an eye out for this in real use. Realistic fixes if it bites:\n- EventHub::publish dedups by version when the new IssueUpserted matches the immediately-prior one for the slug (cheap, doesn't fix order semantics)\n- Client-side per-slug recent-versions cache (cheap, no server changes)\n- Watcher takes a shared/read flock (heavy — serialises watcher reads against PATCH bursts)\n\nDecision: do not fix preemptively. Race window is narrow (two PATCHes <150ms apart); local-loopback single-user usage rarely tickles it. Reopen if user reports.\n\nReviewers: gpt-5.5 proposed read-lock; claude-opus-4-7 countered with version-dedup; deepseek-v4-pro disagreed with the read-lock fix. See history/review-flat-layout.md D1.
