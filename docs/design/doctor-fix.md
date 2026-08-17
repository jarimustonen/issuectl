# doctor `--fix`: forward-progress semantics, preflight blockers, error envelope

Reference for `doctor --fix`'s apply pipeline. Origin issues:
`@doctor-fix-noop` and the apply-envelope work.

## Forward-progress only

When the apply pipeline mutates the repo (flat-layout migration, status
reconciliation, notes rename, ...) and a *later* phase finds a new critical
blocker, doctor bails with the partial progress intact rather than rolling
back. Rolling back N already-completed renames is itself a multi-step
operation that can fail mid-rollback. The `apply_outcome` JSON envelope
carries both the work that landed and the new blockers, distinguished by
`stop_phase`:

- `"ok"` — apply ran to completion (`blockers == []`).
- `"preflight"` — refused to write; no mutations landed
  (`fix_applied: false`, `blockers != []`).
- `"post_apply"` — partial-progress bail; some writes already landed
  (`fix_applied: true`, `blockers != []`). The user resolves the blockers and
  re-runs `--fix`.

Scripted callers should branch on `stop_phase` rather than infer from
`blockers` + `fix_applied`.

## Preflight blockers are layout-fatal only

Per-file manual-merge findings — `## Notes`/`## Comments` ambiguity, malformed
`.issuectl/AGENTS.md`, drift-check-skipped — drive exit-1 via
`critical_blockers` but are NOT in `apply_blockers`. They surface through
`outcome.notes_conflicts_at_apply` (and the regen-gate on AGENTS.md flags
inside `DoctorActions::from_findings`) instead of aborting the whole pass, so
orthogonal auto-fixes (alias coercion, AGENTS.md schema-block regen,
NN-rename) still run. Adding a new finding to `blockers_for(ApplyPreflight)`
requires a one-line justification that it makes the repo genuinely unsafe for
the apply pipeline (layout ambiguity, parse failure, symlink risk).

## `--fix --json` error envelope (scoped to `--fix`)

On non-zero exit, `--fix --json` emits `{"error":{"code","message","details"}}`
on stderr (stdout empty); stable codes are:

- `doctor-blocked` — preflight refusal
- `doctor-partial` — Ok with manual leftovers, PostApply bail, or critical
  findings remain
- `doctor-apply-error` — mid-pipeline failure

The full result object is nested under `details` so scripts still see what
landed. Read-only `--json doctor` keeps the historical contract — full result
on stdout regardless of exit code, so `issuectl --json doctor | jq …` on an
unhealthy repo continues to work.

## Schema-driven coercion (`required_when` + aliases)

A `FieldSpec.required_when: { status_class: <class> }` declares conditional
required fields; built-in: `closed` is required when status_class is closing.
`status_aliases` / `type_aliases` (top-level schema keys, per-key merge over
built-in defaults) map legacy values to canonical ones (closed→done,
resolved→fixed, refactor→chore, …); only `doctor --fix` consumes them and
coerces — mutation commands still reject out-of-enum values, and the mutation
RequiredWhen exemption is scoped to fields a write did **not** touch (so
explicitly clearing `closed` on a closing-status issue is rejected). A coerced
legacy status whose `closed:` is unset gets stamped from git history
(`git log -1 --format=%aI` on `item.md`, falling back to mtime, then today).
