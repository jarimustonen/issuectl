# Panel: focus-areas top-level approach

**Mode:** design
**Roles:** operator (gemini-3.1-pro-preview), maintainer (deepseek-v4-pro), implementer (claude-opus-4-7), contrarian (gemini-2.5-pro)
**Dropped:** api-consumer (domain role) — gemini-3-pro-preview returned 404 (deprecated); retries on gpt-5.4 and gpt-5.3-codex returned 429 (quota). Per the drop order (domain → implementer → maintainer → operator; contrarian preserved), the domain role is droppable.

## Panel summary

- **operator:** focus on `.schema.yaml` corruption / brick-the-CLI risk, mutate-lock discipline, `ConfigSource` cache invalidation, orphan-taxonomy churn breaking `doctor`. Wants safe-mode parser fallback and taxonomy_aliases for renames.
- **maintainer:** flags first-/second-class taxonomy split as a slippery slope (every taxonomy will demand its own CLI verbs). Argues schema mutation via sugar verbs is the single highest-risk surface. Wants `stats --by-area` implemented as alias over `--by-field areas` so the field-name coupling is in the schema, not the CLI.
- **implementer:** smallest viable hook is `FieldSpec.taxonomy: areas` → resolves at load time → derives `field.enum` from `values.keys()` → the existing mutate-validator needs ZERO changes. Says `version: 1` stays (additive change). `areas add` should refuse rather than rewrite YAML (comment-preservation is a rabbit hole). Flags an alternative-minimal shape: `fields.areas.enum_with_descriptions:` collapses two concepts into one and may be the better v1 if `taxonomies` registry only pays off with a second taxonomy.
- **contrarian:** YAGNI. Re-steelmans (a). "Taxonomies is a solution in search of a problem; user asked for validated labels with descriptions, not a generic registry. Governance (open-set vs closed-set) is a policy choice configurable in the schema, not a structural property that justifies a new field." Proposes `labels.namespaces.area.values.<key>.description` as the minimal shape.

## Areas of agreement

- v1 schema stays. The new top-level key is purely additive; older binaries see unknown key, ignore it. Bumping to v2 is unnecessary churn.
- `taxonomies` (or whatever the registry becomes) lives inside `schema::Schema` and rides the existing `ConfigSource` cache path. No second loader, no parallel cache.
- Any write to `.schema.yaml` (`areas add` or any future `taxonomy update`) MUST acquire the repo-wide flock via `mutate.rs`. Schema state is shared state.
- Empty-safe: repos with no taxonomies declared see zero cost — `stats --by-area` returns empty group, `areas list` returns `[]`, `/issue` skill template injects no `<areas>` block.
- Element-wise validation flows through `FieldSpec.enum` derived from the taxonomy at load time. One validation path; mutate-validator untouched.
- `doctor` needs new checks: orphaned taxonomy values (used in issues but not in schema), defined-but-unused taxonomies (warning only), and probably a soft warning on unknown top-level keys in `.schema.yaml` itself (stricter than the issue-frontmatter contract).
- Comment-preservation when programmatically rewriting `.schema.yaml` is a real concern — either preserve comments or refuse to mutate.

## Conflicts and resolutions

### Conflict 1: `taxonomies` registry vs. simpler `enum_with_descriptions` on the field

- **implementer** said: the minimum-viable shape is `fields.areas.enum_with_descriptions: { simuna-net: "Migration of …", ... }` — one config block, no cross-reference. The `taxonomies` registry only pays off when a second taxonomy actually arrives (`components`, `courses`). Until then it's premature generalization paying an indirection cost.
- **maintainer** said: the registry indirection is real cognitive load ("a new contributor will have to jump between two sections"), but the alternative `enum` semantics on the same field is "two ways to constrain the same shape" which is also confusing. Leans neutral, flags both.
- **contrarian** said: even the registry is overkill — extend `labels.namespaces` and you have ONE concept (labels) with optional metadata.

**Resolution:** The verdict's strategic call — `areas` is a separate field from `labels` — survives. The registry-vs-`enum_with_descriptions` IMPLEMENTATION question is genuinely contested and the panel did not break the tie on substance. Defer this implementation shape to the next ADR (the one that designs the schema migration). Both shapes satisfy the locked grounding constraints; both are 2-way reversible (the field name `areas` and the description-bearing values stay either way). **Reasoning:** the parent decision is about whether areas are first-class (yes, per the debate verdict on governance/lifecycle grounds); the internal schema shape is a separable design decision and forcing it here would be over-reaching past what this ADR settles.

### Conflict 2: Should `issuectl areas add` mutate `.schema.yaml` at all in v1?

- **implementer** said: refuse if the `taxonomies.areas:` section is missing; only append a new key if the section exists. Comment-preservation in serde_yaml is a rabbit hole.
- **maintainer** said: don't ship schema-mutation as a side effect of a sugar verb. Provide it only as an explicit `issuectl taxonomy update areas add <value>` with `--dry-run` showing the diff. Or document that taxonomy values are managed by hand-editing.
- **operator** said: if it ships, it MUST take the repo flock; concurrent agents adding areas / mutating issues will tear writes otherwise.

