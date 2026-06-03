---
created: 2026-06-03
updated: 2026-06-03
type: bug
reporter: jari
status: in-progress
priority: normal
---

# doctor broken_attachment_refs heuristic produces false positives

_Source: doctor / attachment resolver_

## Description

`issuectl doctor` flags markdown content as `broken_attachment_refs`
when the issue body merely *describes* `![alt](path)`-style syntax
(inside backticks) or links to repo-relative source-code paths. None
of these are real attachments — they're documentation prose or
cross-file code pointers — but the regex-based heuristic can't tell
them apart from a legitimately missing attachment.

Authors can't silence the noise without mangling their documentation
(dropping backticks, rewriting code-link syntax). The doctor report
stays permanently dirty, which dilutes signal: a *real* broken
attachment gets lost in the noise.

Observed in the 3DBear monorepo against `issuectl 0.6.3`.

## Reproduction

In a repo with issues that describe markdown syntax in code spans or
link to sibling source files:

```bash
issuectl --json doctor 2>&1 | jq '.broken_attachment_refs'
```

Example output from the 3DBear monorepo (`main` @ `bf77c726c`), all 6
entries false positives:

```json
[
  { "ref": "src",                                              "slug": "amazingly-enchanted-vest" },
  { "ref": "path",                                             "slug": "downright-high-suit" },
  { "ref": "kurssi-ai-server/src/cli/sops.ts#L87-L98",         "slug": "phenomenally-distinct-operation" },
  { "ref": "path",                                             "slug": "surprisingly-natural-moon" },
  { "ref": "path",                                             "slug": "surprisingly-natural-moon" },
  { "ref": "kurssi-ai-server/src/tools/db-query.ts#L146-L163", "slug": "very-dizzy-snails" }
]
```

## Classes of false positive

### Class 1 — backtick-wrapped syntax descriptions

The author is *naming a markdown construct*, not using one. Examples:

```markdown
- Uusi `_parse_embed_and_inline_media`: aiempi `::embed[]{}`+`![alt](src){attrs}` -käsittely irrotettiin omaksi funktioksi.
- [ ] Kuvien sisällytys `![alt](path)`-syntaksilla
Images are validated (ERROR if missing), but videos use the same `![](path){.video}` syntax and are silently skipped.
```

Doctor extracts `src` / `path` (the literal placeholder words inside
the backticks) and treats them as attachment paths to resolve.

### Class 2 — repo-relative source-code cross-links

```markdown
`loadMoodleAdminPassword()` in [kurssi-ai-server/src/cli/sops.ts:87-98](kurssi-ai-server/src/cli/sops.ts#L87-L98) reads …
… ks. [kurssi-ai-server/src/tools/db-query.ts:146-163](kurssi-ai-server/src/tools/db-query.ts#L146-L163). …
```

Both target files exist and are tracked in the monorepo. These are
intentional cross-file pointers (analogous to GitHub
`path/to/file.ts#L87-L98` permalinks). They are not attachments and
shouldn't be resolved as siblings of `item.md`.

## Root cause (hypothesis)

The attachment resolver appears to:

1. Regex-scan `item.md` for any `[...](…)` or `![...](…)`.
2. Treat the URL portion as a relative path to be resolved next to
   `item.md`.
3. Flag it as `broken_attachment_refs` when the path doesn't exist in
   the issue directory.

This misses two distinctions:

- **Inside `` `…` `` code spans, markdown link syntax is inert** — per
  CommonMark, code spans are literal text. A parser-based walk (e.g.
  `pulldown-cmark`'s `Event::Text` vs.
  `Event::Start(Tag::Image)` / `Event::Start(Tag::Link)`) skips these
  automatically.
- **Repo-relative paths and external URLs are not attachments.** Only
  paths that resolve as a sibling file of `item.md` (or use a known
  attachment scheme) should be checked. A link target whose resolved
  path escapes the issue directory is almost certainly a cross-file
  code pointer.

## Suggested fix

1. **Use a real markdown parser** (e.g. `pulldown-cmark`) instead of
   regex. Iterate events and only consider `Tag::Image` / `Tag::Link`
   targets emitted *outside* code spans and code blocks.
2. **Scope attachment resolution** to paths that:
   - have no URL scheme (`http://`, `https://`, `mailto:`, …),
   - resolve to an existing file under the issue's own directory tree
     (`issues/<slug>/`).
   - Anything resolving outside `issues/<slug>/` is a cross-file
     pointer — out of scope for `broken_attachment_refs`.
3. **Optional**: an opt-out marker for authors who want literal
   `![alt](path)` prose examples — but a proper parser fix removes
   the need.

## Suggested test cases

Add fixtures under `tests/doctor/fixtures/`:

- `code-span-with-image-syntax/item.md` — `` `![alt](path)` `` inside
  backticks → must not be flagged.
- `repo-relative-code-link/item.md` —
  `[foo.ts:10-20](../foo.ts#L10-L20)` pointing at an existing file
  outside the issue dir → must not be flagged.
- `legit-missing-attachment/item.md` — `![screenshot](missing.png)`
  referencing a sibling file that doesn't exist → must still be
  flagged.
- `legit-existing-attachment/item.md` — `![ok](existing.avif)` with
  the file present → must not be flagged.

A regression test asserting these four classifications would prevent
re-introduction.

## Impact on agent workflows

The `/issue` skill instructs agents to run `doctor` after major issue
work. Agents (and humans) learn to ignore `broken_attachment_refs`
because every report is dominated by false positives — so a real
broken attachment will be missed.

## Workarounds tried (and rejected)

- **Rewriting backticks**: corrupts prose meaning ("the
  `![alt](path)` syntax").
- **Removing the code links**: loses navigability — those links are
  the most valuable part of issues describing code-level bugs.
- **Deleting the issues**: not viable; the issues are accurate
  historical records of fixed bugs.
