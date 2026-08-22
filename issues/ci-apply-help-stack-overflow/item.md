---
created: 2026-08-22
updated: 2026-08-22
type: bug
reporter: mail-triage
status: open
priority: normal
---

# Linux CI stack-overflows parsing apply help example

## Description

## Description

The Linux CI test job aborts with a stack overflow in the CLI parser regression test `cmd::cli_tests::tests::apply_help_body_ops_example_parses_as_a_patch`. The failure persists across the three latest `main` commits (`7a548fb`, `2e272e2`, `6ebbf64`) after an earlier green run, so this is not a stale notification.

## Evidence

Latest failing run: https://github.com/jarimustonen/issuectl/actions/runs/32527521337

```text
Running target/debug/deps/issuectl-6e3c6640e042d0ed
running 105 tests
thread 'cmd::cli_tests::tests::apply_help_body_ops_example_parses_as_a_patch' has overflowed its stack
fatal runtime error: stack overflow, aborting
error: test failed, to rerun pass `-p issuectl --bin issuectl`
process didn't exit successfully ... (signal: 6, SIGABRT)
```

The failure appears while parsing the `apply --help` body-ops example, before the rest of the binary test suite can run. Investigate recursive clap parsing or an unexpectedly large parser value on the test thread's default Linux stack. Add a regression that passes under the normal CI stack rather than masking the issue by increasing `RUST_MIN_STACK`.
