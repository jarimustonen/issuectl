---
created: 2026-08-16
updated: 2026-08-16
type: improvement
status: open
priority: normal
labels: [tooling, cli-canon]
---

# cli-canon: §22 Internal layout: library-first core/cli split

## Description

Filed by the `stack-cli-alignment` comparative-audit rollout (homebase epic),
via `project-canon review --assume-defaults` (project-canon 0.1.1). Advisory
recommendation — **severity: should**, never a release-readiness failure.

**Canon:** AGENTS-AI-FIRST-CLI.md §22 — Internal layout: library-first core/cli split.

**Observed:** no `crates/` directory — no core/cli split.

**Expected:** a library-first layout —
`crates/issuectl-core` (pure domain logic, **no clap / no I/O**) plus `crates/issuectl-cli`
(the clap surface + I/O), with an **injected `Clock`** in core for testable time,
and shared plumbing factored into a thin `*-cli-common`-style crate where the family
converges on one.

**Why:** keeps domain logic unit-testable without the CLI shell, matches the
reference-tier tools (ossctl, orchestratectl), and is the single remaining canon gap
this repo carries per the family audit.

**Check:** read `Cargo.toml` members; grep the core for `clap` / I/O imports.
