---
created: 2026-08-17
updated: 2026-08-19
type: chore
status: open
priority: normal
related: ['@intake-bug-issuectl-bf2580033c3a']
lane: cli-fixes
lane_seq: 30
collision: [templates, crates/issuectl-core/src/schema.rs]
---

# Purge the personal intake-channel name from product surfaces

## Description

## Problem

The maintainer's personal intake channel name ("telegram") is hardwired across the
public product surface. A generic issue tracker should not name any specific chat
service in its shipped examples, schema samples, or migration vocabulary — it is a
maintainer-setup detail of the same class as the machine names covered by
@intake-bug-issuectl-bf2580033c3a (related, but filed separately because this one
touches code and templates, not just repo docs).

Where it appears (grep `telegram`, 2026-08-17):

- `crates/issuectl-core/templates/` — all six skill templates (and their dogfooded
  copies under `.claude/skills/` + `.codex/prompts/`).
- `crates/issuectl-core/src/schema.rs` — provenance enum examples in the shipped
  schema comment and tests.
- `crates/issuectl-core/src/mutate/intake_migrate.rs` — the legacy `via:telegram`
  label → `provenance: telegram` migration mapping (`L_VIA_TELEGRAM`,
  `PROV_TELEGRAM`) and its tests.
- `crates/issuectl/src/main.rs` — help text / examples.
- `crates/issuectl/tests/cli_intake.rs`, `docs/design/intake-flow.md`.

## What to do

1. **Examples and docs: neutralize.** Replace `telegram` with a generic channel
   (e.g. `chat`, `email`, `webform`) in templates, schema examples, help text,
   design docs, and tests that merely need *a* provenance value. Re-run
   `issuectl skill install --agent all --force` so the dogfooded copies match
   (the drift test enforces this).
2. **`intake_migrate`: decide, don't just rename.** The `via:telegram` mapping is
   legacy-compat for repos that used the old label convention. Either generalize it
   (`via:<channel>` → `provenance: <channel>` for any channel token) or keep the
   specific mapping but state in a comment that it is legacy-migration vocabulary,
   not an endorsed channel. Generalizing is preferred — it removes the hardwired
   name *and* covers other historical channels (`via:agent-*` already exists in the
   wild).
3. Repo-local issue files under `issues/` are historical data — leave closed issues'
   frontmatter as-is (provenance values are data, not product surface).

## Collision

Touches `main.rs`, `mutate/`, `schema.rs`, and all six skill templates — nearly every
hot file. Sequenced after @split-main-rs in the cli-fixes lane; do not run in
parallel with the skills lane (template overlap).

## Acceptance

- `grep -ri telegram` over `crates/` and `docs/` matches only (a) the generalized
  migration mechanism if that route is chosen, with a comment explaining it, or
  (b) nothing.
- Green gate + skill-template drift test pass.
