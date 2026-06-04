# Focus-areas top-level approach — debate plan

**Goal:** Settle the top-level a/b/c approach for the "focus areas" feature in `@focus-areas` and record an ADR. No implementation in this run (dry-run).

## Debate Summary

**Positions:**

- **gemini-3.1-pro-preview:** opened (a) — namespaces map under `labels` in schema, CLI sugar `issuectl areas` over generic prefix mechanism. Tiebreaker: "must NOT duplicate label mechanism." Refined after round 1 to "purified (a)": drop ALL domain-specific sugar; use `issuectl stats --group-by label-namespace=area`. The string `area:` is just a prefix; the schema is generic.
- **deepseek-v4-pro:** opened (a) — essentially the same as gemini. Held (a) firmly through rebuttal. Distinguished "structural duplication" (two fields, same purpose) from "semantic specialization" (one field, optional constraints on a slice). Adopted claude's no-mirroring stance and the 6-month kill-criterion.
- **claude-opus-4-7:** opened (b) — top-level `areas:` block in `.schema.yaml`, dedicated `areas: []` frontmatter field, dedicated CLI verbs, no label mirroring. Refined under rebuttal to **(b-refined): a generic `taxonomies` registry** that addresses the opponents' generalization argument while keeping `labels` clean (open-set, descriptionless, governance-free). Areas are the bootstrap instance of the taxonomy concept; future `components`/`courses` declare new taxonomies.
- **gpt-5.5:** unavailable (quota exhausted). Three debaters proceeded.

**Points of agreement:**

- Option (c) "convention-only" was correctly dropped before debate — no schema slot for AI-readable descriptions, no CLI aggregation surface.
- AI-readable descriptions must live in `.schema.yaml`, sourced once, no drift.
- Whatever mechanism ships must be generic enough to handle `team:`, `component:`, `course:` later — a one-off `areas` block is too parochial.
- A 6-month kill-criterion conditioned on adoption is good hygiene.
- No mirroring of areas-into-labels (or vice versa) — pick one storage, render in views.

**Resolved disagreements:**

- **Where the duplication lives.** Both (a) camps argued (b) duplicates the label mechanism (two parallel tag containers). Claude (b) argued (a) is the worse duplication — semantic bimodality inside one field (some labels have descriptions + enum + governance, others don't) is more expensive forever than two lexically-distinct fields with non-overlapping semantics.
  **Verdict:** Claude's argument wins on the governance-lifecycle distinction. Labels in the existing schema are open-set by design (the schema comment literally says "optional, uncomment to constrain"); they accrete cheaply (`infra`, `urgent`, `needs-design`). Areas are closed-set, team-negotiated artifacts with descriptions and a defined addition process. Forcing both into one field requires either bimodal validation (slice of field has enum, rest doesn't — `mutate.rs` parses every label, looks up prefix table, branches) or constraining all labels (kills their open-set nature). Both are worse than two fields.
- **AI contract clarity.** `issuectl areas list --json → [{name, description, issue_count}]` (b) versus `issuectl labels list --filter-prefix=area: --resolve-descriptions` (a) or "no sugar, agents script over generic verbs" (purified a). Project policy: `/issue` skill ships in the binary and is the agent contract.
  **Verdict:** First-class verbs map cleanly to the AI-first CLI policy. Purified (a) explicitly strips the sugar that satisfies the "stats --by-area style" grounding constraint.
- **Generalization.** Opponents' strongest move: `area:`, `team:`, `component:` all want the same treatment, so the mechanism must be generic.
  **Verdict:** Adopt claude's refinement — a `taxonomies` registry in `.schema.yaml` keyed by name (`areas`, `components`, `courses`), with N typed list fields opting in via `taxonomy: <name>`. `issuectl areas` ships as sugar over the generic taxonomy mechanism because areas are the bootstrap case driving the feature; future taxonomies can add their own sugar or stay on the generic verbs.

**Verdict:** **(b), refined as a `taxonomies` registry.** Add a top-level `taxonomies:` block to `.schema.yaml` whose keys define named registered taxonomies (each with a description and a values-map where each value has its own AI-readable description). Fields opt in via `taxonomy: <name>`; element-wise enum validation derives from the registry's keys (no drift). Ship `areas` as the first taxonomy and the first first-class field with dedicated CLI sugar (`issuectl areas list/show/add`, `stats --by-area`); future taxonomies can stay on the generic `issuectl taxonomy …` verbs until they earn their own sugar. The tiebreaker is the **governance/lifecycle distinction** between labels (open-set, descriptionless, ad-hoc) and areas (closed-set, descriptive, team-negotiated) — combined with the **AI-first CLI policy** that makes `issuectl areas list --json` a strictly cleaner agent contract than any `labels --filter-prefix` variant.

This is the ADR-level call. The five open design questions in `@focus-areas` (validation strictness, sub-areas, config location vs. sibling `focus-areas.yaml`, relation to epics, sub-tags) are deferred — they presupposed knowing whether `areas` was a thing or a label-naming convention. With the top-level call made, they become tractable as a follow-up.

## Out of scope (dry-run)

Implementation does not happen here. The next ADR / issue should design the schema migration, the `mutate.rs` taxonomy-validation hook, the `/issue` skill template injection, and answer the deferred design questions.
