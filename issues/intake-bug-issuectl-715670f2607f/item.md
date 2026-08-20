---
created: 2026-08-16
updated: 2026-08-20
type: bug
reporter: jari
status: fixed
priority: normal
closed: 2026-08-16
lane: cli-fixes
lane_seq: 50
provenance: agent-aggountant-wrapup
---

# note rejects --comment although its own help says it mirrors close --co…

## Description

note rejects --comment although its own help says it mirrors close --comment

`issuectl note` accepts `--message` / `--body` but not `--comment`, while `issuectl close` accepts `--comment` / `--note`. The help text for `note` explicitly promises the opposite.

## Observed

    $ issuectl note use-case-onboarding --as claude-triage --comment "Merged from ..."
    For more information, try '--help'.

(exit non-zero, usage error — and note the error output is just that one line, with no statement of *which* argument was wrong; I had to open `--help` to find out.)

Working form:

    $ issuectl note use-case-onboarding --as claude-triage --message "Merged from ..."

## Why this reads as a defect rather than a preference

`issuectl note --help` says, of `--message`:

    Note text as a flag; `--body` is an alias. Mirrors `close --comment` and `new --body`,
    so the whole family shares one vocabulary.

The stated intent is one shared vocabulary across the family, but the family's two writing verbs take disjoint flag names for the same thing: `close --comment` vs `note --message`. Having read that sentence, `--comment` is the natural thing to reach for on `note`.

## Expected

`note` accepts `--comment` as an alias for `--message` (and/or `close` accepts `--message`), so the documented shared vocabulary is real. Failing that, drop the "shares one vocabulary" claim from the help.

Minor, but it cost a failed batch of three note calls in one run.

## Comments

### 2026-08-16T19:49:50Z · @issuectl

Added note --comment and close --message aliases, plus informative bad-flag and shared-input regression coverage.
