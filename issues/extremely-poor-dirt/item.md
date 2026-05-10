---
created: 2026-05-10
updated: 2026-05-10
type: improvement
reporter: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [from-3dbear-0.5.1-feedback]
---

# Schema: closed: is conditionally required for closing statuses but not declared as such

## Description

Schema declares closed: required: false. But lifecycle classification imposes a conditional requirement (closed-status implies closed: must be set), which is not expressed in schema. Fix: add a 'required_when' style constraint, e.g. closed: { required_when: 'status is closing' }; or document in auto-generated schema comments. See @intensely-ill-garden for full feedback context (3DBear monorepo 0.3.1 → 0.5.1 migration, 2026-05-10).
