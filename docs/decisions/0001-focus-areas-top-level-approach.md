# 0001 — Focus areas: top-level approach (a/b/c)

**Status:** Proposed
**Date:** 2026-06-04

## Question

Which top-level approach should issuectl take to satisfy the focus-areas feature request in `@focus-areas` — extend the existing label system, add focus-areas as a first-class concept, or stay on documented conventions over existing fields?

## Grounding

**Decidable horizon.** The ADR settles the top-level a/b/c approach so the five deferred design questions in the source issue (validation strictness, sub-areas, config location, area-vs-epic distinction, sub-tag standardization) become tractable. Decision shapes the next ~3 months of issuectl evolution.

**Constraints (classified):**

- *real:* an issue may belong to multiple focus areas — the storage shape must be a list, not a scalar field.
- *real:* must NOT duplicate the label mechanism (the existing schema already has `labels: list: true` with optional `enum`).
- *real:* must support AI-readable descriptions of each area so the `/issue` skill layer can auto-tag new issues.
- *real:* must enable CLI aggregation (`stats --by-area` style) — pure conventions cannot give the CLI enough structure to do this.
- *real:* optional per repo — repos that don't use focus areas must pay zero cost (no forced frontmatter field, no validation noise, no skill-template clutter).
- *policy:* AI-first CLI design — strict input validation, `--json` everywhere, no interactive prompts, mutations centralized through `mutate.rs` (one place enforces flock + schema check), `/issue` skill template embedded in the binary IS the agent-facing contract.
- *preference:* "geneerinen mutta ei liikaa" — the mechanism should be reusable enough not to be a one-off for this one need, but not pre-built for every hypothetical future need.

**Reversibility:** 2-way. The schema explicitly tolerates unknown frontmatter fields (forward-compat for issues), the `version: 1` schema is explicit, and CLI verbs can be deprecated. A first-class `areas` field can be downgraded to a label-prefix convention later; a label-prefix convention can be promoted to a first-class field later. The harder one-way step is wide downstream adoption — but at decision time there are zero downstream consumers of the feature.

**Who lives with it:** solo maintainer (Jari), AI agents consuming the `/issue` skill template that ships in the binary, downstream repos that adopt issuectl.

**"Right" criteria:**

- Customers can define focus areas with descriptions in a versioned config and the CLI validates against them.
- The CLI can list / aggregate / query by area without hardcoding repo-specific names.
- AI skills can read area descriptions and auto-tag new issues from a closed semantic set.
- Existing label workflow is not duplicated, broken, or made bimodal.
- Repos that don't declare areas see no behavior change at all.

**Repo signals consulted:** `issues/.schema.yaml`, `crates/issuectl-core/src/schema.rs` (custom-field surface), `AGENTS.md` (project conventions: mutate.rs lock discipline, `ConfigSource` cache, `/issue` skill template ownership, `--json` field vocabulary), `README.md`, `issues/focus-areas/item.md` (full requirements + 5 deferred design questions), `Cargo.toml` (no relevant deps), `docs/decisions/` (empty — this is the first ADR).

**HEAD at decision time:** `c5e0266da8a363ea28b8a76ddae9d217d5709305`.

**Implicit ratifications:** the schema already supports `labels: list: true` with optional `enum` (the .schema.yaml comment literally suggests `enum: [infra, frontend, backend]`) — i.e. the "validated-label" data path is partially scaffolded. No `areas` / `focus_areas` code exists. This is not "already-decided" because three approaches are genuinely open and the existing scaffolding does not commit to any of them.

## Options considered

**(a) Generic — extend the label system with a schema-described namespace.**
Areas live in the existing `labels` list with a reserved prefix (`area:simuna-net`). `.schema.yaml` gains a way to declare namespaces under `labels` (each namespace gets a description and an enum-of-allowed-suffixes with per-value descriptions). `mutate.rs` parses every label, splits on `:`, and validates the suffix against the namespace's enum when the prefix is registered. CLI gets either domain sugar (`issuectl areas …`) layered over the generic namespace mechanism or stays generic (`issuectl stats --group-by label-namespace=area`).

