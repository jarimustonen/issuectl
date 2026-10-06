---
created: 2026-10-06
updated: 2026-10-06
type: feature
reporter: jari
status: untriaged
priority: normal
provenance: agent:3dbear-monorepo
source_ref: agent:3dbear-monorepo/reporter:jari/id:3dbear-status-summary-preserve-updated-20261006
---

# Allow summary-only custom field updates without touching updated

## Description

Allow summary-only custom field updates without touching updated

In 3dbear-monorepo a human-maintained `status_summary` custom field describes the current state of every issue, but changing only this description with `issuectl set <slug> status_summary …` resets the reserved `updated:` date to today. A backfill across ~1100 issues consequently made nearly every ticket look freshly updated in the dashboard even though none of the work progressed. `issuectl set <slug> updated <old-date>` is refused (built-in reserved); `issuectl update --patch-file` also rejects `updated`, so there is no safe CLI route for a summary-only edit that preserves it. Request a first-class guarded way to update non-progress/descriptive custom fields without touching `updated` (e.g. opt-in `--preserve-updated` for `set`/`update`, or a declared per-field policy). Default behavior for lifecycle/content changes should continue to stamp today. Dry-run, optimistic version and lock behavior must remain intact; regression test should prove a `status_summary`-only edit preserves an earlier date and a status transition stamps the new date. For now the caller must restore dates from git history and use a repository-local locked summary-only updater as a workaround.

<!-- intakectl:analysis:start job:70d7e54d-26e9-4d4f-baf2-e66235c24015 generation:0 -->
## Triage analysis

**Verdict:** Valid feature request; the described behavior is consistent with the mutation model in this repository. `issuectl-core/src/mutate/update.rs` stamps `updated` as part of ordinary updates, while `updated` is reserved against direct field assignment. That leaves callers no supported way to make a descriptive custom-field-only change without making the issue appear freshly progressed.

**Severity:** Moderate. This is consequential for repositories that use `updated` as a recency signal: bulk descriptive maintenance can distort dashboards and downstream stale-work reporting. The reported 1,100-issue backfill provides a credible real-world example, though this is not a data-loss or correctness failure in the issue contents themselves.

**Affected area:** Mutation API and CLI surface (`issuectl-core/src/mutate/` and the `set`/`update` commands), including JSON/CLI documentation and tests.

**Reproduction status:** Confirmed by code inspection; no runtime reproduction performed. The mutation layer explicitly writes today's date, and reserved-field validation prevents using `set` to restore the previous date.

**Fix sketch:** Add an explicit, guarded opt-in for preserving `updated` on narrowly scoped custom-field changes, retaining the existing default for status, content, and other ordinary updates. Ensure dry-run, expected-version checking, locking, and schema validation still apply. Add regression tests showing a custom-field-only edit preserves the prior date while a status transition updates it. Consider policy-level field declarations only if multiple descriptive fields need this behavior; a narrowly scoped option is simpler to explain and safer than silently changing the default.
<!-- intakectl:analysis:end job:70d7e54d-26e9-4d4f-baf2-e66235c24015 generation:0 -->
