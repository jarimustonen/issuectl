---
created: 2026-08-12
updated: 2026-08-12
type: improvement
status: open
priority: normal
labels: [observability]
---

# Action-verb --json results don't echo the mutated field

_Source: crates/issuectl/src/main.rs_

## Description

## Observed
The mutate action verbs (`update`, `label`, `close`, likely `set`/`check`/`depend`) return a `--json` result object that does NOT include the field they just mutated. Confirmed shapes:
```
issuectl --json update <slug> --priority high  → {slug, version, dir, moved_to_closed, moved_to_open}   (.priority is null/absent)
issuectl --json label <slug> remove <label>    → {slug, version, dir, moved_to_closed, moved_to_open, warnings}   (.labels is null/absent)
```
So `issuectl --json update <slug> --priority high | jq -r .priority` yields `null`, and `… label … remove X | jq .labels` yields `null` even though the write succeeded (a follow-up `show` confirms the new value).

## Impact
issuectl is an agent-first CLI whose thesis is composable, parseable `--json` output. Today a caller cannot **confirm** a mutation from the action verb's own result — it must issue a second `show` round-trip. During a real orchestration session this repeatedly caused a false 'the write failed' read (a priority bump and a label removal both looked like no-ops in the result, though `show` proved they landed).

## Expected / fix direction
Action-verb `--json` results should echo the resulting value of the mutated field(s) so a caller can verify in one call — e.g. `update --priority` includes `priority`, `label` includes the resulting `labels` array, `close`/`update --status` includes `status`. Minimal option: include just the changed field(s). Fuller option: return the projected issue (as `show` does). Keep the existing keys (`slug`/`version`/`dir`/`moved_to_*`) for back-compat. Reconcile with the AGENTS.md '--json output contract' (action verbs → 'a result object') — this tightens what that object must carry.

## Note
Confirmed on 0.10.0 (commit ~42291be).
