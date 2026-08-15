---
created: 2026-08-15
updated: 2026-08-15
type: feature
reporter: jari
status: done
priority: normal
lane: cli-fixes
lane_seq: 50
collision: [crates/issuectl/src/main.rs]
closed: 2026-08-15
---

# Add a 'comment' alias for 'note', and accept --message alongside the positional body

_Source: cli/note ergonomics_

## Description

**Friction (2026-08-15):** to add a comment to an issue I first tried `issuectl comment <slug> ...` (the natural guess) → `error: unrecognized subcommand 'comment'`. The real command is `note`, and its message is a **positional** argument (`issuectl note --as <author> <slug> <message>`) — I also tried `--note`/`--comment` flags first, which failed. Two dead ends before the right form.

**Inconsistency:** `issuectl close` already takes `--comment` (aliased `--note`) for its closing rationale, so the vocabulary is split: 'comment' on close, 'note' as a standalone verb with a positional body.

**Suggestions (either/both):**
1. Add `comment` as an alias of the `note` subcommand.
2. Let `note`/`comment` accept `--message`/`--body` (and `--body-file -`) in addition to the positional, matching `new --body-file` / `close --comment`.

Minor, but it's a common verb and the current split is a small trip hazard.
