---
created: 2026-07-24
updated: 2026-07-24
type: feature
status: open
priority: normal
related: ['@assign-subcommand-alias']
labels: [cli, ux]
---

## Description

Creating an issue non-interactively required three guess-and-correct rounds because the guessed verb/flag shapes did not match the actual CLI, and the errors did not point at the right one. All three are the same class of friction: **the natural first guess is wrong, and the error message doesn't route the user to the working form.**

Observed this session (2026-07-24), in order:

1. **`issuectl create …`** → `error: unrecognized subcommand 'create'`. The tip did suggest `ready` / `rename` (unrelated); it did **not** suggest `new`, which is the actual create verb. `create` is the near-universal verb for "make a new thing" (git branch, gh issue create, kubectl create, docker create), so it's the overwhelming first guess.

2. **`issuectl new --body …`** → flag rejected; the create-body flag is `--description`, not `--body`. Meanwhile the *replace-whole-body* operation lives under a different subcommand (`issuectl body set`), which uses `--from-file` / `--stdin`. So "body" means two different things depending on entry point, and neither is named `--body` on `new`.

3. **`issuectl body <slug> --file …`** → `error: unrecognized subcommand '<slug>'`. `body` is a subcommand *group* whose actual op is `body set <slug> --from-file`. The error treated the slug as a subcommand rather than hinting "did you mean `body set`?".

## Observed vs expected

- **Observed:** three failed invocations before landing on `issuectl new --description …` + `issuectl body set <slug> --from-file …`. The `unrecognized subcommand` errors either suggested unrelated commands (`ready`/`rename`) or none, and never named the working alternative.
- **Expected:** either the guessed forms work as aliases, or the error explicitly routes to the right one (e.g. `create` → "did you mean `new`?"; `body <slug>` → "did you mean `body set <slug>`?").

## Suggested scope (pick some / all)

- **Alias `create` → `new`.** Cheapest single win; `create` is the dominant muscle-memory verb.
- **Improve near-miss suggestions.** When an unknown subcommand is a known-alias-target (or when a slug is passed where a sub-subcommand is expected, as in `body <slug>`), suggest the correct form by name. clap supports `.alias()` and custom error hints.
- **Consider accepting `--body` as an alias for `--description` on `new`** (and/or reject with a hint pointing at `--description` + `body set`).

## Why it matters

Non-interactive/agent use (Claude Code, scripts, CI) can't discover verbs interactively — it guesses from convention, and every wrong guess is a wasted round-trip. Convention-aligned aliases + routing hints make the CLI self-correcting for exactly the automation that uses it most.

## Context

Surfaced 2026-07-24 filing a different issuectl feature request (`@assign-subcommand-alias`) from 3dbear-monorepo S2 canvas-LTI work. Sibling to that issue — both are "the guessed shape doesn't match, and the error doesn't route." Low urgency; the working forms exist.
