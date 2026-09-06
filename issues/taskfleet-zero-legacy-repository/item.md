---
created: 2026-09-06
updated: 2026-09-06
type: task
status: open
priority: high
related: ['@taskfleet-issue-intake-template-convergence']
---

# Converge entire issuectl repository on Taskfleet identity

## Goal

Converge the entire maintained issuectl repository on the final Taskfleet clean-break identity. The earlier template-only change intentionally retained compatibility fixtures and historical wording; that policy is superseded.

## Required work

Review every tracked source, generated artifact, test, fixture, snapshot, script, workflow, document, issue, and agent instruction. Remove the retired product, command, environment-prefix, package, protocol, and repository identity from maintained HEAD rather than preserving it as a compatibility fixture. Rewrite tests to prove canonical behavior without embedding retired identity strings.

Treat issuectl as a generator owner: canonical `/issue`, `/issue-intake`, and related generated skill/template output must use only Taskfleet naming. Regenerate all dogfood copies through issuectl's supported generation path and update integrity hashes/snapshots. Validate a fresh disposable repository initialized and skill-installed by the candidate binary; its tracked/current generated surfaces must contain only canonical Taskfleet references.

Do not mutate the installed issuectl, deploy machines, publish a release, or edit another repository from the worker. Keep immutable Git history and already-published artifacts untouched.

## Acceptance Criteria

- [x] Case-insensitive tracked path/content scans find zero retired product, command, environment-prefix, package, protocol, or repository identities.
- [x] Canonical generator templates and all dogfood copies are byte/hash coherent.
- [x] A disposable fresh init/install exercise emits Taskfleet-only skills and leaves no residue.
- [x] Full repository gate passes.
- [x] The change is ready for a normal issuectl release and Homebase deployment by the conductor.
