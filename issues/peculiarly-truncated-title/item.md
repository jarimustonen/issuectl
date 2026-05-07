---
created: 2026-05-07
updated: 2026-05-07
type: bug
status: fixed
priority: low
reporter: jari
assignee: jari
epic: exorbitantly-ill-apples
labels: [cli, display]
closed: 2026-05-07
commits:
- hash: 6549ce6
  summary: fix parser shadowing that dropped leading E in titles
---

# `issuectl ls` drops first character of issue title in CLI display

## Description

In `issuectl ls` (and likely any other place that derives a one-line
title from the H1 in `item.md`), the first character of the title is
silently dropped in the rendered output. The on-disk file is
correct.

## Reproduction

Observed on an issue whose body starts with:

```
# Esimiehen ennakkolupa-flow
```

`issuectl ls` rendered the title as `simiehen ennakkolupa-flow` —
the leading `E` is gone. The file itself parses fine and the
heading is intact.

## Expected

Full H1 text shown verbatim.

## Likely cause

A title parser somewhere (probably in the listing render path,
not in the storage layer — only display is wrong) strips one
character too many. Candidate suspects:

- An off-by-one in code that skips the `# ` prefix and accidentally
  also skips the next character.
- A heading-style normalizer that strips a leading whitespace /
  BOM / ATX marker too aggressively.
- Unicode-related (the example starts with `E` — a plain ASCII
  letter — so probably not encoding-related, but worth checking
  with non-ASCII first characters too).

Fix is almost certainly local to the title-extraction helper.

## Quick test

```
echo -e "---\ntype: feature\nstatus: open\n---\n\n# AAA test title\n" \
  > issues/test-slug/item.md
issuectl ls | grep -i 'test title'
```

Expected: shows `AAA test title`. Actual (with bug): shows
`AA test title`.

## Scope

Cosmetic-only — does not affect data integrity, sync, or any
write path. Safe to fix in v0.5.0 alongside the writable kanban
work since the bug surfaces frequently in the CLI when reviewing
the board state.
