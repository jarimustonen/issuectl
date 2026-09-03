---
created: 2026-05-06
updated: 2026-05-31
type: feature
status: done
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [validation, workflow]
closed: 2026-05-31
---

# Markdown DoD validation: parse acceptance criteria + block done transition until satisfied

_Source: src/body.rs (new — markdown section parser), src/cli/{ready,update,close}.rs_

## Description

Standardize body sections: '## Acceptance Criteria', '## Tests Run', '## Implementation Notes'. Parse markdown task lists ('- [ ]' / '- [x]'). Add 'issuectl ready <slug>' that reports completion status. By default warn on '→ done' transitions with unchecked acceptance criteria; with strict mode in schema config, block. Zero schema-frontmatter changes — markdown stays human-readable. Gives agents a verifiable completion contract; small teams get a checklist that doesn't rot in YAML.
