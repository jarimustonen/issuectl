---
created: 2026-08-18
updated: 2026-08-19
type: feature
status: open
priority: normal
lane: verb-surface
lane_seq: 4
collision: [crates/issuectl/src/cmd/write.rs, templates]
---

# No CLI way to retitle an issue; body set silently replaces the title H1

## Description

## Observed

There is no command that changes an issue's title. `rename` changes the slug only; `update` has no `--title`; and `issuectl body set` REPLACES the entire markdown body including the `# <title>` H1 — a body file without a heading silently strips the title (observed 2026-08-17 while extending @split-main-rs: the title vanished and had to be restored by hand with a second `body set`).

## Expected

- `issuectl update <slug> --title "..."` rewrites the H1 through the locked mutation path (canonical_hash impact: title is already part of the hash via the body, so this is a normal content edit, not a schema change).
- `body set` either preserves the existing H1 when the incoming body lacks one, or at minimum warns that the title is being removed/changed.

## Notes

Fits the ADR 0004 update-surface work; same echo/write surface as @update-canonical-forms.
