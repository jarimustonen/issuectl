---
created: 2026-05-05
updated: 2026-05-05
type: feature
reporter: jari
assignee: jari
status: in-progress
priority: normal
slug: altogether-sassy-house
---

# Replace sequential issue numbers with random word slugs

_Source: src/slug.rs (new), src/cli/doctor.rs (new), migration of existing `issues/`_

## Problem

Sequential issue numbering (`#1`, `#2`, …) collides whenever two parallel worktrees or branches create issues independently. The current `renumber` subcommand exists exactly because of this — it is a band-aid, not a fix. We want a naming scheme that is collision-free by construction so that distributed work can land in any order without renumbering.

## Goal

Replace the integer issue identifier with a **random multi-word slug** (e.g. `quiet-brave-otter`). The slug is the canonical identifier; directories become `issues/open/<slug>/item.md`. References (`#NN`) are replaced by the slug form (`@quiet-brave-otter` or similar — final syntax decided during implementation).

Target structure: **adjective-adjective-noun** for ample entropy and readable slugs. With ~500 adjectives and ~1000 nouns we get ~250M combinations — well within birthday-paradox safety for any realistic issue count.

## Approach

### Phase 1 — Build the wordlists (one-shot, results checked in)

The wordlists are a static asset of the program. Build them once, commit the result, do not regenerate at runtime.

1. **Download three source corpora** into a scratch directory (e.g. `tools/wordlists/sources/`):
   - **EFF Long Wordlist** — 7776 mixed words (CC-BY 3.0). Source: https://www.eff.org/dice / mirrored in `bitwarden/clients` at `libs/common/src/platform/misc/wordlist.ts`.
   - **Moby (Docker) name generator** — ~108 adjectives + ~237 surnames (Apache 2.0). Source: `moby/moby` at `pkg/namesgenerator/names-generator.go`.
   - **`names` crate wordlists** — adjectives + nouns (MIT). Source: `fnichol/names` repo, `data/` directory.

2. **Merge all words into a single CSV** at `tools/wordlists/all-words.csv` with columns:
   ```
   word,sources
   abacus,eff
   admiring,moby;names
   ...
   ```
   Deduplicate by word (lowercase). Keep `sources` as a `;`-separated list for traceability.

3. **Classify in batches with an LLM agent.** Feed the CSV to an agent in batches of ~100 words. For each word the agent returns one or more part-of-speech tags from a fixed set:
   - `adjective`
   - `noun`
   - `verb`
   - `adverb`
   - `other` (or skip)

   A word may appear in multiple lists (e.g. `run` is both verb and noun, `quiet` is both adjective and verb). That is intentional — output is `word,classes` where `classes` is a `;`-separated list.

4. **Filter out unwanted words.** Drop offensive, hard-to-spell, or ambiguous entries. The agent should flag candidates; human reviews the flag list once.

5. **Emit final per-class lists** as Rust source files (one `const &[&str]` per class) under `src/slug/wordlists/`:
   - `adjectives.rs`
   - `nouns.rs`
   - `verbs.rs` (kept for future flexibility, may be unused initially)

   Each file starts with an attribution comment listing the three source licenses (Apache 2.0, MIT, CC-BY 3.0) and pointing to `NOTICE`.

6. **Add `NOTICE`** at the repo root with full attributions.

> **Note for `/worktree-merge` review:** the wordlists and their classifications are trusted to be correct on first pass. Reviews of worktrees that touch this issue do **not** need to inspect or second-guess the contents of `adjectives.rs`, `nouns.rs`, etc., nor the classification CSV. Treat them as a vendored asset.

### Phase 2 — Slug generation (`src/slug.rs`)

- `pub fn generate() -> String` → returns `adj-adj-noun` using `rand`.
- `pub fn is_valid(s: &str) -> bool` → loose validation (lowercase, kebab, 2–4 segments, all-alpha).
- Collision check at issue creation: if `issues/{open,closed}/<slug>/` exists, regenerate (loop with a small retry cap, e.g. 8). With 250M combinations a retry should essentially never fire.

### Phase 3 — Integrate into `issuectl`

