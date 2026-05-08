---
created: 2026-05-08
updated: 2026-05-08
type: chore
reporter: jari
status: in-progress
priority: normal
epic: exorbitantly-ill-apples
---

# do_new_locked: return typed error instead of stringly-typed anyhow

_Source: src/main.rs / src/mutate.rs_

## Description

mutate::new_issue translates do_new_locked errors back into typed MutateError variants by string-matching the formatted anyhow message ("schema:" prefix, "already"/"exists" substrings). Brittle to message-format drift in do_new_locked: a wording change silently downgrades errors to MutateError::Validation, which the API surfaces as a wrong HTTP status / wrong frontend error message. Fix: return a typed enum (e.g. enum DoNewError { Schema, Conflict, Io, ... }) from do_new_locked and have mutate::new_issue map variant-to-MutateError directly. Also touches CLI cmd_new which currently uses anyhow's Display.
