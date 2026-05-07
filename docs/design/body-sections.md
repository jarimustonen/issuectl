# Body section conventions

Append-only markdown sections that the safe-mutation CLI can both
parse and write. Markdown stays human-readable; the tool gives it
shape so agents and the web UI can locate, append, and audit
without re-litigating layout.

Zero schema beyond the heading shape — anything inside a block is
freeform markdown.

## Section names

A "section" is an H2 heading whose text matches one of these
reserved names exactly (post-trim):

| Heading              | Purpose                                                |
| -------------------- | ------------------------------------------------------ |
| `## Comments`        | Free-form comments / notes from humans or agents       |
| `## Notes`           | Alias of `Comments`. Existing files keep this heading. |
| `## Decisions`       | Architectural choices recorded so agents stop redoing  |
| `## Agent Runs`      | Auto-appended audit trail of agent attempts            |
| `## Reopen Notes — <YYYY-MM-DD>` | Rationale stub auto-appended on reopen     |

Other H2 sections (e.g. `## Description`, `## Acceptance Criteria`)
are untouched by these tools — the convention is additive.

## Block shape

Each block inside a section is an H3 heading carrying a UTC ISO-8601
timestamp and an `@author`, followed by freeform body markdown:

```
### 2026-05-07T14:23:11Z · @alice

Free-form content. Multiple paragraphs OK. Code fences OK.
```

- Timestamp: `YYYY-MM-DDTHH:MM:SSZ` (UTC, second precision).
- Separator: ` · ` (U+00B7 with single spaces on each side).
- Author: `@<name>` — agent identifier or human handle.
- Block ends at the next H3 within the same section, or at the next
  H2, or EOF.

Blocks are append-only. New entries land at the **end** of the
section (newest-last, matching commit-log convention). The
mutation CLI never reorders prior blocks.

## Reopen Notes

Each closing → active transition appends one new section:

```
## Reopen Notes — 2026-05-07

_Add rationale for reopening here._
```

If the same issue is reopened again later, a second
`## Reopen Notes — <date>` section is appended below the first
(rather than merged) so the audit trail keeps every event distinct.

## Section ordering

When the CLI creates a previously-missing section, it appends it at
the **end** of the body. Existing section ordering is never rewritten.
`issuectl fmt` does not reorder sections, blocks, or their content —
the only normalisations it performs are the ones already documented
in `web-edit-sync.md` (frontmatter key order, sorted arrays,
trailing-whitespace strip outside code fences, ATX headings).

## Idempotency contract

- Repeated appends to the same section keep all prior blocks intact.
- `issuectl fmt` is a fixed point: `fmt(fmt(x)) == fmt(x)` for any
  body that uses these conventions.
- An unrelated edit to the body (frontmatter PATCH, body rewrite via
  `issuectl body set`) preserves these sections byte-for-byte unless
  the edit explicitly touches them.

## CLI surface

- `issuectl note <slug> --as <user> "<message>"` — appends a block
  to `## Comments` (creating the section if missing).
- Reopen auto-stub: any `update --status <active>` (or equivalent
  PATCH) on a closed issue appends a `## Reopen Notes — <today>`
  section in the same atomic write.

The companion safe-mutation CLI verbs (`decide`, `agent-run`,
…) build on top of `body_sections::append_block` with the
same shape.

## Implementation

`src/body_sections.rs` exposes:

- `append_block(body, section, block) -> String` — append-or-create.
- `append_reopen_notes(body, date) -> String` — always creates a
  new section.
- `render_note_block(ts, author, message) -> String` — block shape.
- `now_iso() -> String` — UTC ISO-8601 timestamp.

`src/mutate.rs::note_issue` wraps `append_block` under the repo
`flock` + atomic-write protocol from §3 of `web-edit-sync.md`.
The reopen auto-append is wired into `update_issue_under_lock` so it
participates in the same write.
