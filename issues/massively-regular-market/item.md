---
created: 2026-05-09
updated: 2026-05-09
type: feature
status: done
priority: normal
reporter: jari
assignee: jari
epic: exorbitantly-ill-apples
labels: [agent-friendly, cli]
related: ['@peculiarly-political-interest']
closed: 2026-05-09
---

# apply transactional body ops (note/check) in single flock

_Source: src/mutate.rs (UpdateIssueRequest), src/main.rs (cmd_apply)_

## Description

Today `issuectl apply <patch.yaml>` applies frontmatter-only operations
atomically under one flock — built-in fields, `custom_fields`,
add/remove labels, add/remove related, add commits. It cannot include
`note` (append-only body block) or `check` (toggle markdown checkbox)
in the same transaction. An agent that wants

  "set status testing AND check 'tests passing' AND note 'CI green'"

has to issue three separate writes, each with its own version
roundtrip, breaking optimistic-concurrency under contention.

This was raised in @peculiarly-political-interest's LLM review
(Anthropic's TOP-3) as the architectural follow-up — not a bug in the
v0.5.0 wave-C agent-mutation-cli landing, but a missing capability
that the writable-kanban epic should close before v0.5.0 ships.

## Proposed shape

Add a body-ops vector to `UpdateIssueRequest` so the same envelope
that drives `update_issue` also drives body mutations:

```rust
pub enum BodyOp {
    ToggleCheckbox { task: String },
    AppendNote { section: NoteSection, author: String, message: String },
}

pub struct UpdateIssueRequest {
    // ... existing fields ...
    pub body_ops: Vec<BodyOp>,
}
```

`cmd_check` and `cmd_note` keep their thin handler shape but build a
single-element `body_ops` request and route through `update_issue`
instead of separate `toggle_checkbox` / `note_issue` entry points.
That collapses three "almost-the-same" pub fns into one, kills the
schema-validation drift between body verbs (sister issue —
@peculiarly-political-interest review found `note_issue` and
`toggle_checkbox` skip schema validation that `update_body` does),
and lets `apply <patch.yaml>` declare:

```yaml
slug: my-issue
expected_version: sha256:...
status: testing
body_ops:
  - toggle_checkbox: "tests passing"
  - append_note:
      section: agent_runs
      author: ci-bot
      message: "all checks green"
```

… which the CLI applies under one flock, one schema-validation pass,
one canonical-hash bump, one publish.

## Out of scope

- Removing the standalone `cmd_check` / `cmd_note` handlers — they
  remain as thin sugar over the body_ops path.
- Web API parity — the existing PATCH endpoint can grow `body_ops`
  in the same shape, but server-side wiring is its own follow-up.
- Per-op partial-failure semantics (all-or-nothing under one flock
  is the only mode supported initially).

## Why now (v0.5.0 scope)

The agent-mutation-cli surface is the agent-facing contract for the
writable kanban; shipping v0.5.0 with `apply` that excludes the two
body operations agents most want to bundle leaves a hole the next
release would have to fill anyway. Better to land the unified
mutation envelope before agents calcify against the partial surface.

## Success criteria

- `apply` patch can declare `body_ops:` mixed with frontmatter
  fields; everything happens under one flock.
- `cmd_check` and `cmd_note` continue to work; CLI ergonomics
  unchanged.
- Schema validation runs once, after all body_ops + frontmatter
  patches apply.
- Existing tests pass; new tests cover mixed body_ops + frontmatter
  patches and rollback semantics.
