---
created: 2026-08-17
updated: 2026-08-17
type: task
status: done
priority: normal
labels: [architecture, decision]
lane: verb-surface
lane_seq: 5
commits:
- hash: 334dca279e1b02b0f4de27521a5f064bc0b8f8e0
  summary: ADR 0004
closed: 2026-08-17
---

# Consolidate the CLI verb surface

## Description

## Problem

The top level has ~50 commands and it keeps growing. Breadth is cheaper for an
AI-first CLI than for humans (agents read `--help`), but every verb costs skill-template
sync, docs, tests, and completion surface forever — and overlapping verbs make the
surface harder to teach in the `/issue` skill.

Known overlap clusters (from the 2026-08-17 architecture review):

- **Field mutation, five and a half ways:** `set` / `update` / `assign` / `label` /
  `apply` / `bulk`. All route through the same mutate path; the question is which
  spellings earn their place. `assign` is a wrapper over `set`; `label` has two
  equivalent invocation forms of its own; `apply` and `bulk` are batch variants.
- **`note` / `comment`** — already aliased; fine, but the ADR should bless the pattern
  (one canonical name + alias) as the norm.
- **`triage` (inbox promote) vs the `intake` flow** — two parallel reception
  mechanisms; see @deprecate-triage-inbox for the analysis. `@intake-queue-legacy-mismatch`
  is a symptom of this seam.
- **`export` / `import` vs `--json` everywhere** — is a lossy CSV/markdown export
  worth its maintenance?
- **Read views:** `stats` / `metrics` / `workload` / `burndown` / `activity` /
  `timeline` / `epic` / `cycle` — each defensible, but the cluster deserves one
  deliberate look.

## Deliverable

An ADR in `docs/decisions/` that:

1. States the policy for adding new top-level verbs (default: extend an existing verb
   with flags; a new verb needs a stated reason).
2. Classifies every current top-level command: keep / alias-then-remove / fold into
   another verb / leave as-is. Removals must follow a deprecation window (hidden alias +
   warning first), since published agents' skills reference these verbs.
3. Sequences the resulting implementation issues (each fold/removal is its own small
   issue; they mostly touch `main.rs`, so they slot into the post-split module files).

## Constraints

- The `/issue` + `/issue-new` + `/issue-intake` skill templates are the agent-facing
  contract — any surface change lands in the same commit as a template update
  (AGENTS.md critical rule).
- CLI verbs are the semver contract of the `issuectl` binary. Removals are breaking:
  batch them for a minor (pre-1.0) release and say so in the CHANGELOG.
- Decision only in this issue — no code changes. Implementation issues come out of the
  ADR.
