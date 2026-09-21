---
created: 2026-09-21
updated: 2026-09-21
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:native-agent-host-wrapup-note-eof-20260921
---

# issuectl note leaves a blank line at EOF

## Description

issuectl note leaves a blank line at EOF

## Observed

Running `issuectl --json note <slug> --as pi --expected-version <version> '<message>'` appends the note but leaves an extra blank line at end of `issues/<slug>/item.md`.

`git diff --check` then reports:

```
issues/receive-designer-v1/item.md:48: new blank line at EOF.
```

The same warning occurred earlier after adding a note to `review-ui-v1`.

## Expected

The issue body should end with exactly one newline so an ordinary `issuectl note` result passes `git diff --check` without a follow-up `issuectl body set` normalization.

## Environment

- issuectl 0.18.4
- command used with `--json` and `--expected-version`
