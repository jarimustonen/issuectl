---
created: 2026-08-16
updated: 2026-08-16
type: bug
status: open
priority: normal
lane: cli-fixes
lane_seq: 20
---

# create --help text contradicts actual default slug behaviour

## Description

`issuectl create --help` describes the default-slug behaviour incorrectly. It
claims the random `intensifier-adjective-noun` slug is what you get when
`--slug` is omitted; in fact the default is **title-derived**, and random is
only the opt-in (`--slug-random`) or the fallback when the title yields nothing
usable.

**Observed** (help text, 0.13.0):

> Create a new issue or epic. Pass `--slug <descriptive-2-3-word-kebab>` derived
> from the title; a random `intensifier-adjective-noun` slug is the fallback when
> `--slug` is omitted

**Actual behaviour** (verified 2026-08-16 in a fresh repo, built from `main` at
0.13.0):

```
$ issuectl --json create --type bug "Fix broken login redirect loop"
→ slug: fix-broken-login
```

Title-derived, not random.

**Expected:** the help text should match `AGENTS.md`'s documented convention
("The CLI default slug is title-derived; random is the opt-in/fallback"), i.e.
say that omitting `--slug` derives a 2–3 word kebab slug from the title, that
`--slug-random` opts into the random form, and that random is also the automatic
fallback when the title yields no sensible slug.

## Why this matters more than a typo

`--help --json` (canon §14) shipped in 0.12.0 and is now the sanctioned way for
an agent to discover the CLI surface without scraping prose. A wrong help string
is therefore no longer just cosmetic — it propagates directly into consumer-side
agents' model of the tool. Worth checking the other subcommand help strings for
the same kind of drift while in there.

## Scope

- Fix the `create` help text (`crates/issuectl/src/main.rs` clap attributes).
- Sweep sibling help strings for other claims that no longer match behaviour.
- Add a test if one can meaningfully pin help/behaviour agreement.
