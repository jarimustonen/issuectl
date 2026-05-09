---
created: 2026-05-09
updated: 2026-05-09
type: bug
status: open
priority: normal
epic: exorbitantly-ill-apples
---

# Doctor: legacy_number_from_mapping ignores user-supplied slug when number is also present (data loss)

## Description

Spin-off from @greatly-flat-sleet round-2 /llm-review (DeepSeek#1).
Pre-existing bug — predates the apply-pipeline refactor — but real
data-loss potential, deserves its own issue.

## Problem

`crates/issuectl-core/src/doctor.rs::legacy_number_from_mapping` checks
`number:` BEFORE `slug:`:

```rust
if let Some(v) = m.get(serde_yaml::Value::String("number".into())) {
    if let Some(n) = v.as_u64().and_then(|u| u32::try_from(u).ok()) {
        return Some(n);                     // <-- early return
    }
}
if m.get(serde_yaml::Value::String("slug".into()))
    .and_then(|v| v.as_str())
    .is_some()
{
    return None;
}
```

When a frontmatter has BOTH `number:` and `slug:` (e.g. a user manually
added `slug:` to a legacy numbered issue intending to fix the slug
themselves), the early `return Some(n)` fires and the `slug:` check
is never reached. `--fix` then classifies the dir as legacy, generates
a fresh random slug via `slug::generate_unique`, renames the directory,
and rewrites the frontmatter. **The user-assigned slug is lost.**

## User impact

- Triggering condition: a legacy `<NN>-<slug>/item.md` whose
  frontmatter the user manually augmented with `slug:` (the obvious
  workaround for "I want to choose the post-migration name").
- Visible effect: `--fix` reports a successful migration. The
  user's chosen slug is gone, replaced by a random
  `intensifier-adj-noun` slug. External links (`@<old-slug>`) break.
- Severity: silent data loss + petollinen onnistumisilmoitus.

## Why this needs its own design

The one-line fix (swap the order) is obvious, but the right behaviour
is debatable:

1. Should the user's `slug:` override the legacy classification
   silently? (Risk: a user typo in `slug:` becomes the canonical
   slug forever.)
2. Should it be a Hard parse error so `--fix` refuses until the user
   resolves the ambiguity?
3. Should `--fix` honour the user's slug but still rewrite refs +
   drop `number:`?

Each choice has different testing needs and may surface during
`/llm-review`. Scope: pick one, document in CLAUDE.md / AGENTS.md
under the legacy-migration rules, add a regression test for the
`number+slug` combination.

## Acceptance criteria

- `legacy_number_from_mapping` produces unambiguous behaviour when
  both `number:` and `slug:` are present.
- New test asserts the chosen behaviour for the dual-key case.
- Existing test `scan_does_not_migrate_user_slug_starting_with_digits`
  still passes.

## Related

- Round-2 /llm-review on @greatly-flat-sleet (DeepSeek#1)
- `crates/issuectl-core/src/doctor.rs::legacy_number_from_mapping`
