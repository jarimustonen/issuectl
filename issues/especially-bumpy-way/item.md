---
created: 2026-05-08
updated: 2026-05-08
type: task
reporter: jari
status: open
priority: normal
---

# deser_epic must error on malformed shape

## Description

`parser::deser_epic` returns `Ok(None)` for any non-string,
non-number `epic:` value (e.g. `epic: [1, 2]`, `epic: { a: 1 }`).
The malformed value is silently dropped from `Issue::epic`,
produces no warning, and never reaches `canonical_hash` —
optimistic concurrency has a blind spot for it. The raw mapping
in `write::ItemFile` still preserves the bad value on disk, so
nothing is physically lost; but a writer who edits *only* `epic:`
externally (with a malformed value) cannot trigger a 409 on a
stale `expected_version`.

Per project policy: AI agents must get strict feedback on every
malformed input. `deser_epic` should return
`Err(de::Error::custom(...))` so the whole-frontmatter parse
fails through the existing fall-back and the file flows to
`MutateError::Corrupt`. This is consistent with the parser's
treatment of every other field shape.

Migration concern: existing repos may have legacy malformed
`epic:` values that were tolerated. `issuectl doctor --fix`
already migrates legacy numeric epic refs; extend it to also
detect/normalise non-scalar shapes so the strict parse doesn't
brick affected issues.

Spun off from @painfully-endurable-steel review.
