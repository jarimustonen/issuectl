---
created: 2026-05-09
updated: 2026-05-09
type: chore
reporter: jari
status: open
priority: normal
epic: exorbitantly-ill-apples
related: ['@ridiculously-outgoing-brass']
labels: [release-v0.5.0]
---

# Derive lifecycle status classification (active vs closing) from schema/transitions

## Description

Spin-off from @ridiculously-outgoing-brass /llm-review (round 1) — `is_closing_status`/static `CLOSING_STATUSES` cement the wrong abstraction.

## Problem

`schema::status_universe()` already derives the allowed status set from the project schema (`enum:` constraint on the `status` field), with `issue_fields::all_statuses()` only as the fallback when the schema has no explicit `enum:`. This means a project schema can introduce custom statuses like `archived` or `verified`.

But `is_closing_status` is hardcoded against `CLOSING_STATUSES` in `crates/issuectl-core/src/issue_fields.rs`:

```rust
pub fn is_closing_status(status: &str) -> bool {
    CLOSING_STATUSES.contains(&status)
}
```

So a schema-allowed `archived` status will:

- not be classified as closing in `repo::folder_for_status` → issue stays in the open bucket
- not stamp `closed:` in `mutate::update_issue_under_lock` → no lifecycle timestamp
- not match doctor's open/closed consistency check
- not appear in `mutate`'s close-defaulting logic

Schema lets users add the status; the rest of the codebase silently treats it as active.

## Why this needs design

Several plausible answers to "where does lifecycle classification live":

1. **Extend the schema field spec** with a per-value `closing: true` flag (e.g. under `status:` in the schema YAML), and have `is_closing_status` consult schema-derived metadata.
2. **Promote `transitions.yaml` to authoritative**, e.g. a status is "closing" iff a rule with kind `set-closed` reaches it, or some equivalent declarative form.
3. **Keep static built-ins as defaults** but give projects a `closing_statuses:` override in `.issuectl/` config.

Each option has cascading impact across `mutate`, `repo`, `doctor`, `transitions`, `schema`. None is a one-PR fix.

## Definition of done

- A status added to a project schema (or transition rules) can be classified as closing without code changes to `issuectl-core`.
- `repo::folder_for_status`, `mutate`'s close-on-status logic, and `doctor`'s consistency check all agree on the lifecycle classification of any user-defined status.
- `is_closing_status`'s hardcoded fallback (or an equivalent default) survives for projects with no schema.
- Existing built-in statuses behave exactly as today (regression tests preserved).

## Out of scope

- Removing the built-in defaults entirely.
- Renaming any existing closing status.
- Behaviour changes on projects that haven't customised statuses.

## Origin

Surfaced by /llm-review on @ridiculously-outgoing-brass (commit ceedc71). OpenAI flagged the schema-vs-static divergence; Anthropic and DeepSeek confirmed. Pre-existing bug (predates the lib/bin split), but the split made it more visible by enshrining the static set in a public domain module.
