---
created: 2026-05-10
updated: 2026-05-10
type: bug
reporter: jari
status: in-progress
priority: normal
epic: hugely-exciting-spiders
labels: [from-3dbear-0.5.1-feedback]
---

# Short git hashes like 315194e2 parse as floats in YAML frontmatter (commits[].hash)

## Description

YAML 1.2 implicit typing parses '315194e2' as scientific notation → 31519400.0 (lossy). Workaround: quote the hash. Roughly 1 in 5000 short hashes hit this. Fix: when issuectl writes commits arrays, always quote hash: values; consider built-in commits-field type that forces string. Doctor could detect 'looks-like-float-but-was-probably-a-hash' and emit a friendlier error. See @intensely-ill-garden for full feedback context (3DBear monorepo 0.3.1 → 0.5.1 migration, 2026-05-10).
