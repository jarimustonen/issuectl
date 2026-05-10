---
created: 2026-05-10
updated: 2026-05-10
type: bug
reporter: jari
status: done
priority: normal
closed: 2026-05-10
---

# 0.5.1: bugs and friction encountered during 3DBear monorepo adoption

_Source: 3DBear monorepo upgrade from 0.3.1 → 0.5.1 (2026-05-10)._
_Follow-up to issuectl-feedback-2026-05-06.md (0.3.1 findings, separately delivered)._

Captured while migrating ~243 issues from the legacy
`issues/{open,closed}/<slug>/` layout to the canonical flat layout.
The migration ultimately succeeded but required hours of manual
schema-violation cleanup before `doctor --fix` would run. Several of
the issues below are user-experience footguns that would block any
team adopting 0.5.1 against an organically-grown issue tree.

## Finding 1: `issuectl list` still panics on non-ASCII titles (regression / unfixed)

**Command:** `issuectl list`

```
thread 'main' (65842462) panicked at crates/issuectl/src/main.rs:1999:29:
```

This is the same UTF-8 byte-vs-char-boundary bug reported as Finding 1
in the 0.3.1 feedback. In 0.5.1 the panic site moved from
`src/main.rs:1162` to `crates/issuectl/src/main.rs:1999` — likely the
table renderer was refactored but the underlying byte-index slicing
remains. `issuectl list --json` still works.

In a Finnish-language repo (titles full of ä/ö) `list` is unusable
from the terminal — agents and humans both have to fall back to
`--json | jq` or the web board.

**Suggested fix:** same as before — use `s.chars().take(N)` or
`unicode-width` instead of byte-index slicing.

## Finding 2: `doctor --fix` is all-or-nothing — refuses to run while ANY violation exists

**Observed:** `doctor` reported the repo had:

- 240 directories needing layout migration
- 216 schema violations (renames, status enums, missing `type:`, etc.)
- 2 unparseable files
- 1 invalid slug (`flat/daily`)
- 48 status/closed-date inconsistencies

`doctor --fix` refused to migrate the layout until **every** schema
violation was hand-fixed first. This is backwards for adoption
workflows: layout migration is the safest, most mechanical operation
in the doctor's toolbox (just `git mv`), and would dramatically
shrink the cleanup surface (paths in error messages get shorter,
schema validation runs on the canonical structure, etc.). Forcing
it last makes the user fight a 200-line error report with paths
referring to a layout that's about to vanish.

**Concrete pain:** I ended up writing a custom Python script to
apply the renames and status normalizations across 162 files,
because doctor wouldn't budge until all of them were already fixed.
At that point the layout migration is almost beside the point.

**Suggested fix:**

- Add `--fix-layout-only` (or staged `--fix --phase=layout`,
  `--phase=schema`) so users can take the migration in chunks.
- Or: run independent fix passes in dependency order, reporting
  "applied N, blocked on M" instead of refusing the whole batch.
- At minimum: `--fix --force` that does what it can and leaves the
  rest reported.

## Finding 3: `--fix` does not create `.schema.yaml` even though doctor's own message promises it will

**Output line from `doctor`:**

```
Schema file missing at issues/.schema.yaml (will be auto-created on first `--fix` or write).
```

In practice: `doctor --fix` aborted with the all-or-nothing block
above (Finding 2), so it never reached the schema-creation code
path. The promise is broken: there's no way to ask doctor to "just
create the schema file so I can edit it." I had to wait until every
other violation was clean before the schema file appeared.

The schema file is also useful as a reference for *what fields the
built-in schema knows about* — without it, users guess at allowed
status values, allowed type values, etc.

**Suggested fix:** create `.schema.yaml` (with the auto-generated
defaults) on the first `--fix` invocation regardless of whether
other violations block migration. Even better: `issuectl schema
init` as an explicit subcommand.

## Finding 4: doctor reports phantom "frontmatter fields" from body YAML code blocks

This caused real confusion during the migration. Doctor reported
"Unknown frontmatter keys" for fields that I knew weren't in the
frontmatter:

```
Unknown frontmatter keys (not declared by schema):
  simply-hateful-wheel: shortname, bcf_source, course_id, deployed_by,
                        mode, timestamp, commit, commit_full
  markedly-reminiscent-range: feedback, options
  ridiculously-responsible-vest: riskit
  particularly-tawdry-wire: qa_suppressions
  notably-alluring-owl: launched
```

