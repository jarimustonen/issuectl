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
| `## Decisions`       | Architectural choices recorded so agents stop redoing  |
| `## Agent Runs`      | Auto-appended audit trail of agent attempts            |
| `## Reopen Notes — <YYYY-MM-DD>` | Rationale stub auto-appended on reopen     |

`## Notes` is **not** a recognised section. Pre-existing files using
`## Notes` are migrated to `## Comments` by `issuectl doctor --fix`.
If both headings exist in the same file, doctor flags the slug as a
conflict and skips the rewrite — manual merge is required.

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

## Validation

The `note_issue` mutation rejects inputs that would let the heading
shape be fabricated:

- `author` cannot contain whitespace, control characters, `@`, or the
  middle-dot separator. Without this, `--as $'alice\n## Pwned'`
  would mint a fake H2 section.
- `message` cannot contain a line beginning with `## ` or `### `
  outside a fenced code block. Quoting the same content inside a
  fence is fine because the parser is fence-aware.

Heading detection in both the writer (`append_block`,
`insert_block_in_section`) and the reader (`parse_section`) tracks
fenced code-block state. A user pasting a shell snippet whose
comments start with `##` cannot accidentally truncate the section
they're commenting on.

## Implementation

`src/body_sections.rs` exposes:

- `append_block(body, section, block) -> String` — append-or-create
  (writer).
- `append_reopen_notes(body, date) -> String` — always creates a
  new section.
- `render_note_block(ts, author, message) -> String` — block shape.
- `parse_section(body, section) -> Vec<Block>` — reader. `Block`
  carries `timestamp`, `author`, `body`.
- `validate_author(author) -> Result<()>` and
  `validate_message(message) -> Result<()>` — input guards.
- `canonicalise_body_leading(body) -> String` — used after every
  body edit so `serialize_item` always emits `---\n\n<body>`.
- `now_iso() -> String` — UTC ISO-8601 timestamp.

`src/mutate.rs::note_issue` wraps `append_block` under the repo
`flock` + atomic-write protocol from §3 of `web-edit-sync.md`.
The reopen auto-append is wired into `update_issue_under_lock` so it
participates in the same write.