- Replace integer `number` field in frontmatter with `slug` (string).
- Directory layout: `issues/open/<slug>/item.md` (no numeric prefix).
- CLI surface:
  - `issuectl new` → returns `slug` instead of `number` in `--json` output.
  - `issuectl show <slug>`, `close <slug>`, `update <slug>` accept slugs.
  - `--json` output: rename `number` → `slug`. Drop `number` field entirely.
- Cross-references in body text: decide on a sigil. Proposal: `@quiet-brave-otter` (since `#` is reserved for markdown headers and gh-style numeric refs). To be confirmed.
- Remove or deprecate `issuectl renumber` — collisions should not happen anymore. Removal is cleaner; if kept, document as legacy-only.

### Phase 4 — `issuectl doctor` subcommand

New top-level subcommand whose job is to verify repository health and apply automatic migrations.

Initial checks:
1. **Numbered → slug migration.** Detect any directory matching `issues/{open,closed}/<NN>-<slug>/` (legacy format). For each:
   - Generate a fresh random `adj-adj-noun` slug (or reuse the trailing slug part of the directory name if it parses cleanly — TBD during implementation).
   - Rename the directory to `issues/{open,closed}/<new-slug>/`.
   - Rewrite frontmatter: drop `number`, add `slug`.
   - Rewrite `#NN` references throughout `issues/**/*.md` (best-effort regex; flag ambiguous matches).
   - Print a migration summary (old → new mapping).
2. **Frontmatter validity.** Catch missing/invalid fields the way `renumber` does today.
3. **Orphan check.** Issues referenced by an epic that no longer exist, and vice versa.
4. **Slug sanity.** Verify all slugs match `is_valid()` and are unique.

Flags:
- `issuectl doctor` — read-only report.
- `issuectl doctor --fix` — apply migrations and fixes.
- `issuectl --json doctor` — machine-readable report.

## Tasks

- [ ] **Phase 1** — Build wordlists
  - [ ] Download EFF, moby, `names` corpora into `tools/wordlists/sources/`
  - [ ] Merge to `tools/wordlists/all-words.csv` with source provenance
  - [ ] Classify via agent in 100-word batches → `tools/wordlists/classified.csv`
  - [ ] Human review of flagged words
  - [ ] Emit `src/slug/wordlists/{adjectives,nouns,verbs}.rs`
  - [ ] Add `NOTICE` with attributions
- [ ] **Phase 2** — `src/slug.rs` with `generate()`, `is_valid()`, collision retry
- [ ] **Phase 3** — Migrate `issuectl` core to slugs
  - [ ] Frontmatter: `number` → `slug`
  - [ ] Directory layout: drop numeric prefix
  - [ ] CLI commands accept slugs
  - [ ] `--json` output: `slug` field
  - [ ] Decide reference sigil (`@slug` vs other) and update body-text handling
  - [ ] Remove or deprecate `renumber`
- [ ] **Phase 4** — `issuectl doctor`
  - [ ] Subcommand scaffolding + `--json` output
  - [ ] Numbered → slug migration with reference rewriting
  - [ ] Frontmatter / orphan / slug-sanity checks
  - [ ] `--fix` flag
- [ ] Update `README.md` and the `/issue` skill docs (`.claude/skills/issue/SKILL.md`)
- [ ] Run `cargo test`, `cargo clippy`, integration tests

## Out of scope

- GitHub-style numeric back-compat (`#42` everywhere). We accept the breaking change; this is an early-stage tool and migration is one-shot.
- Localized / non-English wordlists.
- User-configurable wordlists (could be a future feature).

## Notes

- **Worktree review carve-out:** `/worktree-merge` and similar review flows do **not** need to validate the wordlist contents or classifications. Those are trusted as a vendored asset and the reviewer should skip them. Review focus belongs on `slug.rs`, the CLI/frontmatter migration, and the `doctor` subcommand.
- This is a breaking change to repo layout. Any existing `issues/` directories must be migrated via `issuectl doctor --fix` before further work.
- Birthday-paradox math for adj-adj-noun (~250M combos): 50% collision risk only after ~18 000 issues; per-creation retry cap of 8 covers it with massive headroom.
