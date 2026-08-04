---
created: 2026-07-26
updated: 2026-08-04
type: bug
reporter: jari
status: fixed
priority: normal
closed: 2026-08-04
---

# `--json close/update` requires --expected-version but the non-JSON path does not (output-format flag silently changes required args)

_Source: cli: close/update --json_

## Description

`issuectl --json close <slug> --status <status>` fails with `--expected-version is required with --json`, while the non-JSON `issuectl close <slug> --status <status>` succeeds with no version token. The `--json` flag — which a caller reasonably expects to change only the OUTPUT FORMAT — silently changes the REQUIRED-ARGUMENT surface, so automated/agent callers that add `--json` for machine-readable output get a hard non-zero failure until they also fetch and pass `--expected-version`.

## Observed (issuectl, orchestratectl repo, 2026-07-25)
```
$ issuectl --json close reducer-adopt-explicit-merge --status fixed
{
  "error": {
    "code": "command-failed",
    "message": "--expected-version is required with --json (per design D4=B); fetch with `issuectl show <slug> --json`"
  }
}
$ echo $?
1
$ issuectl close reducer-adopt-explicit-merge --status fixed
Closed reducer-adopt-explicit-merge (...)   # exit 0, no version token
```

The error is well-formed and has a helpful hint, so this is not a crash — it is a design-consistency / DX issue. It bit an autonomous `/stint` worker: it did `--json close`, got exit 1, and the tracking issue was left open (the caller fell back to a plain `close` on a later reconcile).

## Expected (one of)
1. `--json` is a pure output-format flag: do NOT make it mandate `--expected-version`. Keep optimistic-concurrency opt-in (pass `--expected-version` when you want the CAS check) regardless of output format.
2. If the coupling is intentional (per design D4=B), document it prominently — at minimum in the `--json` flag help on `close`/`update` — and consider having `close` without a token succeed but WARN, rather than hard-fail, so agent callers aren't blocked.

## Notes
- Same coupling presumably applies to `issuectl --json update` (same `--expected-version` semantics noted in `close --help`). Worth checking both.
- Affects automated agents most: they default to `--json` for parseable output and don't expect required-arg drift between output formats.