**(b) First-class — `areas` as a dedicated frontmatter field.**
A dedicated `areas: []` list field in the schema, validated element-wise against a definition block in `.schema.yaml`. Refined during debate into a generic `taxonomies` registry: a top-level `taxonomies:` block keyed by name (`areas` is the bootstrap instance) where each value carries a description, and `FieldSpec` fields opt in via `taxonomy: <name>` to derive their enum from the registry's keys. Dedicated CLI verbs (`issuectl areas list/show/add`, `stats --by-area`) ship as sugar aliases over generic `taxonomy` verbs. `/issue` skill template injects an `<areas>` block at render time. Labels stay untouched (open-set, descriptionless).

**(c) Convention-only — no code change, docs only.**
*Dropped before debate.* Violates two real constraints: (1) without a schema slot for descriptions, AI-readable descriptions live nowhere parseable; (2) without CLI knowledge of which labels are areas, `stats --by-area` cannot exist without per-repo CLI patches. Recording it as considered, not rejected on the merits — it doesn't clear the constraint bar.

## Decision

**Adopt (b) — first-class `areas` field — as the top-level approach, with the *registry-vs-inline schema shape* explicitly deferred to the implementation ADR.**

The tiebreaker is the **governance / lifecycle distinction**, reinforced by the **AI-first CLI policy** constraint. Labels in this codebase are open-set by design — the schema comment says "enum is optional, uncomment to constrain"; labels accrete (`infra`, `urgent`, `needs-design`, ad-hoc, individually-added). Focus areas are closed-set, team-negotiated artifacts with descriptions and a defined addition process. The two governance models cannot coexist cleanly in one field without either bimodal validation (parse every label, look up a prefix table, branch on whether the prefix is registered — different validation rules to elements within the same list) or constraining all labels (which kills the open-set property that makes them useful). Both alternatives are worse than two lexically-distinct fields with non-overlapping semantics.

The AI-first policy compounds this. `issuectl areas list --json` returns `[{slug, description, issue_count}]` cleanly; the (a)-equivalent is either `issuectl labels list --filter-prefix=area: --resolve-descriptions` (a leaky abstraction over labels-with-extra-data) or no sugar at all (`stats --group-by label-namespace=area`), which loses the ergonomics the grounding constraint demands. The `/issue` skill template — the only contract downstream agents see — has a clean `<areas>` block in (b); in (a) it has to inject `<labels namespace="area">` plus explain to the model both the prefix protocol and the fact that other labels in the same field are free-form.

The contrarian re-steelman of (a) (YAGNI; governance as schema policy, not field structure) is acknowledged but does not break the tie at this level. It is the strongest argument against the **internal schema shape** of (b), not against the **field-level structural call**, and is preserved as an open follow-up.

## Rejected alternatives

**(a) Generic / label-namespace.** Two debaters proposed it independently; the strongest formulation ("purified (a)") explicitly drops all domain-specific CLI sugar to avoid the leaky-abstraction smell, leaving `issuectl stats --group-by label-namespace=area` as the only access path. This trade strips the ergonomics that the "stats --by-area style aggregation" grounding constraint exists to deliver — the AI-first policy makes a generic prefix-routing CLI a worse agent contract than dedicated verbs. Combined with the bimodal-labels cost (validation, agent prompts, user mental model carry the bifurcation forever — see `panel.md` Conflict 4 and `debate.md` Resolved disagreements / "Where the duplication lives"), (a) loses to (b) on the same "must not duplicate label mechanism" constraint both sides invoke: bimodal-single-field is semantic duplication, two-field-with-distinct-semantics is only lexical duplication. The latter is cheaper.

