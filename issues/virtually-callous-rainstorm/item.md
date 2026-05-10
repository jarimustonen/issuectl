---
created: 2026-05-10
updated: 2026-05-10
type: bug
reporter: jari
status: in-progress
priority: high
epic: hugely-exciting-spiders
labels: [from-3dbear-0.5.1-feedback]
---

# doctor flags YAML inside fenced code blocks in issue body as 'unknown frontmatter keys'

## Description

Frontmatter parsing leaks past --- boundaries and treats indented YAML inside body code blocks (```yaml ... ```) as frontmatter. Also catches accidental 'word:' line-wraps in prose (e.g. 'launched: the OIDC handshake completes...'). Fix: restrict frontmatter parsing strictly to the content between the first --- pair; ignore body and code blocks. See @intensely-ill-garden for full feedback context (3DBear monorepo 0.3.1 → 0.5.1 migration, 2026-05-10).
