---
created: 2026-08-22
updated: 2026-08-22
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:orchestratectl-stint8-issuectl-update-blocked-by-json
---

# update JSON omits persisted blocked_by after add-blocked-by

## Description

update JSON omits persisted blocked_by after add-blocked-by

## Observed

With issuectl 0.17.0, a positional update that explicitly requested a DAG dependency persisted the dependency but did not echo the persisted value in the JSON response as documented.

This occurred twice in the orchestratectl repository on 2026-08-22:

```sh
issuectl --json update add-configurable-agent \
  --status open \
  --add-blocked-by '@worker-telemetry-protocol'
```

Reading `.data.blocked_by` immediately from the successful response yielded `null`/absence. A second invocation on another issue behaved the same way:

```sh
issuectl --json update worker-control-plane-review \
  --add-blocked-by '@worker-telemetry-protocol' \
  --add-blocked-by '@add-configurable-agent'
```

In both cases a subsequent `issuectl dag --json --reservations '[]'` showed the requested `blocked_by` arrays correctly persisted, so this is response projection drift rather than a failed write.

## Expected

The installed `/issue` contract says that when an update invocation requests a scheduling-field operation, `.data` echoes that field's persisted post-update value; a missing key means the operation was not requested, while present `null` means cleared/unset. Therefore `--add-blocked-by` should return the resulting `blocked_by` array, just as scheduling setters such as collision are echoed.

## Impact

A machine caller cannot trust the mutation response and must perform a follow-up DAG/read call to distinguish a successful dependency update from a missing/cleared value. Returning `null` is especially misleading because it means the opposite of the persisted state.

## Triage analysis

**Verdict: expected behaviour under the current contract; low severity.** The reported occurrence is real, but the issue interprets the update echo promise too broadly.

Reproduced with the shipped `issuectl 0.17.0` in a temporary repository. A positional `update blocked-subject --add-blocked-by @blocking-prerequisite --json` exited successfully and persisted `blocked_by: ['@blocking-prerequisite']`; `show --json` returned `.data.blocked_by == ["@blocking-prerequisite"]` and `dag --json` contained the edge. The update result itself had no `blocked_by` key (`has("blocked_by") == false`); `jq .data.blocked_by` therefore displays `null`, but the CLI did not return a present JSON null.

The shipped `/issue` template promises conditional post-update echoes for the three optional typed scheduling fields it then names and exemplifies: `lane`, `lane_seq`, and `collision`. Its generic update result shape does not include `blocked_by`. The wording “a scheduling-field operation” is broad enough to invite the reporter's reading, because `blocked_by` also drives scheduling, but the surrounding enumeration, implementation, and regression tests scope the promise to those three fields. `UpdateEchoes` in `crates/issuectl/src/cmd/write.rs` tracks only those fields (plus `title`), while the blocked-by tests verify persistence and the canonical read projection rather than an update-result echo.

This distinction is deliberate: ADR 0003 keeps `blocked_by` in `Issue::extra` to avoid version-token churn. `project_blocked_by` in `crates/issuectl/src/cmd/read.rs` provides its canonical top-level `@`-prefixed array for issue-reading results (`show`/`ls`/`search`); an `update` response is an action result, not that full issue projection. Nothing here justifies typing `blocked_by`.

Affected callers are agents or orchestrators that add/remove a dependency and then assume the mutation result is a full issue object. They may raise a false alarm or need a follow-up `show`/`dag`, but no edge is lost and the response's new `version` confirms a mutation landed. This is therefore not an implementation bug against the current documented output shape; at most it is a documentation ambiguity exposed by a real machine caller.

**Narrow correction:** change the template sentence to say explicitly “when an update requests `lane`, `lane_seq`, or `collision`…” and state that `--add/--remove-blocked-by` callers must use `show` or `dag` to read the canonical edge. If a one-call dependency confirmation is desired as a new additive contract, extend `UpdateEchoes`/`UpdateOutcome` to emit the canonical top-level `blocked_by` array only when a blocked-by operation was requested, with black-box add/remove/no-op tests; keep storage in `extra`.
