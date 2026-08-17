# 0002 — I/O stays in `issuectl-core`; the binary crate keeps its name

- Status: accepted (2026-08-16, `@cli-canon-s22`)
- Deciders: maintainer

## Context

The AI-first CLI canon's §22 (library-first layout) asks for a domain core with
**no I/O** and a cli crate named `*-cli`. A `project-canon review
--assume-defaults` audit filed `@cli-canon-s22` claiming the repo had *"no
`crates/` directory — no core/cli split"* — which was simply false; the split
long predates the audit and core was already clap-free. (That false finding is
what produced the repo-wide *verify-before-acting* rule in AGENTS.md.)

## Decision

Both §22 asks are deliberately **rejected** — do not "fix" them:

- **I/O stays in `issuectl-core`.** issuectl is a filesystem-backed tracker
  whose markdown files *are* the domain; ~27 core modules touch `std::fs` by
  design. Hiding the disk behind a trait would be a full rewrite of core for no
  testability gain — the tests are already hermetic via tempdirs. The §22
  rationale (unit-testable domain without the CLI shell) is already satisfied:
  core has **no `clap` dependency**, and `Clock` covers the one genuinely
  untestable ambient dependency.
- **The binary crate stays `issuectl`, not `issuectl-cli`.** It is published on
  crates.io under that name; renaming breaks the published name for cosmetic
  conformance.

## Consequences

What §22 *did* yield is the `Clock` seam (see AGENTS.md "Wall-clock time goes
through `Clock`"). `issuectl-core` remains **published but explicitly
internal** (see its `lib.rs` doc comment) — `pub` items there are *not* a
semver contract; the semver contract is the `issuectl` binary's CLI surface.
