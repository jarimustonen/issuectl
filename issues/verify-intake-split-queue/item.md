---
created: 2026-08-06
updated: 2026-08-06
type: task
status: done
priority: normal
commits:
- hash: 1cf6c3c
  summary: verify §6 split queue + human-mode regression test
closed: 2026-08-06
---

# Verify intake queue surfaces legacy label-encoded items (transitional split queue)

## Description

docs/design/intake-flow.md §6 'transitional split queue' specifies that `issuectl intake queue` should also surface recognised legacy forms (open + needs-triage/deferred labels, via:telegram) with a `legacy: true` flag + a 'run intake migration' nudge, until a repo reports migration-complete — so the queue never silently abandons un-migrated items. Verify this shipped in 0.7.0; if not, implement it. It is the safety net for the homebase/deutschpad `issuectl intake migrate` runs.

## Verification

**What §6 requires (the transitional split queue):** until a repo runs `issuectl intake migrate`, `intake queue` must also surface recognised legacy label-encoded items so they are never silently abandoned — a legacy `untriaged` form (`status: open` + `label: needs-triage`) and a legacy `deferred` form (`status: open` + `label: deferred`), each flagged `legacy: true` (JSON) / `[legacy]` (human); a "run intake migration" nudge (`legacy_pending` count + `migration_hint` in JSON, a `Note:` line in human) while legacy items are pending; and, after `intake migrate --apply`, the item becomes a first-class `untriaged` item (`legacy: false`, provenance set) with the flag/nudge gone.

**What shipped (0.7.0):** `crates/issuectl/src/main.rs` — `fn is_legacy_for` (keys `open + needs-triage` → legacy `untriaged`, `open + deferred` → legacy `deferred` off the queue `target`) and `cmd_intake_queue` (per-row `legacy`, top-level `legacy_pending` + `migration_hint` in JSON; inline `[legacy]` flag + trailing `Note:` nudge in human mode). `queue_provenance` derives `telegram` from `via:telegram` for unmigrated items so `--provenance telegram` still finds them. Migration itself in `intake migrate` (dry-run default).

**Tests covering it** (`crates/issuectl/tests/cli_intake.rs`, all green):
- `queue_surfaces_legacy_form_with_flag_and_nudge` — legacy `untriaged` form, JSON `legacy`/`legacy_pending`/`migration_hint`.
- `queue_state_deferred_surfaces_legacy_deferred_form` — legacy `deferred` form surfaced only under `--state deferred`.
- `queue_provenance_filter_surfaces_unmigrated_telegram_items` — `--provenance telegram` finds unmigrated legacy items, filters out non-telegram.
- `migrate_dry_run_then_apply_is_idempotent` — post-`--apply`: `legacy: false`, provenance set, nudge gone, idempotent.
- `queue_human_mode_flags_legacy_row_and_prints_nudge` — **added by this issue** to close the one gap: the human output path (`[legacy]` inline flag + `Note:` nudge) was implemented but exercised by no test (all queue tests used `--json`).

**Conclusion:** §6 shipped and works. Every acceptance bullet is met in the shipped code across both JSON and human paths, for the `needs-triage` and `deferred` legacy forms. The only gap was a missing regression test for the human output path, now added (test-only; no production change needed). Closed as done.
