---
name: triage-bugs
description: "DEPRECATED alias for /issue-intake, kept so the old name still works. Prints a one-line rename notice and forwards to /issue-intake. Prefer /issue-intake directly; load this only when the user types /triage-bugs."
argument-hint: (deprecated — passes through to /issue-intake)
---

# triage-bugs — renamed to /issue-intake

This skill used to triage bot-filed bugs by their `via:<channel>` labels. That
job was folded into the standard intake flow (`docs/design/intake-flow.md`),
where the queue is a first-class `untriaged` status covering bugs and non-bugs
from any provenance. `/issue-intake` is that skill. Nothing of the old
behaviour lives here any more; this file exists only so a habit of typing the
old name does not strand the user, and it will be removed once the habit has
faded.

Tell the user in one line that `/triage-bugs` is now `/issue-intake` and that
you are running that instead. The line is the mechanism by which the habit
gets corrected, so it should be said every time, and once is enough. Then run
`/issue-intake` with the arguments below unchanged. Its flags (`--no-pull`,
`--state`, `--type`, `--provenance`) are documented there, and it owns the
whole behaviour, including the read-only "brief, then stop" contract, so any
detail you would be tempted to restate here is better read from that skill.

Arguments: `$ARGUMENTS`
