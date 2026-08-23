---
created: 2026-08-21
updated: 2026-08-23
type: bug
status: wontfix
priority: normal
closed: 2026-08-23
closed_by: jari
---

# intake file generates meaningless slugs where create derives them from the title

## Description

## Observed

Issues filed by agents through `issuectl intake file` get meaningless auto-generated
adjective-noun slugs, while `issuectl create` derives a readable slug from the title.

Two examples from a single ossctl stint, both substantive bugs filed by autonomous workers:

| generated slug | actual title |
|---|---|
| `nominally-disastrous-destruction` | release verify infers delivery from the destination, not the delegated run: pending and failed are indistinguishable |
| `virtually-bustling-horse` | Release bump can leave inherited exact workspace pins stale |

Neither slug carries any signal about its issue. Both had to be renamed by hand
(`issuectl rename nominally-disastrous-destruction verify-delegated-run-state`,
`issuectl rename virtually-bustling-horse bump-inherited-workspace-pins`) before they were
readable in `issuectl dag` output or in a handoff narrative.

## Why this matters beyond aesthetics

The slug is the issue's identity everywhere it is cheap to look: `issuectl dag` lane listings,
`blocked_by` / `related` frontmatter, `@slug` body mentions, commit-message trailers, and
handoff documents. A scheduling DAG whose rows read `nominally-disastrous-destruction` and
`virtually-bustling-horse` cannot be scanned — an orchestrator has to open each issue to learn
what its own plan contains.

It is also self-inflicted asymmetry: `issuectl create` already does this correctly, and even
documents the rule in its own warning output:

```
warning: derived base `assess-models-wedges` differs from title slug
`assess-models-wedges-workers-for-hours-on-an-unbounded-find-move-the-corpus-to-haapa`:
derived slugs retain 2–3 significant words after dropping stop-words
```

So the title-derivation logic exists and is good; the intake path simply does not use it.

Note that not every intake-filed issue gets a random slug — a maintainer-filed intake in the
same repo arrived as `intake-bug-ossctl-d38ddf598fd5`, which is at least structured and
traceable even if not descriptive. So the behaviour appears to differ by filing path.

## Expected

`issuectl intake file` derives its slug from the title using the same rule as
`issuectl create` (2–3 significant words after dropping stop-words), falling back to a
structured `intake-bug-<repo>-<hash>` form only when no usable title is available. Random
adjective-noun pairs should not be a normal outcome for an issue that has a perfectly good
title.

## Close condition

Close as fixed when an agent-filed intake issue with a descriptive title lands with a slug
derived from that title. Close as wontfix if slug collision-avoidance across concurrent filers
provably requires an opaque identifier — in which case record that reasoning, and consider
whether the descriptive form can still be used as a prefix.

## Resolution

### 2026-08-23T18:34:24Z · @jari

By design: intake titles are untrusted and may contain customer names or secrets that must not enter persistent directory or automation identifiers. Reopen only for an explicit trusted-title mode with documented privacy guarantees.
