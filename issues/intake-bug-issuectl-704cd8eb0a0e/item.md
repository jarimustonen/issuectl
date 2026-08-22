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
