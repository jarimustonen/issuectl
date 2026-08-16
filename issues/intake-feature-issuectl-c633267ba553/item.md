---
created: 2026-08-16
updated: 2026-08-16
type: feature
reporter: jari
status: open
priority: normal
labels:
- via:agent-aggountant-wrapup
- needs-triage
---

# Document how to design a lane structure: lanes are serial queues, so la…

## Description

Document how to design a lane structure: lanes are serial queues, so lane boundaries are conflict boundaries

`issuectl dag` documents what lanes *are* precisely, but not how to *choose* them — and the natural first guess (lanes = themes) is wrong in a way that only shows up later as merge conflicts.

## What I got wrong first

Restructuring a 42-issue queue, I initially proposed lanes by theme: `alv`, `tuonti`, `ci`, `tilinpaatos`. Then I read `dag --help` properly and saw the semantics:

- a lane is a **serial queue** — only head-of-line is `spawnable`
- `lane_seq` orders within a lane, after `blocked_by` and priority
- `collision` is a separate cross-lane hot-file token

That changes the design completely. The right conclusions, none of which are stated anywhere I could find:

- **Number of lanes = parallelism budget.** One deep lane = one worker. A theme lane with nine issues is nine serial slices, whether or not those issues actually conflict.
- **Lane boundaries should be conflict boundaries.** Two issues in different lanes must be independently mergeable. A theme lane is only correct when its members genuinely touch the same files.
- **Cross-lane file overlap is `collision`'s job, not a reason to merge lanes.**
- **A hot file that collects many issues is a scheduling problem.** In my case one 5172-line file had nine issues pointing at it, forcing a nine-deep serial lane; splitting the file was the highest-leverage scheduling move available, not a cosmetic refactor.
- **A "parked" lane is a useful idiom** for real findings that will never be scheduled (a standing register issue). It keeps them in the DAG so `open AND NOT laned` sweeps stop nagging, without pretending they are queued work.

## Ask

A short "designing a lane structure" section in the `dag` docs (or `dag --help`'s long help) covering the above. The reference documentation is accurate and complete about mechanics; what is missing is the one paragraph that stops a user from building a theme-shaped DAG that serialises work which could have run in parallel.

Optional and smaller: `issuectl dag` could surface per-lane depth and the count of spawnable heads, which is the number a user actually wants when asking "how parallel is my plan right now".

Filed from aggountant, where the restructure landed as 9 lanes / 8 spawnable heads.
