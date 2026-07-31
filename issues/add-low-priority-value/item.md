---
created: 2026-07-31
updated: 2026-07-31
type: feature
status: open
priority: normal
---

# Add `low` as a valid priority value

_Source: crates/issuectl-core/src/issue_fields.rs PRIORITIES_

## Description

`issuectl new --priority low` (and `set/update --priority low`) is rejected: `error: invalid value 'low' for '--priority' [possible values: normal, high]`. Many backlog items are genuinely low-priority (build-hygiene sweeps, nice-to-haves), so the two-value set forces them to `normal` and flattens the signal.

## Change
Add `low` as the lowest priority. Single source of truth is `PRIORITIES` in crates/issuectl-core/src/issue_fields.rs (currently `&["normal", "high"]`) → `&["low", "normal", "high"]` (ascending). The three clap `PossibleValuesParser::new(PRIORITIES)` sites (new/ls/update in crates/issuectl/src/main.rs) and both validation sites (crates/issuectl-core/src/mutate/mod.rs) derive from that const, so they pick it up automatically. `default_value = "normal"` stays.

## Observed vs expected
- **Observed:** `--priority low` → invalid-value error; only `normal`/`high` accepted.
- **Expected:** `low`, `normal`, `high` all accepted; default remains `normal`.

## Out of scope
- Priority-based *sorting* (no rank function exists yet; that is a separate future item, @truly-somber-payment). This change only widens the accepted set.

## Skill-sync (required)
Per AGENTS.md 'keep the skill in sync with the CLI', update crates/issuectl-core/templates/issue-skill.md (the `normal, high` enumerations at ~lines 109, 206, 350, 505), regenerate issue-prompt.md, and run `issuectl skill install --agent all --force`.