Reproduction: a `# Heading` followed by a fenced YAML code block in
the issue body, e.g.:

````markdown
## Proposed data model

```yaml
shortname: "ako"
bcf_source: "courses/ako"
course_id: 123
```
````

Doctor scans past the `---/---` boundaries and treats indented YAML
keys *inside fenced code blocks in the body* as if they were
frontmatter fields. This pulled in:

- YAML examples in proposal docs (`shortname: "ako"`)
- Wrapped English prose (`launched: the OIDC handshake completes,
  then Moodle POSTs...` — the word "launched:" at the start of a
  continuation line)
- Bare `riskit:` because there happened to be a `## Riskit` heading
  followed by indented bullets

The most pernicious case was `launched:` from prose like:

```
... currently fails when
launched: the OIDC handshake completes, then Moodle POSTs the id_token
...
```

That's not YAML — it's just a line wrap. Doctor flagged it as a
frontmatter field anyway.

**Suggested fix:** restrict frontmatter parsing strictly to the
content between the *first* pair of `---` lines. Code blocks
(``` ... ``` or `~~~ ... ~~~`) and the rest of the body are out of
scope for frontmatter validation.

## Finding 5: `commits[].hash` value `315194e2` parses as a float (`3.15194e2 → 31519400.0`)

**Command:** `issuectl doctor`

```
closed/utterly-draconian-ink: invalid YAML frontmatter:
  invalid type: floating point `31519400.0`, expected a string
```

The frontmatter contained:

```yaml
commits:
  - hash: 315194e2
    summary: "..."
```

YAML 1.2 with implicit typing parses `315194e2` as scientific
notation → 3.15194 × 10² = 31519400.0 (the integer part of the
float, lossy). Same can happen with any short hash that happens to
contain `e` followed by digits.

I worked around it by quoting the hash (`hash: "315194e2"`), but
this is a real-world hazard: roughly 1 in ~5,000 short git hashes
will parse as a float. Anyone using `issuectl note --commit
<short-hash>` or hand-editing commits arrays will eventually hit it.

**Suggested fix:**

- When `issuectl` itself writes commits arrays, always quote
  `hash:` values regardless of content.
- Consider a built-in `commits` field type that forces hash strings.
- Doctor could detect "looks-like-float-but-was-probably-a-hash"
  and emit a friendlier error pointing at quoting.

## Finding 6: Status names diverge from common conventions; no auto-coerce

The schema requires:

```
[open, in-progress, testing, done, fixed, wontfix, duplicate,
 cannot-reproduce, obsolete]
```

But our 240-issue corpus contained, all of them rejected:

- `closed` (~80 issues — extremely common in pre-issuectl repos)
- `resolved` (3)
- `in_progress` (with underscore, common Moodle/Django convention)
- `paused`, `blocked` (real workflow states many teams use)
- `enhancement`, `refactor` for `type:` (GitHub/GitLab common values)

`doctor --fix` does not auto-coerce any of these. I had to write a
script to map them. The mappings I chose (`closed → done`, `resolved
→ done`, `in_progress → in-progress`, `enhancement → improvement`,
`refactor → improvement`) felt obvious — `--fix` could plausibly
do them with a `--coerce-status`/`--coerce-type` flag.

**Suggested fix:** add a `status_aliases` and `type_aliases` map
to the schema (built-in plus user-overridable):

```yaml
status_aliases:
  closed: done
  resolved: done
  in_progress: in-progress
type_aliases:
  enhancement: improvement
  refactor: improvement
```

Then `--fix` rewrites them automatically. This single feature
would have saved ~half the manual work in our migration.

## Finding 7: `priority: low` is invalid (only `normal`, `high`)

This caught me by surprise — many issue trackers default to
`low/medium/high` or `low/normal/high/critical`. The 0.5.1 schema
allows only `normal` and `high`, so all `priority: low` entries
were rejected. We mapped them all to `normal`, but losing the
information was a small annoyance.

**Discussion point, not a bug:** is the two-value design
intentional? If so, the schema docs / `issuectl new --help` should
say "only normal/high — low is intentionally unsupported because
[reason]." If not, consider allowing `low`.

## Finding 8: `closed:` is required for closing statuses, but no one tells you that

After hand-stripping `closed:` from 41 issues (because we thought it
was a vestigial field), `doctor` came back with:

```
Status / closed-date inconsistencies:
  absolutely-glamorous-feather: closing status "done" requires `closed:` date
  ... × 158
```

