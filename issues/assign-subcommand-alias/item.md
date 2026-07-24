---
created: 2026-07-24
updated: 2026-07-24
type: feature
status: open
priority: normal
labels: [cli, ux]
---

## Description

When reassigning an issue, the natural first guess is `issuectl assign <slug> <user>` — but that subcommand does not exist:

```
$ issuectl assign s2-canvas-lti-passback alisa
error: unrecognized subcommand 'assign'
```

The working paths are:

- `issuectl set <slug> --assignee <user>`
- `issuectl update <slug> --assignee <user>`

Both work, but `assign` is the verb people reach for first (git/GitHub-adjacent muscle memory: "assign this to X"). A thin alias would remove a discoverability papercut.

## Observed vs expected

- **Observed:** `assign` is an unrecognized subcommand; the user has to fall back to guessing `set` / `update`.
- **Expected:** `issuectl assign <slug> <user>` sets `assignee: <user>`, equivalent to `set <slug> --assignee <user>`. Optionally `issuectl assign <slug> --clear` to unassign (mirrors `set --clear`).

## Suggested scope

- Add `assign <slug> <user>` as a subcommand that routes through the existing typed `set --assignee` path — pure convenience wrapper, no new storage semantics.
- Keep validation/idempotency identical to `set`.

## Context

Surfaced 2026-07-24 during 3dbear-monorepo S2 canvas-LTI work (reassigning `s2-canvas-lti-passback` from jussi → alisa). Low urgency — `set` / `update` already cover the functionality; this is UX polish.
