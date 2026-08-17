---
created: 2026-08-16
updated: 2026-08-17
type: feature
reporter: jari
status: done
priority: normal
labels:
- via:agent-homebase-wrapup
lane: help-docs
lane_seq: 10
collision: [crates/issuectl/src/cmd/write.rs]
commits:
- hash: 48f4881
  summary: document body_ops patch shapes in apply help
closed: 2026-08-17
---

# apply --help does not document body_ops operation shapes

## Description

apply --help does not document body_ops operation shapes

`issuectl apply --help` names the `body_ops` operations but not their shape, so the first
attempt at a patch file fails.

Observed (2026-08-16):

    $ issuectl apply --help
    ... The file declares `slug:` plus any combination of built-in fields, `custom_fields:`,
    label/related list ops, commits, and `body_ops:` (toggle_checkbox / append_note) ...

Reading that, the natural patch is a plain string:

    slug: canon-no-user-specifics
    body_ops:
      - append_note: |
          Some multi-line note text.

    $ issuectl apply /tmp/patch.yaml
    0: cannot parse patch fields
    1: invalid type: string "Some multi-line note text.", expected struct AppendNoteOp

The error itself is good — it names the expected type. What is missing is any way to learn the
shape *before* hitting it: `--help` lists the op names only, and `AppendNoteOp`'s fields are not
discoverable from the CLI.

Expected: `apply --help` (or `--help --json`, per canon §14) shows a minimal example patch
covering each `body_ops` operation with its actual fields, e.g.

    body_ops:
      - append_note:
          body: "…"        # whatever the real field(s) are
      - toggle_checkbox:
          ...

Alternatively, accept the bare-string form for `append_note` as a convenience shorthand, since
a note is a single body of text and the string form is what the help text implies.

Low severity — one round-trip, and `issuectl note --body-file` was a fine substitute. Filing it
because `apply` is the transactional path an agent is steered toward for multi-field edits, and
its patch format is currently only learnable by trial and error.
