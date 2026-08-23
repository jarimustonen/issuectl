---
created: 2026-08-22
updated: 2026-08-23
type: bug
reporter: mail-triage
status: in-progress
priority: normal
lane: cli-parser
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

## Triage analysis

**Verdict: real regression, normal severity, but test/CI-only in observed impact.** The Linux test lane is reliably blocked; no shipped CLI invocation has been seen to overflow.

**Reproduction and evidence.** Run 32527521337 does fail as reported, while later Linux runs fail in different parser tests (`alias_near_miss_routes_to_canonical_verb` and `body_and_comment_flags_are_aliases_for_message_on_note`), so the apply-help example is only whichever parser test reaches the limit first. A clean Rust 1.98 Linux arm64 container reproduces the named test at the normal 2 MiB Rust test-thread stack. The same test passes with 4 MiB; on macOS it passes at 2 MiB and fails at 1.8 MiB. This is finite stack pressure, not evidence of recursive apply-patch parsing.

A container comparison pins the regression to `a3119e6` (`feat: deprecate triage inbox reception path`): its parent passes the parser test at 2 MiB and the commit aborts. That commit added one parser-visible `ScanTodos.file_intake: bool` field (plus hidden clap metadata), apparently pushing the already-large generated parser over Linux's boundary.

**Reachability and impact.** Every affected test calls `Cli::command()` or `Cli::try_parse_from()`, whose clap derive builds the full, monolithic `Command` tree before selecting a verb. Parallel test scheduling explains the changing blamed test. The consequence is an early SIGABRT that prevents the Linux unit suite and green gate from completing. Product/runtime risk is currently low: normal CLI parsing occurs on the process main stack rather than a 2 MiB test worker, and Linux `issuectl apply --help` completes there. The narrow margin nevertheless makes future parser growth fragile.

**Fix sketch.** Reduce stack use in the clap construction path—prefer splitting/boxing the large derived command tree or otherwise moving generated intermediates off the test-thread stack—and retain a Linux/default-stack parser regression. Do not paper over it with repository-wide `RUST_MIN_STACK`. This is a genuine CI regression, not an overprotective speculative test finding, although it is not an apply/body-ops runtime bug.
