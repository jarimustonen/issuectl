# Frontmatter schema (v1)

Lives at `issues/.schema.yaml`. Auto-written on first use; safe to
edit and commit. Drives `issuectl new` / `update` / `doctor`
validation: missing required fields and out-of-enum values are
rejected (mutations) or reported (doctor).

## Decisions

- **Format: YAML.** The repo already uses YAML for frontmatter, so
  the schema mirrors what it describes, supports comments, and is
  human-editable. TOML would force a second config dialect; JSON
  has no comments and would make the leading rationale harder to
  ship in-file.

- **Path: `issues/.schema.yaml`.** Co-located with what it
  describes. The leading dot keeps it unobtrusive in `ls`. All
  existing `read_dir` callers in the codebase gate on `is_dir()`,
  so a plain file in `issues/` is already filtered out of slug
  discovery — no extra carve-out needed.

- **User schema is layered on the default (whole-`FieldSpec`
  replacement).** `schema::load` starts from the built-in default
  and overlays each user-declared field by name; entries the user
  doesn't redeclare keep their built-in spec. This is *whole-spec*
  replacement, not property-level merge — a user redeclaring `type`
  must restate the enum if they want to keep it. Unknown fields in
  an issue are NOT errors, so agents can attach extra metadata
  freely.

- **Built-in `required` can be overridden** by redeclaring the
  field. Default schema requires `type`, `status`, `priority`.
  `created` is intentionally NOT required by default — repos that
  pre-date schema enforcement may have issues without it. Tighten
  it once you've verified all issues have it (run `doctor` first).

- **`--field key=value` for custom fields on `new`.** Repeatable;
  validated against the schema along with the rest of the
  frontmatter. Built-in keys (`type`, `priority`, etc.) must use
  their dedicated flags so clap-level validation isn't bypassed.

- **Type-strict.** A field declared scalar but populated as a list
  (or vice versa), and a non-string element in a string-typed list
  field, raise a `WrongType` violation. Without this, `type: 42` or
  `labels: bogus` would silently bypass enum constraints.

- **Enums (v1).** A field may declare `enum: [..]` of allowed
  string values. For list-shaped fields (`list: true`), the
  constraint applies element-wise. Complex shapes (e.g. `commits`,
  a list of `{hash, summary}` mappings) are intentionally not
  describable in v1 — sum/object/regex constraints are deferred
  until a concrete need surfaces.

- **Atomic bootstrap.** `ensure_default_written` uses
  `O_CREAT|O_EXCL` so an interrupted prior run or a racing writer
  cannot truncate an existing schema file.

- **Two error classes.** `MutateError::SchemaViolation` (the
  request would produce schema-violating frontmatter — 422,
  client-actionable, supply `--field`) is distinct from
  `MutateError::SchemaConfig` (the schema file itself is malformed
  or unsatisfiable — 500, an operator must edit the schema file).

- **Load-time guards.** `schema::load` rejects two configurations
  that would make `issuectl new` permanently fail:
  - `slug: required: true` — slug is the directory name, not a
    frontmatter field.
  - any custom field with `required: true, list: true` — `--field
    key=value` is scalar-only in v1; built-in list fields
    (`labels`, `related`) are exempt because their dedicated flags
    can populate them.
  Failing at load time gives the user the error when they edit
  `.schema.yaml`, not later when an unrelated mutation fails.

- **API surface for custom fields.** Both `issuectl new --field
  key=value` (CLI) and `POST /api/issues` with `"custom_fields":
  {...}` (web) accept arbitrary scalar custom fields. Built-in
  keys (`type`, `priority`, ...) are reserved and must use their
  dedicated flag/field to keep clap-level validation in play.

## Enforcement points

- `issuectl new` builds the frontmatter Mapping in memory,
  validates it against the schema, then renders + writes. The
  in-memory check avoids the previous round-trip through string
  parsing.
- `issuectl update` / `close` / `body set` validate the
  post-mutation frontmatter inside the same write lock that holds
  the rest of the M1 protocol (read → patch → write → publish).
  The schema check sits between patch and write so a rejection is
  exit-time, not after a partial rewrite.
- `issuectl doctor` walks every flat-layout issue and surfaces
  schema violations alongside its existing checks. Legacy
  `<NN>-<slug>` directories *under `issues/{open,closed}/`* are
  skipped — `doctor --fix` rewrites their frontmatter anyway, so
  flagging them as schema-violating would just be noise. A flat
  issue whose name happens to match the legacy shape (e.g.
  `--slug 12-things`) is still validated.

## Bootstrap

A missing `issues/.schema.yaml` is written on:
- the first `issuectl new` / `update` / `close` / `body set` (any
  WriteLock-holding mutation), and
- `issuectl doctor --fix`.

Read-only `doctor` reports the missing file as a hint without
writing it.
