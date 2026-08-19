---
name: triage-bugs
description: "DEPRECATED — renamed to /issue-intake. Read-only intake processing for filed bug reports and feature requests. This is now a thin alias that prints a rename notice and delegates to /issue-intake, kept so existing muscle memory and `/stint` call sites keep working during the deprecation window. Use /issue-intake directly. Composes /issue-intake; drives /worktree-bug-analysis."
argument-hint: (deprecated — passes through to /issue-intake)
---

# triage-bugs — DEPRECATED, renamed → /issue-intake

This skill was **renamed to `/issue-intake`** when ad-hoc channel-labelled bug
triage folded into the standard intake flow (`docs/design/intake-flow.md`).
`/issue-intake` does the same job — read the intake queue, drive
`/worktree-bug-analysis` on unclear items, brief the user in product-owner
language, and stop — but against the first-class `untriaged` intake state and
across **both** bug reports and feature requests, regardless of provenance.

This alias exists only for the deprecation window so old habits and `/stint` call
sites don't break. It will be removed.

## What to do

1. **Tell the user, once:** `/triage-bugs` has been renamed to `/issue-intake`;
   running that instead. (Keep it to one line — do not lecture.)
2. **Delegate to `/issue-intake`**, forwarding whatever arguments were passed
   (`--no-pull`, `--state …`, `--type …`, `--provenance …` all pass through
   unchanged). `/issue-intake` owns the entire behaviour; this file adds nothing
   of its own beyond the notice and the hand-off.

Do not reimplement any triage logic here. Everything — the queue read, the
read-only analysis engine, the PO briefing, the "present then STOP" contract, the
`<!-- intake-return -->` block — lives in `/issue-intake`.

## Arguments

Argument: `$ARGUMENTS` (forwarded verbatim to `/issue-intake`).
