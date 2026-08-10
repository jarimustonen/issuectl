---
created: 2026-08-10
updated: 2026-08-10
type: feature
status: in-progress
priority: normal
commits:
- hash: pending
  summary: file feature
---

# issuectl new: derive default slug from title instead of random words

## Description


## Problem

`issuectl new "<title>"` without `--slug` mints a random `intensifier-adjective-noun` slug
(e.g. `immensely-cooperative-measure`, `markedly-chunky-appliance`). An agent filing several
worker-review follow-ups in a batch routinely forgets `--slug` and then has to `issuectl rename`
each one to something descriptive — 4 renames in a single ossctl stint (#14).

## Idea

Default the slug to a **title-derived 2–3 word kebab** (the same shape the `--slug` help already
recommends: "descriptive 2-3 word kebab-case slug derived from the title"), instead of random words.
Keep the random `intensifier-adjective-noun` form as an explicit opt-in / fallback for when the title
would leak sensitive data or yields no sensible slug. `--slug` stays authoritative when passed.

## Considerations

- Collision handling: if the derived slug already exists, disambiguate (numeric suffix) rather than
  silently colliding — the random form avoids collisions by construction, so the derived-default path
  needs its own dedupe.
- Sensitive titles: the current help notes the random fallback is "only [for] when no obvious short
  slug exists or the title would leak sensitive data." A title-derived default should be skippable
  (e.g. `--slug-random` or `--no-derive-slug`) for those cases.
- Stop-word/length trimming so a long title yields a clean 2–3 word slug.

## Source

Recurring friction across ossctl stint #14 (filing worker `/llm-review` follow-ups). Low-stakes UX.
