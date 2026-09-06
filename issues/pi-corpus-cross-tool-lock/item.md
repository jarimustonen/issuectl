---
created: 2026-08-12
updated: 2026-08-14
type: improvement
status: wontfix
priority: low
related: ['@pi-manifest-locking']
closed: 2026-08-14
closed_by: jari
---

# pi-corpus: issuectl and taskfleet hold separate locks — no cross-tool serialization on shared skill dirs

_Source: crates/issuectl-core/src/skill.rs_

## Description

Spin-off from /llm-review of pi-manifest-locking (consensus, design).

The provenance manifest is tool-namespaced (`.issuectl-manifest.json`) and the new lock lives at `<pi_root>/.issuectl/write.lock`. If the sibling `taskfleet` corpus writer uses its own lock path, the two tools do NOT serialize against each other: they can concurrently create/overwrite/delete the same-named skill directory under the shared corpus. Same-tool operations are now safe; cross-tool ones are not.

Fix direction: agree a corpus-wide lock protocol both writers honour for skill-file + manifest mutation (e.g. a well-known `<pi_root>/.corpus.lock`), rather than per-tool locks over shared data files. Cross-repo/cross-tool design; out of scope for the single-tool locking change.

## Resolution

### 2026-08-14T03:41:55Z · @jari

Wontfix: cross-tool concurrent-write serialization; only bites if issuectl+taskfleet write the same corpus dir at the same instant. Premature until that's a real scenario.
