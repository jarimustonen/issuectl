# 0003 — Which frontmatter fields are typed, and why `blocked_by` is not

- Status: accepted (decisions from `@show-json-omits-blocked-by`,
  `@json-blocked-by-null-top-level`, the four-way `closed_by` review, and the
  lane-fields work)
- Deciders: maintainer

## Context

Promoting a raw `extra`-map frontmatter key to a typed `Frontmatter` field
changes how the value is folded into `canonical_hash` — and therefore changes
**every existing issue's version token**. Each promotion is a per-field
decision that weighs that hash impact.

## Decision: `blocked_by` stays in `extra`

Unlike `closed_by` (typed) or `related`/`labels` (plain-serialized),
`blocked_by` is deliberately kept as a raw `extra` map entry: it is folded into
`canonical_hash` as the raw user-written value, so typing it would invalidate
every version token. Its JSON top-level appearance is a **canonical
projection**, not a typed field: `show` / `ls` / `search --json` surface it via
the shared `project_blocked_by` helper (sorted/deduped/`@`-prefixed canonical
list; the raw `extra.blocked_by` is stripped so there is one wire
representation, plus a derived `blocks` reverse view on `show`).

Do **not** "fix" the historical top-level-`null` shape by typing the field —
that regression was considered and rejected in `@show-json-omits-blocked-by` /
`@json-blocked-by-null-top-level`. `@intensely-blushing-galley` is the
contrasting case where the hash impact of typing *was* acceptable.

## Decision: `lane` / `collision` / `lane_seq` are typed

The scheduling-DAG fields follow the `closed_by` (typed) precedent. They are
typed `Option`s on `Issue`, lifted from the raw mapping by the parser (a
*string* `lane:`, a *list of strings* `collision:`, an *integer* `lane_seq:`;
malformed shapes stay in `extra`) and projected into `canonical_hash` **only
when `Some`** — so an issue that sets none hashes identically to the pre-field
shape (pinned by `no_lane_collision_hashes_identically` +
`no_lane_seq_hashes_identically` + the unchanged `golden_hash_with_title`
vector). No `SUPPORTED_SCHEMA_VERSION` bump: they are additive optional fields
inside the v1 format, and bumping would reject every repo's `version: 1`
`.schema.yaml`.

All three are reserved custom-field keys — the only writers are `update
--lane` / `--add-collision` / `--lane-seq`. Note `lane`/`collision` are
*declared* in `DEFAULT_SCHEMA_YAML` but `lane_seq` is **not**: it is numeric
and the v1 string validator would reject the YAML integer (same reason
`commits` and `estimate` are undeclared) — so it is instead added to doctor's
hardcoded known-key list.

## Related DAG semantics (live in `crate::dag`, not in stored state)

- An `in-progress` issue **is still `spawnable`**: in-progress means *started,
  not done* — `dag` is consulted only when nothing is running, so an
  in-progress head is an idle, resumable candidate that must surface.
  Preventing a double-spawn is the caller's reservation responsibility, not
  the dag's.
- The reserved lane value **`lane: unlaned`** (`dag::UNLANED`) is a
  first-class *parallel-safe* marker — its members surface as unscheduled,
  each its own head-of-line and independently spawnable (never serialized with
  each other), distinct from an **absent** lane which means "unclassified".
- Reservations are a caller-supplied input (`--reservations`), never read from
  an orchestrator — issuectl stays orchestrator-agnostic.

See also `docs/design/lane-design.md`.