**(c) Convention-only.** Dropped before debate — violates "CLI aggregation" and "AI-readable descriptions" because there is no schema slot for either. Per the skill's pass-at-most-two-options rule, dropped on most-real-constraint-violations (two).

## Trade-offs accepted

- **Two list-shaped tagging fields (`labels`, `areas`) in the schema.** Users will ask "why not just labels with a prefix?" recurringly. The answer ("areas have descriptions, governance, and a defined lifecycle; labels don't") has to be documented in `/issue` skill and in user-facing docs and defended.
- **`.schema.yaml` becomes a more actively-edited file** (especially if a `taxonomies` registry block lands). Mutation frequency for the schema file rises; the loader, the cache, and the lock domain all have to absorb the new write category. Op concerns (corruption, orphan values, cache invalidation) are real and inform the implementation ADR's constraints — see `panel.md` Final recommendation.
- **CLI surface grows.** New verbs (`issuectl areas list/show/add`, `stats --by-area`) plus their generic underpinnings (`taxonomy …`, `stats --by-field`). The `/issue` skill template grows a conditional `<areas>` block. Each is small, but the cumulative agent-surface noise is non-trivial and only justified by adoption.
- **Hierarchical sub-areas (`course:raksa`) punted.** v1 areas are flat. If 3DBear wants sub-areas, they use a naming convention (`course-raksa`, `course-mipa`) until a follow-up ADR decides hierarchy.
- **Internal schema shape unresolved** (`taxonomies` registry vs. inline `fields.areas.enum_with_descriptions`). Both satisfy the locked constraints; the registry pays an indirection cost for future generality, the inline shape pays a re-design cost when a second taxonomy arrives. Deferring this is itself a trade-off — implementers won't have a unified shape to copy from until the next ADR.

## Open follow-ups

- **Internal schema shape decision.** Pick `taxonomies` registry or inline `fields.areas.enum_with_descriptions` before implementation starts. Revisit if a second concrete taxonomy (`components`, `courses`) is requested — that arrival should tip the inline shape into the registry shape.
- **api-consumer review pass.** The panel could not fill that role (all candidate models unavailable). Cover before the implementation ADR lands: skill-template versioning, CLI SemVer status of new `areas` verbs, JSON-shape stability of `areas list/show` (reuse AGENTS.md vocab: `slug`, not `name`/`key`), and backwards-compat behavior for repos on issuectl v0.6.x that don't know about taxonomies.
- **6-month adoption check (revisit by 2027-01).** If by 2027-01 fewer than 2 downstream repos have populated a focus-areas block in their `.schema.yaml`, collapse the feature to a documented label-prefix convention. The first-class bet is only justified if independent teams reach for it.
- **The five deferred design questions** from `@focus-areas` (validation strictness, sub-areas, config location, area-vs-epic, sub-tag standardization) become tractable now that the top-level structural call is made.

## Provenance

- Debate plan: [.transcripts/0001/debate.md](.transcripts/0001/debate.md) — 3 debaters (gemini-3.1-pro-preview, deepseek-v4-pro, claude-opus-4-7); gpt-5.5 failed (quota). One rebuttal round; second round skipped on moderator's call because positions had crystallized and additional rounds would not change the substantive analysis.
- Panel synthesis: [.transcripts/0001/panel.md](.transcripts/0001/panel.md) — 4 roles delivered (operator gemini-3.1-pro-preview, maintainer deepseek-v4-pro, implementer claude-opus-4-7, contrarian gemini-2.5-pro). api-consumer (domain role) dropped after gemini-3-pro-preview returned 404 and gpt-5.4 / gpt-5.3-codex retries returned 429.
- No web research (the decision is internal architecture; no vendor/library candidates in question).
- Per-debater / per-role raw transcripts not persisted to disk: the repo has no `model-performance/` corpus, so `--log-prompts` was a no-op in both sub-skills. Debate and panel synthesis files capture the substance.
