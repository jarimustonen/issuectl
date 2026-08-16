---
created: 2026-08-16
updated: 2026-08-16
type: task
status: open
priority: high
---

# Audit: no user-specific facts in a public artifact

## Description

## Description

This repository is **public** and distributed. Audit it for user-specific facts that must not
ship in a public artifact, and move any that exist into user configuration.

### The rule

Maintainer rule, 2026-08-16: *a public repo must not reference user-specific things at all;
user-specific things belong in user config.*

A publicly-distributed artifact MUST NOT contain personal account handles, private
repo/project names, personal filesystem-layout conventions, hostnames, internal URLs, or
org-internal identifiers — not in source, **not in built-in defaults**, not in generated
scaffold/template output, not in installed skill content, not in docs, not in tests or
fixtures.

**Key point, and the one that caused the original defect: overridability does not launder a
user-specific default.** "Every value is configurable" is not a defence — an unset default is
still whatever ships in the package. The correct built-in default is neutral/absent, with an
actionable error naming the config key or env var to set. Never a silent guess at someone's
environment.

### Why this issue exists

`project-canon` — the family's own conformance tool — shipped these to crates.io in `0.1.1`
and `0.2.0`:

```rust
gh_account: "jarimustonen".to_string(),
repo_root:  "~/Sources".to_string(),
const DEFAULT_FAMILY_TOOLS: [&str; 7] = [
    "issuectl", "orchestratectl", "crmctl", "tilictl", "ossctl", "intakectl", "glasspad",
];
```

Three of those seven are **private** repositories, so a public crate disclosed the names of
private projects. The defect survived a design pass that explicitly considered portability,
because the reasoning stopped at "it's overridable".

Every public tool in the family is plausibly exposed to the same class of defect, so each is
being audited rather than assumed clean.

## What to check

Grep the whole repo — source, defaults, templates, generated output, docs, README, tests,
golden fixtures, skill content — for:

- the maintainer's account handle / name / email
- names of **private** family repos: `crmctl`, `tilictl`, `intakectl`, `aggountant`
  (public siblings are fine to reference where genuinely relevant, e.g. a real dependency)
- personal path conventions (`~/Sources`, `/Users/<name>`, personal machine hostnames)
- internal URLs, internal service names, org-internal identifiers
- any built-in default that encodes one person's environment rather than a neutral value

## Acceptance

- No user-specific value anywhere in the shipped artifact (per the list above).
- Any environment-specific value the tool genuinely needs is read from user config, with a
  neutral built-in default and an actionable error when a required value is unset.
- Fixtures, examples, and docs use obviously fictional values.
- The maintainer's own setup still works, expressed through user config outside the repo.
- **If the audit finds nothing, close the issue saying so** — a recorded clean result is the
  point; it is what makes the family-wide sweep meaningful.

## Comments

Filed 2026-08-16 from `project-canon` as part of a family-wide sweep after the leak above was
found. Companion work in `project-canon`: `portable-neutral-defaults` (the concrete cleanup)
and `canon-no-user-specifics` (promoting this rule to a canon section with a mechanical
`doctor` check, so it is enforced rather than remembered). Once that check ships, this audit
becomes automated — this issue is the one-time manual pass.
