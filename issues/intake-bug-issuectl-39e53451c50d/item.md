---
created: 2026-09-06
updated: 2026-09-06
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:homebase-wrapup-20260906-issuectl-broken-pipe
---

# Handle closed stdout without panicking

## Description

Handle closed stdout without panicking

## Observed

While querying the Homebase DAG, an incorrectly composed local pipeline closed `issuectl`'s stdout early. Instead of exiting broken pipe into a normal process exit, issuectl panicked:

```text
thread 'main' (...) panicked at crates/issuectl/src/cmd/mod.rs:85:40:
stdout must be writable: Os { code: 32, kind: BrokenPipe, message: "Broken pipe" }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

The command involved was:

```bash
issuectl --json dag | python3 - <<'PY'
...
PY
```

The here-document consumed Python's stdin, so the reader did not consume issuectl's pipe. The caller pipeline was wrong, but an ordinary closed stdout must not produce a Rust panic.

## Expected

Treat `EPIPE`/`BrokenPipe` while writing stdout as normal downstream termination, without a panic or backtrace. Preserve ordinary errors for other stdout write failures and add a regression test that pipes a JSON command into an early-closing consumer.
