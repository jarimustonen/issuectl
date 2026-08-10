---
created: 2026-08-10
updated: 2026-08-10
type: feature
status: open
priority: normal
labels: [from-homebase-research]
---

# issuectl dag: lane/collision fields + a scheduling-DAG view command

## Description

## Origin
Recommendation from a homebase research report (`agent-dag-tool-placement`, 2026-08-09): the
per-project agent execution-DAG (today a hand-maintained markdown block in each repo's
`TODO.md`, driven by the `/stint-*` skills) should live **mostly in issuectl**, not in a new
tool and not in orchestratectl.

## Why issuectl
issuectl already owns the DAG's edges: `blocked_by` is a first-class field, `depend
add/remove` maintains the reverse `blocks` mirror, and `doctor` already detects cycles +
self-deps — i.e. the DAG's `after <slug> (needs …)` mirrors and its "no cycle / no dangling
edge" validation. The markdown block has been re-implementing what issuectl already models.

## What's missing (this feature)
1. **Two per-issue fields:** `lane` (hot-file family — the scheduling group) and `collision`
   (extra hot files beyond the lane that force spawn-time exclusion). Persisted in frontmatter,
   validated by the schema + `doctor`.
2. **`issuectl dag` view command** — renders the scheduling DAG by joining lane + order +
   `blocked_by` with live status. Computes head-of-line **on read** (never stores status —
   status stays issuectl's, the plan is lanes+deps). `--json` for agents.
3. **Spawnability is computed-on-read, not stored.** The one thing issuectl cannot know alone
   is *live run reservations* (which lane/collision files an in-flight orchestratectl run holds).
   Design the `dag` view so that signal can be supplied (e.g. an optional `--reservations
   <file|json>` the caller passes, or a documented hook) rather than issuectl reaching into
   orchestratectl. Keep issuectl orchestrator-agnostic.

## Acceptance
- `lane` + `collision` fields in the schema, round-tripped, `doctor`-validated.
- `issuectl dag [--json]` prints lanes, per-lane order, blocked_by mirror, and a computed
  head-of-line; deterministic, AI-first (§ AGENTS-AI-FIRST-CLI: noun-verb, `--json`, schema_version).
- Docs + companion skill updated; `version --json` reflects any schema bump.
- Migration note for consumers replacing the markdown `## Execution DAG` block.

Full rationale + data-model sketch: homebase `research/agent-dag-tool-placement.md`.
Target: next minor (**0.8.0**) — homebase has a follow-up gated on that release.
