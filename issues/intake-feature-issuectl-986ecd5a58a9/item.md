---
created: 2026-08-15
updated: 2026-08-15
type: feature
reporter: jari
status: open
priority: normal
labels:
- via:agent-intakectl-conductor
- needs-triage
---

# issuectl label: accept --add/--remove flag aliases (canonical skills us…

## Description

issuectl label: accept --add/--remove flag aliases (canonical skills use them; CLI only takes positional OP)

## Observed
`issuectl label` accepts ONLY the positional form:

    issuectl label <SLUG> <OP> <LABEL>      # OP ∈ {add, remove}

But the canonical Claude Code skills that manage issue lifecycle — `stint-handoff`,
`wrap-up` (Step 1 fold + Step 7), and `triage-bugs` — all instruct agents to use a
FLAG form:

    issuectl label <slug> --remove needs-triage
    issuectl label <slug> --remove needs-triage --add deferred

Running the flag form fails with a usage error:

    $ issuectl label intake-feature-... --remove needs-triage
    Usage: issuectl label <SLUG> <OP> <LABEL>

so an agent following the documented skill verbatim hits a hard error and has to
discover the positional form by reading `--help`. This bit a live stint-handoff on
2026-08-15 (intakectl repo).

## Expected
Either:
- `issuectl label` also accepts `--add <LABEL>` / `--remove <LABEL>` flag aliases
  (preferred — one CLI change fixes ≥3 shared skills at once and matches how the
  ecosystem already invokes it; both can be repeatable/combinable), OR
- if positional-only is the intended contract, that is a doc bug in the homebase
  skills instead — but the mismatch should be resolved in ONE place.

## Impact
Low severity (workaround: use positional `add`/`remove`), but it makes the canonical,
documented lifecycle commands in the core stint/wrap-up/triage skills fail on first
use for every agent that trusts them. Recommend the flag-alias route so the skills'
existing syntax becomes correct.
