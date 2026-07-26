---
created: 2026-06-01
updated: 2026-07-26
type: bug
reporter: jari
assignee: jari
status: fixed
priority: high
commits:
- hash: 438d22f
  summary: fix doctor --fix silent no-op on unrelated findings
- hash: 1573d2a
  summary: address multi-LLM review findings
- hash: 5a49a14
  summary: document narrowed doctor preflight scope
closed: 2026-07-26
---

# doctor --fix silently no-ops on alias coercions and AGENTS.md drift

_Source: crates/issuectl-core/src/doctor.rs (apply path)_

## Description

issuectl 0.6.1: doctor --fix exits 1 but performs zero of the fixes it just reported (status alias coercion, AGENTS.md schema-derived block regen). Files are byte-identical before/after; rerun reports the same findings. Full report stored at issues/doctor-fix-noop/bug-report.md (originally /tmp/issuectl-doctor-fix-bug.md from reporter).

## Reproduction (verified locally — minimal repro)

Setup at `/tmp/doctor-repro3/`:
- `issues/.schema.yaml` — minimal schema (built-in `closed → done` alias is enough)
- `issues/legacy-closed-bug/item.md` — `status: closed` (legacy, should coerce to `done`)
- `issues/notes-conflict-bug/item.md` — body has both `## Notes` and `## Comments` (unmergeable)
- `.issuectl/AGENTS.md` — stale managed block (drift)

Running `issuectl doctor --fix` produces the exact symptoms in the report:
- exit=1
- Both `doctor: cannot safely apply --fix...` AND `Applied. 0 ...` lines in stdout (contradiction)
- alias coercion not applied: `status: closed` unchanged
- AGENTS.md md5 unchanged

## Root cause

`notes_conflicts` is included in `blockers_for(BlockerScope::ApplyPreflight)` at `crates/issuectl-core/src/doctor.rs:866`. This makes a single unmergeable-body issue abort the entire apply pass — even though the alias-coercion and AGENTS.md regen paths are orthogonal to body content and could safely run.

## Fix scope (proposed)

A. **Remove `notes_conflicts` from preflight blockers.** Keep it as a finding the human-output reports ("merge required, doctor will skip these"), but let apply continue. `rename_notes_to_comments` already has a `notes_conflicts_at_apply` outcome field — the apply path already accounts for skipping conflict-marked slugs.

B. **Make human-output coherent on preflight bail.** If `stop_phase != Ok`, do NOT print the `Applied. N legacy dir(s)...` summary line — print a "Refused / partial — no writes" line instead, plus the blockers list.

C. **Honour `--json` error envelope on exit≠0.** Per the documented contract, exit≠0 + `--json` should emit `{"error":{"code":"...","message":"..."}}` on stderr, not a normal-shape result object on stdout. Pick a stable code (`doctor-blocked` / `doctor-partial`).

## Regression test

Add an integration test that builds the `/tmp/doctor-repro3`-style fixture (all three findings together) and asserts:
- alias coercion DID apply to the legacy issue
- AGENTS.md WAS regenerated
- the notes-conflict slug is reported as skipped (not as blocker)
- exit code matches: 0 if all auto-fixable findings applied, ≠ 0 only when something truly unfixable remains — and then with a JSON error envelope.
