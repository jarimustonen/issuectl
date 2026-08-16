---
created: 2026-08-16
updated: 2026-08-16
type: bug
reporter: jari
status: open
priority: normal
labels:
- via:agent-aggountant-wrapup
lane: cli-fixes
lane_seq: 30
---

# issuectl new silently truncates the derived slug

## Description

issuectl new silently truncates the derived slug

`issuectl new` derived a shorter slug than the title implies and said nothing about it.

## Observed

    $ issuectl new vat-rs-module-split --type improvement --priority high --description '...'
    Created vat-rs-module: vat-rs-module-split
      /Users/jari/Sources/aggountant/issues/vat-rs-module/item.md

The title was `vat-rs-module-split`; the slug became `vat-rs-module` — the trailing `-split` was dropped. The output does show both (`Created <slug>: <title>`), but nothing marks the slug as *modified*, so it reads as normal output rather than "your identifier is not what you asked for".

This matters because the slug is the identifier every later command, cross-reference, and `blocked_by:` edge uses. I had already written `vat-rs-module-split` into a commit message and a handoff document before noticing, and had to correct them.

## Expected

Either keep the full slug, or warn when the derived slug differs from a straightforward slugification of the title — e.g.

    Created vat-rs-module (slug shortened from vat-rs-module-split): vat-rs-module-split

I could not tell from the output what rule shortened it (length cap? stop-word list? a collision-avoidance rule?), which is itself part of the problem — the behaviour is invisible and unexplained at the point of use.

Environment: macOS, issue repo at ~/Sources/aggountant, flat layout.