The schema file we generated *does* declare `closed:` as `required:
false`, but apparently the lifecycle classification (closing vs.
active status) imposes a *conditional* requirement that isn't
expressed in the schema doc.

**Suggested fix:** make this constraint explicit in the schema —
something like `closed: { required_when: "status is closing" }`.
Or at minimum, document it in the auto-generated schema file
comments.

## Finding 9: `doctor` reports the same legacy-layout list 240 times in the same run

When `--fix` is blocked, doctor still prints the entire layout
warning list. This is fine in itself, but combined with the
all-or-nothing fix policy (Finding 2), every iteration of
"fix-something-rerun-doctor" produces 240 lines of layout warnings
that scroll off the screen. A `--no-layout-warnings` flag, or
collapsing to "240 issues need layout migration" by default with
`--verbose` to expand, would help.

## Finding 10: doctor doesn't notice when issuectl-relevant files are gitignored

After running `issuectl agents init`, the new `.issuectl/AGENTS.md`
file appeared. But our repo's `.gitignore` had a blanket
`.issuectl/` rule from an earlier setup, so the file was silently
untracked — invisible to teammates and to CI.

Doctor reported nothing about this. It happily references
`.issuectl/AGENTS.md` in its policy hints (`run issuectl agents init
to opt in`) but doesn't check whether the file would actually be
tracked once written. The same blind spot likely applies to
`issues/.schema.yaml` — if a user has `issues/.*` or similar in
gitignore, the schema file gets created but never committed.

The footgun is asymmetric: untracked-but-present feels normal on the
author's machine (everything works), but agents on other machines
(or CI) see a "missing schema" / "missing AGENTS.md" repo and may
silently fall back to defaults, diverging from local behavior.

**Suggested fix:** when doctor sees that `.issuectl/AGENTS.md`,
`issues/.schema.yaml`, or any tracked-by-design issuectl file is
matched by `git check-ignore`, surface a warning:

```
.issuectl/AGENTS.md is gitignored — agents on other machines won't
see your policy file. Adjust .gitignore or move the file.
```

This is cheap to detect (`git check-ignore -v <path>`) and would
have saved me from silently committing a half-working setup.

## Finding 11 (minor): `agents init` always succeeds even when `issues/.schema.yaml` doesn't exist yet

I ran `issuectl agents init` after the migration completed. It
wrote `.issuectl/AGENTS.md` using the schema-derived block. But
because of Finding 3 (schema file gets written late), running
`agents init` *before* the migration is complete would write a
schema-block based on... what? The built-in defaults? An empty
schema? The output is silent about which.

**Suggested fix:** `agents init` could log "Using built-in default
schema (issues/.schema.yaml not found)" or "Using project schema
at issues/.schema.yaml" so the user knows which inputs went in.

## Working well

- `doctor --fix` is correct and fast once it agrees to run — 240
  directory moves + schema file generation in well under a second.
- The schema file format is excellent: the comments are clear, the
  custom-field declarations are obvious, and the merge semantics
  ("layered on top of built-in defaults") are documented inline.
- Random slug names continue to eliminate merge conflicts entirely.
- `issuectl serve` (kanban) renders the post-migration repo
  beautifully.
- `issuectl new` correctly emits a quoted hash by default in
  commits arrays, so Finding 5 only bites legacy data.
- The classification of statuses into "active" vs. "closing" with
  configurable `status_classes` overrides is a nice design.

## Migration log (for the record)

The 3DBear monorepo migration produced four commits in `main`:

- `chore(issues): poista closed/resolution-kentät, normalisoi statukset` (47 files)
- `chore(daily): siirrä päiväraportit issues/daily → daily/` (31 files)
- `chore(issues): normalisoi schema (issuectl 0.5.1)` (162 files)
- `chore(issues): valmistele issuectl 0.5.1 -migraatio + aja layout-fix` (379 files)

Total: ~360 unique files touched across the four phases. Doctor is
now fully clean.

## Comments

### 2026-05-10T12:16:10Z · @jari

Split into 11 separate issues, all labelled 'from-3dbear-0.5.1-feedback' under @hugely-exciting-spiders. See: marginally-receptive-kettle, staggeringly-important-zoo, unreasonably-attractive-star, virtually-callous-rainstorm, thoroughly-kaput-pocket, reasonably-likeable-stone, tremendously-broken-brain, extremely-poor-dirt, ridiculously-outrageous-fold, simply-workable-umbrella, eminently-dramatic-anger.
