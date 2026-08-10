---
created: 2026-08-10
updated: 2026-08-10
type: feature
status: in-progress
priority: normal
labels: [from-homebase-research]
commits:
- hash: bba2329
  summary: lane/collision schema fields + update write path
- hash: 1818fd1
  summary: issuectl dag scheduling-view command + tests
- hash: '7218610'
  summary: docs + skill sync for lane/collision + dag
- hash: b863bb6
  summary: apply llm-review findings (ordering/semantics/strictness)
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

## Decisions

### 2026-08-10T05:32:37Z · @agent-dag

# Design sketch — dag scheduling view (lane/collision + `issuectl dag`)

#### Data model
- `lane: Option<String>` — the scheduling group (hot-file family). An issue belongs to at
  most one lane. Absent = unscheduled (runs on its own, no lane exclusion).
- `collision: Option<Vec<String>>` — extra hot-file tokens beyond the lane that force
  spawn-time mutual exclusion (e.g. a shared file two lanes both touch).

Both are TYPED optional fields on `Issue`, lifted from the raw frontmatter mapping by the
parser exactly like `closed_by`: a *string* `lane:` and a *list-of-strings* `collision:` are
lifted into the typed slots and removed from `extra`; a malformed shape stays in `extra`
(readable, hashed as-is), so a hand-edit can't wedge the whole typed parse. Reserved
custom-field keys (`--field lane=…` rejected) with dedicated write slots on `update`.

#### canonical_hash impact (load-bearing)
`canonical_frontmatter_value` inserts a `lane` / `collision` entry ONLY when the field is
`Some` — same pattern as `closed_by`/`labels`. An issue that sets neither adds no map entries,
so its canonical projection (and thus `sha256:v1:` token) is byte-identical to pre-change.
Pinned by a `no_lane_collision_hashes_identically` test + a golden-vector check. NO bump of
`SUPPORTED_SCHEMA_VERSION` (staying at 1): these are additive optional fields inside the v1
schema format; bumping to v2 would reject every existing repo's `version: 1` .schema.yaml
(`load_uncached` bails on mismatch). So there is no on-disk schema-version change to surface;
`dag --json` still emits `schema_version` per AI-first.

#### `dag` view — everything computed on read, nothing stored
`issuectl dag [--json] [--reservations <file|-|inline-json>]`:
1. Load all active+archived issues; build the `blocked_by` graph (existing helper).
2. Group by `lane`. Issues without a lane → `unscheduled[]` (each independent).
3. Per-lane order: topological by `blocked_by` (deps first), tie-broken deterministically by
   (priority high→low, created asc, slug asc). Cycles fall back to slug order (doctor already
   flags cycles; dag stays render-only).
4. Head-of-line per lane = the first NOT-done issue in lane order whose `blocked_by` deps are
   all closing-status (done). A lane is mutual-exclusion: one runs at a time.
5. Spawnable(issue) = is_head_of_line ∧ all blockers done ∧ not reserved. Without
   `--reservations`, the reserved term is false (head-of-line is reported spawnable).

#### Reservations (orchestrator-agnostic)
`--reservations` supplies which lane/collision tokens in-flight runs currently hold. Accepted
shapes (flexible, all unioned into one reserved-token set):
- `{"lanes": ["schema"], "collision": ["crates/.../schema.rs"]}`
- `[{"run_id":"…","lane":"schema","collision":["…"]}, …]`
An issue is `reserved` when its lane ∈ reserved ∨ any of its collision ∈ reserved. issuectl
never reads orchestratectl state — the caller passes this in.

#### `--json` shape
{ "schema_version": 1, "reservations_applied": bool,
  "lanes": [ { "lane": "schema", "head_of_line": "slug|null",
              "issues": [ { "slug","title","status","priority","position",
                            "blocked_by":[…], "blockers_open":[…],
                            "is_head_of_line":bool, "spawnable":bool,
                            "reserved":bool, "lane","collision":[…] } ] } ],
  "unscheduled": [ …same issue shape, lane=null… ] }
Reuses shared vocab (slug/title/status/priority). Deterministic ordering throughout.
