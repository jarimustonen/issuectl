---
created: 2026-08-17
updated: 2026-08-20
type: feature
reporter: jari
status: obsolete
priority: normal
labels:
- via:agent-homebase-wrapup
closed: 2026-08-17
---

# collision frontmatter field is documented but not implemented in update

## Description

collision frontmatter field is documented but not implemented in update

## Observed

`.issuectl/AGENTS.md` documents `collision` as an optional list field (line 76):

    - `collision` (optional, list)

But there is no way to set it:

- `issuectl update <slug> --collision <path>` — no such flag (`update --help` offers only
  `--lane` and `--lane-seq` among the scheduling fields).
- `issuectl set <slug> collision <path>` — rejected, because `set` routes non-built-in keys
  through `custom_fields` and `collision` is not schema-declared in `.issuectl/.schema.yaml`.
- No issue in the orchestratectl repo uses the field (`grep -rl '^collision:' issues/*/item.md`
  returns nothing), so it appears to be documented-but-never-shipped.

## Expected

Either a `--collision` flag on `issuectl update` (list-valued, additive like the label ops),
or removal of the field from the documented schema if it was abandoned.

## Why it matters

The execution-DAG convention that the stint skills run on uses `collision:` as its named
mechanism for an issue that touches a *second* lane's hot files — a spawn-time exclusion
marker. It is the difference between "these two lanes are parallel-safe" and "these two look
parallel but will collide".

Concretely, in orchestratectl today: `add-configurable-agent` is laned `surface` (config
surface) but also touches `harness::select` and the run-create path, which is `lifecycle`
territory. That is exactly a `collision:` case, and running the two in parallel is the shape
that has broken integrated `main` twice in that repo. With no way to set the field, the
warning had to go into the issue body as prose — which the DAG renderer cannot see and a
scheduling agent may not read.

So the gap is not cosmetic: the DAG's own collision-avoidance mechanism is unavailable, and
the fallback is unstructured text.

## Minor, same area

`issuectl set <slug> lane <name>` fails with `validation: custom field "lane" is built-in:
use \`update --lane <name>\` / \`--no-lane\``. The error is correct and the suggested fix is
right there, so this is low value — but the built-in-vs-custom split is only discoverable by
guessing wrong first. Mentioned for context, not worth its own issue.

## Environment

issuectl as installed on macOS arm64, 2026-08-17.

## Comments

### 2026-08-17T17:13:03Z · @agent-stint

Triage: obsolete against issuectl 0.14.x — update has had --add-collision / --remove-collision (repeatable, additive+removing list ops) since the lane-fields work; create mirrors them at creation time. The reporter probed --collision and 'set collision', which indeed do not exist; the flag spelling is add/remove. The .issuectl/AGENTS.md in the reporting repo that documents the field is repo-authored, not issuectl's scaffold (issuectl's agents.rs scaffold does not mention collision). No code change needed.
