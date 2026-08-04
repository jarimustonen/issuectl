---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: open
priority: high
---

# Standard intake flow for bugs and feature-requests

_Filed 2026-08-04 (high priority). **Design-first** — no implementation until the
design below is written and approved by the user._

## Design

📄 **[docs/design/intake-flow.md](../../docs/design/intake-flow.md)** — the
comprehensive design document (the deliverable for this issue). Covers the intake
model, lifecycle state machine, CLI + skill surface, responsibility split,
migration, and **13 open decisions for the user to resolve on review** (OD-1..OD-13).
Drafted, then revised after a four-model `/llm-review` (report:
`history/review-intake-flow.md`). Awaiting user review — implementation stays out
of scope until the open decisions are settled.

## Motivation

We currently have a **couple of ad-hoc issue-intake structures** bolted on
separately (e.g. the Telegram bug intake: slug `tg-bug-<user>-<chat>-<msg-id>`,
labels `via:telegram` + `needs-triage`, triaged by the `/triage-bugs` skill and
analysed by `/worktree-bug-analysis`). They should all be **replaced by one
standard, first-class intake implementation** that `issuectl` itself owns and
that any filing agent and any processing human/agent can rely on.

## What it must do

1. **Reception (filing side).** A *reporting agent* (or human) can file an
   incoming report through a standard `issuectl` path — the CLI support plus the
   skill(s) a filing agent uses. Intake should be cheap, structured, and
   self-describing.
2. **Processing (dev / product-manager side).** A developer or product-manager
   can process an intake item through a standard path — CLI support plus the
   skill(s) they use. **This replaces `/triage-bugs`** (and folds in what
   `/worktree-bug-analysis` does today).
3. **Lifecycle.** The intake item's full lifecycle is a first-class,
   well-defined state model owned here (reception → triage/analysis →
   decision → fix/defer/reject → closure), not re-invented per intake source.
4. **Feature requests too.** The same standard must handle **feature-requests**,
   not only bugs — a unified intake with a type/kind distinction.

## Two personas, clear responsibilities (to be nailed down in the design)

- **Filing agent / reporter** — captures the report, produces a well-formed
  intake item. Owns: faithful capture, initial classification hint.
- **Developer / product-manager** — triages, decides disposition, drives to
  resolution or rejection. Owns: the decision and the lifecycle transitions.

The design must state exactly **who owns which step and which state transitions**.

## The ask (this issue)

Produce a **comprehensive design document** first, covering: the intake model
(bug vs feature-request), the lifecycle state machine, the CLI surface for the
filing side and the processing side, the skill surface on both sides (and how it
supersedes `/triage-bugs` + `/worktree-bug-analysis`), the responsibility split
between the two personas, and a migration path for the existing ad-hoc intake
(the `via:telegram` / `tg-bug-*` structure). Flag open product decisions as
explicit choices for the user. **Implementation is out of scope until the design
is reviewed and approved.**

## Related / to unify or supersede

- `/triage-bugs` skill (read-only intake triage for bot-filed bugs) — to be
  replaced.
- `/worktree-bug-analysis` skill (autonomous read-only bug analysis) — to be
  folded in.
- Existing `issuectl` lifecycle: `status` + `status_classes` (active/closing) in
  `crates/issuectl-core/src/schema.rs`; labels (`via:telegram`, `needs-triage`);
  `type` enum (bug/task/feature/improvement/chore/epic).
