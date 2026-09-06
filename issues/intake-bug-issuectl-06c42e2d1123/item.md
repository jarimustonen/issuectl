---
created: 2026-08-16
updated: 2026-08-20
type: bug
reporter: jari
status: fixed
priority: high
closed: 2026-08-16
lane: cli-fixes
lane_seq: 10
provenance: agent-homebase-wrapup
---

# doctor --fix miscounts remaining findings (reports 1, lists 9)

## Description

doctor --fix miscounts remaining findings (reports 1, lists 9)

## Observed

`issuectl doctor --fix` ends with:

    Partial — 1 unfixable finding(s) remain (see above). 0 legacy dir(s) migrated,
    0 flat-layout dir(s) migrated, 0 markdown file(s) rewritten, 16 `## Notes`
    rename(s), 1 AGENTS.md block(s) regenerated.

But the output immediately above it listed **nine** unfixed findings, not one:

    Status / closed-date inconsistencies:
      all-kinds-spawn: closing status "done" requires `closed:` date
      cross-platform-lock-validation: ...
      id-canonical-form-validation: ...
      orchestrate-driver-node-id-undiscoverable: ...
      report-validator-into-core: ...
      run-state-symlink-containment: ...
      skill-bundling-campaign: ...
      test-harness-leaks-supervisors: ...        (8 of these)

    Unknown frontmatter keys (not declared by schema):
      arch-supervision-alternatives: deliverable  (1)

A follow-up read-only `issuectl doctor` confirmed all nine were still present
after the --fix run.

## Expected

The trailing count should match the findings actually left unfixed — here, 9.

## Why it matters

The summary line is what an agent or a CI check reads to decide whether the repo
is clean. "1 unfixable finding" invites treating it as a single known nit and
moving on, when in fact eight issues were missing a required field.

## Environment

Repo: taskfleet (~/Sources/taskfleet), 2026-08-16.
Reproduced simply by running `issuectl doctor --fix` then `issuectl doctor` in a
repo with more than one class of unfixable finding.
