---
created: 2026-07-28
updated: 2026-07-31
type: chore
status: in-progress
priority: normal
---

# unreachable-expression warning: return fail(...) in cmd_show

_Source: crates/issuectl/src/main.rs cmd_show_

## Description

`cargo build` emits a pre-existing `warning: unreachable expression` at crates/issuectl/src/main.rs (~line 2421, in `cmd_show`). In the `resolve_slug_input` error arm the code does `return fail(json, 1, "ambiguous-slug", …)`; `fail()` diverges (returns `!`), so the `return` keyword wrapping a never-typed value is flagged as an unreachable expression.

## Observed vs expected
- **Observed:** every `cargo build` prints one `unreachable expression` warning (`#[warn(unreachable_code)]`), keeping the build output noisy.
- **Expected:** clean build. Drop the redundant `return` (call `fail(...)` as the tail expression of the match arm) or restructure so the divergence is not wrapped in `return`.

## Notes
- Pre-existing: present on the pre-round baseline (52b3598), not introduced by the 2026-07-26 CLI-alias round; the clippy warning count was identical (65) before and after. Cosmetic only — no behavioural defect; a build-hygiene sweep item.