**Resolution:** Ship `areas add` in v1 ONLY as a hand-edit-or-refuse command (implementer's path) — it appends within an existing `taxonomies.areas:` section, otherwise refuses with a clear instruction. Goes through `mutate.rs` for the flock. Comment-preserving YAML rewriting and full `--dry-run`/diff UX deferred to follow-up. **Reasoning:** prefer-simpler; the high-risk surface (corrupting `.schema.yaml`) stays small in v1. The user can always hand-edit; the CLI only handles the append-to-existing case.

### Conflict 3: Hardcoded `stats --by-area` flag vs. generic `stats --by-field areas`

- **maintainer** said: hardcoding `area` in CLI flag names ties the CLI to the metadata key forever. If a repo wants `focus_areas` later, the sugar breaks. Implementation should be `stats --by-field areas` with `--by-area` as an alias.
- **implementer** said: ship the sugar; the registry knows which field carries the taxonomy.

**Resolution:** Adopt the maintainer's structure — `stats --by-field <name>` is the canonical command, `stats --by-area` is a thin alias that resolves to `--by-field areas`. Same for `issuectl areas list` (alias of `issuectl taxonomy list areas`). **Reasoning:** preserves the project policy of generic primitives + sugar; if a repo renames the field, only the alias breaks (which is the right granularity).

### Conflict 4 (the big one): is the verdict correct at all?

- **contrarian** said: option (a) — `labels.namespaces` with a `values` map carrying descriptions — satisfies every locked constraint, avoids the new-field cognitive cost, and reuses the existing label-query infrastructure. Treating governance as a structural property (rather than a schema policy on labels) is the architectural mistake.
- **implementer** said: the loader-level cost of (b-refined) is small (one cross-reference at load time) and the agent contract is strictly cleaner.
- **operator** said: any path that mutates `.schema.yaml` carries the same lock+invalidation risks; doesn't break the tie.
- **maintainer** said: prefers (b-refined) on grounds that two semantically-distinct constructs (open-set labels, closed-set taxonomies) deserve two field names — but acknowledges contrarian's point as substantial.

**Resolution:** Verdict survives. The governance/lifecycle distinction was the debate's tiebreaker; the contrarian re-frames it as configurable-policy, but doing so requires bimodal label validation (some labels prefix-validated, others not) — exactly the cost the debate concluded was worse than two fields. **Reasoning:** the contrarian's strongest point is YAGNI on the registry, NOT on first-class areas. The registry-vs-`enum_with_descriptions` question is the actual disagreement and is preserved as Conflict 1.

## Unresolved disagreements

- **Schema shape: `taxonomies` registry vs. `fields.areas.enum_with_descriptions`.** Both satisfy the locked constraints. The registry is forward-compatible with `components`/`courses` but pays an indirection cost; the inline form is one config block smaller but locks `areas` as a one-off. Decision deferred to the implementation ADR. What would settle it: a second concrete taxonomy use case (currently absent) or a benchmark on the loader-cost / cognitive-cost trade-off.
- **api-consumer perspective gap.** The role was dropped (all candidate models unavailable). Partial coverage from other roles: implementer noted JSON-shape vocab (use `slug`, not `name`/`key`; reuse AGENTS.md vocabulary) and the dogfood-test impact; maintainer noted skill-template versioning concerns. A separate review pass focused on the agent-facing contract (skill versioning, CLI SemVer for `areas` verbs, JSON shape across `areas list/show`) is recommended before the implementation ADR lands. No findings from this gap blocked the strategic call.

## Final recommendation

**Adopt the verdict — option (b-refined), `areas` as a first-class list field — with these constraints from the panel:**

1. Schema additive on v1 (no version bump); `.schema.yaml` loader must error on `enum`+`taxonomy` collision and on references to undefined taxonomies; soft-warn on unknown top-level keys in `.schema.yaml`.
2. Validation derives `FieldSpec.enum` from the taxonomy at load time so the existing mutate-validator path stays single-source.
3. All `.schema.yaml` mutations route through `mutate.rs` and take the repo flock. `ConfigSource` discipline maintained (no bare `schema::load`).
4. `areas add` in v1: append-within-existing-section, refuse otherwise. Comment-preserving rewriting deferred.
5. `stats --by-area` and `issuectl areas …` are sugar aliases over `stats --by-field areas` and `issuectl taxonomy …`. The generic primitives are the canonical commands.
6. `doctor`: warn on orphaned taxonomy values, warn on defined-but-unused taxonomies, warn on unknown `.schema.yaml` top-level keys.
7. Empty-safe across the surface: no taxonomies declared → zero cost everywhere.
8. The schema shape (registry vs. `enum_with_descriptions`) and the api-consumer review are explicit handoffs to the implementation ADR.

## Thread map

- operator: `gemini-3.1-pro-preview` / `api_f6dee5fd4df446d1af2d92ac45d71877`
- maintainer: `deepseek-v4-pro` / `api_1fe51ed9f48e48bf8d4f38dabc51f774`
- implementer: `claude-opus-4-7` / `api_bdc917f4e3ce4869ada8bd65e4c51911`
- contrarian: `gemini-2.5-pro` / `api_9fa2b8ea4dd848b099278da5892c96a6`
- api-consumer: DROPPED (all candidate models unavailable)
