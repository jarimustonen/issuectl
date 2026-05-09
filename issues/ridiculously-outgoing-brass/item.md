---
created: 2026-05-09
updated: 2026-05-09
type: chore
reporter: jari
status: in-progress
priority: normal
epic: exorbitantly-ill-apples
related: ['@partially-ahead-button']
labels: [release-v0.5.0]
commits:
- hash: ceedc71
  summary: split src/lib.rs and relocate issue-domain primitives
- hash: e81daab
  summary: 'workspace split + migrate_layout hardening + #9 #12 fixes'
- hash: a581c62
  summary: round-2 review fixes (R1-R7,R10,R11,R13) + spin-off @greatly-flat-sleet
- hash: 1e3844e
  summary: round-3 review fixes (R3.1-3.4,3.6); pre-existing legacy_number ambiguity deferred
---

# Move issue-domain constants and helpers out of main.rs (and consider lib.rs split)

## Description

Spin-off from @partially-ahead-button /llm-review (round 1).

## Description

After @partially-ahead-button extracted `do_new_locked` and `normalize_related_refs` out of `main.rs`, `mutate.rs` still reaches across the binary root for several issue-domain primitives:

- `crate::ISSUE_TYPES` (used at mutate/mod.rs:1099)
- `crate::PRIORITIES` (used at mutate/mod.rs:1105 + mutate/mod.rs:198)
- `crate::all_statuses()` (used at mutate/mod.rs:191)
- `crate::is_closing_status()` (used at mutate/mod.rs:561, 562, 729)
- `crate::RESERVED_CUSTOM_FIELD_KEYS` (referenced from main.rs::tests; defined in mutate.rs but conceptually domain)

The reviewers (anthropic, openai) flagged this as the same architectural problem that motivated @partially-ahead-button, just for constants/helpers instead of the do_new cluster.

## Combined scope: lib.rs split

The single architectural change that makes the AGENTS.md rule actually enforceable — rather than aspirational — is splitting the binary crate into:

- `src/lib.rs` — domain modules (mutate, write, parser, schema, refs, etc.)
- `src/main.rs` — just `fn main`, clap definitions, and the `cmd_*` dispatch handlers

Without this, every domain module's `crate::*` resolves to `main.rs`, and module privacy can't enforce layering. With it, domain modules see their crate root as `lib.rs` (which contains only domain types/helpers), and `main.rs` becomes a thin consumer.

This is a strategic v0.5.0 anchor: it lets future domain refactors stop fighting the binary-root pattern.

## Fix sketch

1. Move issue-domain constants and helpers into a domain module:
   - `ISSUE_TYPES`, `PRIORITIES`, `ACTIVE_STATUSES`, `CLOSING_STATUSES`, `all_statuses`, `is_closing_status` → `src/issue_fields.rs` (or `src/domain.rs`).
   - `RESERVED_CUSTOM_FIELD_KEYS` already lives in `mutate.rs`, but it conceptually belongs alongside the field validators — possibly `src/custom_fields.rs`.
2. Promote the binary to lib + thin main:
   - Move all `mod foo;` declarations from `main.rs` into a new `src/lib.rs`.
   - `main.rs` keeps only the clap struct + `fn main` + the `cmd_*` handlers (which call into `issuectl::*`).
   - Audit `pub(crate)` visibility — items needed by `main.rs` become `pub` in `lib.rs`.
3. Re-target every `crate::FOO` reference inside domain modules. Most should now resolve via the new module path; the rest become outright errors that flag the leak.
4. Verify: `cargo build` + `cargo test` on the test suite (currently 457 tests).

## Definition of done

- No domain module under `src/` references `crate::*` for items the new lib.rs does not own.
- `main.rs` is small (clap + `fn main` + cmd_* handlers; ~few hundred lines).
- AGENTS.md rule from @partially-ahead-button now enforceable: any future `mutate::*` reaching `crate::*` for a non-CLI item is a compile error.
- All 457+ tests still pass.
