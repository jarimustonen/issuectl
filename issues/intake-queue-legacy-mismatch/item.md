---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: open
priority: normal
lane: cli-fixes
lane_seq: 20
---

# intake queue lists legacy label-based items that every intake transition then refuses

## Description

## Summary

`issuectl intake queue` lists items whose status is `open` with a `needs-triage` **label**
(the legacy, pre-`untriaged` intake shape). Every `issuectl intake <disposition>` command then
**refuses** those same items, because the transitions validate on **status** while the queue
projection evidently does not. The queue therefore hands the user a work list they cannot act on
through the intake commands at all.

## Observed (2026-08-17, issuectl 0.14.1)

    $ issuectl intake queue --json
    state: untriaged   items: 1
     - intake-feature-issuectl-77792e73735b | feature | needs_analysis: True

    $ issuectl intake accept intake-feature-issuectl-77792e73735b
    Error: transition: cannot accept intake-feature-issuectl-77792e73735b: it is "open";
    accept applies to an item in [untriaged, deferred, needs-info]

The item's frontmatter:

    status: open
    labels:
      - via:agent-homebase-wrapup
      - needs-triage

So the queue counts it as `untriaged` while the transition sees `open`. Both cannot be right.

## Why this is more than cosmetic

The shipped `/issue-intake` skill documents the **opposite** behaviour, in an explicit
"Legacy note":

> The queue reads the first-class `untriaged` **status**. A repo still carrying old label-based
> intake items (`status: open` + `label: needs-triage`) will **not** appear here — the queue
> filters strictly on status, not labels.

An agent following that note concludes a listed item must be first-class `untriaged`, calls
`intake accept`, and gets a hard error with no documented recovery. Since the skill is the only
contract consumer-side agents see, the skill and the binary disagree about the queue's own
filter — and the skill is what agents trust.

There is also no `intake` path to fix it up: the legacy item cannot be accepted, deferred,
rejected, or retyped. The only way to admit it is out-of-band —
`issuectl label <slug> --remove needs-triage` — which is exactly the hand-editing-around-the-CLI
pattern that `@intake-feature-issuectl-ff7665d266e6` was filed and fixed for elsewhere.

## Expected

Pick one and make the queue, the transitions, and the skill agree:

1. **Queue filters strictly on status** (what the skill claims): legacy label-based items stop
   appearing, and the repo needs the documented one-time intake migration to surface them. The
   queue should then say so when it detects label-based items it is hiding, rather than leaving
   the user wondering where a filed report went.
2. **Or the transitions accept the legacy shape too**: `open` + `needs-triage` is treated as
   `untriaged` for disposition purposes, so anything the queue lists can be acted on.

(2) is the smaller change and keeps already-filed reports actionable; (1) is cleaner long-term
but strands existing items until a migration runs. Either way the shipped skill's Legacy note
must be corrected in the same commit — it currently describes behaviour the binary does not have.

## Repro

Create an issue with `status: open` and a `needs-triage` label, then run
`issuectl intake queue` followed by `issuectl intake accept <slug>`.
