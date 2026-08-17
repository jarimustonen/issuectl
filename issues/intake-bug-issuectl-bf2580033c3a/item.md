---
created: 2026-08-17
updated: 2026-08-17
type: bug
reporter: jari
status: open
priority: normal
labels:
- via:agent-homebase-wrapup
- needs-triage
---

# Personal-setup references leak into the public repo (fleet-apply, homeb…

## Description

Personal-setup references leak into the public repo (fleet-apply, homebase, hauis runner)

issuectl is a public OSS project and must carry zero references to the
maintainer's personal machine fleet or private repos. A sweep of the working
tree found five such references. None of them leak into the templates that
`issuectl skill install` writes into *other people's* repos (those were checked
and reference only shared CLIs — `/worktree-bug-analysis` is orchestratectl's
bundled skill, a legitimate cross-CLI dependency), so this is contained to the
repo's own source and docs.

## Observed

1. `crates/issuectl-core/src/skill.rs:305-306` — code comment:
   "the same way `/issue` does, so the fleet-apply hook distributes them to
   both fleets." `fleet-apply` and "both fleets" are the maintainer's private
   homebase infrastructure. (The `/issue` half is fine — that is issuectl's own
   skill.)

2. `crates/issuectl-core/src/skill.rs:335` — vendored-filter rationale:
   "matching homebase `dotfiles link`, which copies just the skill body into
   the pi corpus." Cites a private repo as the normative reason.

3. `AGENTS.md:77` — same "matching homebase `dotfiles link`" citation.

4. `AGENTS.md:486` — names the self-hosted runner as "the `hauis` runner".
   The runner override itself is a deliberate, maintainer-owned decision and
   stays; only the machine name should go.

5. `TODO.md:90` — duplicates the same `hauis` runner note.

## Expected

- (1) reworded generically, e.g. "so an installer hook can distribute them to
  every configured agent".
- (2) and (3) reworded to describe the behaviour without naming a private repo,
  e.g. "matching a dotfile linker that copies only the skill body into the pi
  corpus".
- (4) "a maintainer-operated self-hosted macOS ARM64 runner" instead of the
  machine name. Note the rationale must stay in AGENTS.md rather than move into
  `dist-workspace.toml`: `ossctl dist generate` rewrites that file, so a comment
  there is destroyed exactly when the warning is needed.
- (5) dropped as a duplicate of (4).
