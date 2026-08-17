# 0004 — Consolidate the CLI verb surface

- Status: accepted (2026-08-17, `@cli-verb-surface`)
- Deciders: maintainer

## Context

`issuectl` has grown to roughly fifty top-level commands. Breadth is less
costly for an AI-first CLI than for a human-only one, but every spelling is a
binary semver commitment and must be taught in help, completions, tests, and
the six bundled skill templates. Several current spellings are wrappers over
the same mutation, and `triage` retains a second intake layout.

The binding AI-first CLI canon requires one CRUD vocabulary (`create`,
`update`, `delete`, `list`, `show`), treats `apply` as an exception only for
real convergent manifests, and requires a warning-bearing deprecation window.
A four-lens review (architecture, maintainability, agent ergonomics, and
compatibility) agreed on removing wrappers, retaining distinct domain
operations, and avoiding a disruptive synthetic reporting namespace.

## Decision

### Canonical surface

`update` is the sole selective field and relationship mutation. It will gain
field, list-operation, body-operation, patch-file, and query-selection forms
as needed. `bulk` is therefore `update --query …`; it is not a second writer.
`apply` is `update --patch-file …`: its current expected-version transaction
is a patch, not the convergent declarative state required for the canon's
standalone `apply` exception. `close`, `set`, `assign`, `label`, and `depend`
are similarly update forms.

`note` is the one canonical append-to-history verb. `comment` is only its
compatibility alias. Aliases are compatibility machinery, never a second
recommended spelling: help, examples, completions, and skills use only the
canonical form.

The intake flow is the sole reception pipeline. `triage` and `issues/inbox/`
are deprecated as proposed in `@deprecate-triage-inbox`; `scan-todos` files
untracked findings through `intake file` rather than creating inbox drafts.
This is gated on fixing `@intake-queue-legacy-mismatch` first. `doctor --fix`
will migrate stranded inbox drafts during the whole transition.

`export` remains a portability endpoint, but only for its machine-readable
JSON representation. `import json` and `import github` remain ingestion
adapters, not backup/restore claims. CSV and Markdown export are lossy,
human-oriented renderings and will be removed; callers needing a view derive
it from `list --json`.

Read views retain their distinct top-level domain names rather than moving
behind a new `report` noun. That rename would make every existing read script
migrate while retaining all implementations and output shapes. The proven
overlaps are folded: `stats` and `workload` become `metrics` views, and
`burndown` becomes `cycle burndown`. `activity` (repository history),
`timeline` (one issue's status history), `epic`, and `cycle` are distinct
queries, not synonyms.

| Current top-level command | Classification | Canonical destination or rationale |
| --- | --- | --- |
| `version` | keep | Drift-contract lifecycle verb. |
| `config` | keep | Inspectable configuration surface. |
| `list` | keep | Canonical multi-issue query. |
| `show` | keep | Canonical single-issue query. |
| `open` | leave-as-is | Explicit human convenience, never taught to agents or extended; no new interactive verbs. |
| `attach` | leave-as-is | Attachment-copy transaction has no field-patch equivalent. |
| `search` | leave-as-is | Full-text query is distinct from field-filtered `list`. |
| `stats` | fold-into-`metrics` | Summary becomes a metrics view. |
| `ready` | leave-as-is | Definition-of-Done evaluation, not a generic read. |
| `duplicates` | leave-as-is | Heuristic analysis operation. |
| `create` | keep | Canonical creation verb. Its `new` alias is alias-then-remove. |
| `update` | keep | Sole selective mutation verb. |
| `close` | fold-into-`update` | `update <slug> --status <closing-status>` is equivalent. |
| `rename` | leave-as-is | Repo-wide reference rewrite is a distinct transaction. |
| `stale` | leave-as-is | Time/history-derived diagnostic query. |
| `archive` | leave-as-is | Repository lifecycle move. |
| `note` | keep | Canonical append-to-history verb; `comment` is alias-then-remove. |
| `set` | fold-into-`update` | Field flags and `--clear-<field>` replace positional field mutation. |
| `assign` | fold-into-`update` | `--assignee` / clear-assignee replace the wrapper. |
| `check` | leave-as-is | Checked markdown-task transition, not a frontmatter field patch. |
| `label` | fold-into-`update` | `--add-label` / `--remove-label` replace both current invocation forms. |
| `apply` | fold-into-`update` | `--patch-file` preserves its one-transaction patch semantics. |
| `bulk` | fold-into-`update` | `--query` plus the same mutation flags preserves batch semantics. |
| `body` | leave-as-is | Body replacement is a large-payload operation with its own stdin/file contract. |
| `init` | keep | Canonical bootstrap lifecycle verb. |
| `doctor` | keep | Canonical diagnostic and repair lifecycle verb. |
| `hooks` | fold-into-`fmt` | `fmt install-hooks` is the canonical formatter-owned installer. |
| `sync-commits` | leave-as-is | Git-history reconciliation job. |
| `agents` | leave-as-is | Repo policy resource namespace. |
| `skill` | keep | Bundled agent-skill namespace. |
| `fmt` | keep | Canonical record canonicalizer and installer namespace. |
| `context` | leave-as-is | Deterministic agent-context rendering. |
| `prompt` | leave-as-is | Template rendering against context. |
| `install-merge-driver` | fold-into-`fmt` | `fmt install-merge-driver` is the canonical formatter-owned installer. |
| `export` | keep | JSON-only portability endpoint after lossy formats are removed. |
| `triage` | alias-then-remove | Intake queue/transition flow replaces inbox promotion. |
| `pick` | alias-then-remove | Agents use `list --json`; interactive selection is not an AI-first command. |
| `completions` | leave-as-is | Shell-integration generator. |
| `scan-todos` | leave-as-is | Source-marker analysis; `--create-inbox` is alias-then-remove in favor of an intake-file flag. |
| `import` | leave-as-is | External-source ingestion, not a selective create wrapper. |
| `intake` | keep | Sole reception and disposition resource namespace. |
| `activity` | leave-as-is | Repository-wide issue-file Git history. |
| `timeline` | leave-as-is | Per-issue status-transition reconstruction. |
| `changelog` | leave-as-is | Release-note generation from Git range. |
| `metrics` | keep | Analytics namespace; absorbs `stats` and `workload`. |
| `depend` | fold-into-`update` | `--add-blocked-by` / `--remove-blocked-by` replace relationship subcommands. |
| `dag` | leave-as-is | Scheduling analysis with reservations. |
| `epic` | leave-as-is | Epic-navigation resource namespace. |
| `cycle` | leave-as-is | Cycle resource namespace; absorbs `burndown`. |
| `schedule` | leave-as-is | Recurrence resource namespace. |
| `workload` | fold-into-`metrics` | Workload rollup becomes a metrics view. |
| `burndown` | fold-into-`cycle` | Cycle-specific chart becomes `cycle burndown`. |

`help` is clap's generated help command, not a product verb; it remains.
Hidden completion helpers are implementation details and are not surface
commands. The current `new`, `comment`, and hidden `ls` aliases are covered above even
though clap does not list them as separate command rows. `ls` is an
alias-then-remove spelling of `list` in the same 0.16.0 → 0.17.0 window.

### New top-level verb policy

Default to extending an existing resource namespace or adding a flag to an
existing command. A proposed top-level verb needs all of the following:

1. It is not CRUD-shaped, a spelling alias, a field/list/body patch, or a
   presentation variant of an existing query.
2. It names a real domain transaction, a canon-sanctioned lifecycle operation,
   or a resource namespace with at least two coherent subcommands.
3. Its `--json` result, errors, dry-run behavior when mutating, help examples,
   completion impact, and all affected bundled skill templates are specified.
4. The proposing issue explains why nesting under the closest existing verb is
   worse, and records the compatibility and maintenance cost.

A new top-level verb requires an ADR amendment before implementation. A new
subcommand or flag does not, but still follows the same JSON and skill-sync
contract. Permanent synonyms are forbidden.

### Deprecation sequence

The target releases are deliberately batched to avoid maintaining several
half-converted surfaces:

- **0.15.0, preparation:** fix `@intake-queue-legacy-mismatch`; add the
  canonical replacement forms, doctor inbox migration/check, and tests. No
  deprecated spelling is removed.
- **0.16.0, deprecation release:** hide each folded command and alias it to
  its replacement. `triage`, `pick`, `new`, and `comment` follow the same
  treatment. CSV/Markdown `export` formats and `scan-todos --create-inbox`
  are deprecated aliases. Ship canonical-only help, completions, and all six
  skill-template updates in this release, and name 0.17.0 as removal.
- **0.17.0, removal release:** remove the deprecated command/format/flag
  implementations, inbox writers and discovery after migration coverage is
  proven, and batch the resulting breaking changes in the changelog.
- **0.18.0:** remove any compatibility tombstone that recognized a removed
  spelling solely to return a targeted `deprecated-verb-removed` error.

During 0.16.0 every deprecated invocation remains behaviorally equivalent and
exits as before. Text mode writes `warning:` to stderr; `--json` puts a
structured warning in the successful stdout envelope with a deprecation id,
replacement argv, and removal version. `ISSUECTL_NO_DEPRECATION_WARNINGS=1`
suppresses it. Deprecated spellings are absent from normal help and skills.
Removal never silently changes the `--json` envelope contract; if a tombstone
is retained, its error uses the normal structured error envelope.

## Consequences

The root surface becomes smaller where duplication is real, while preserving
specialized operations whose names communicate a distinct transaction or
query. `update` acquires a wider but regular flag contract, so implementation
must keep all write forms behind the existing locked, schema-validated mutation
path. Each fold is a separately testable post-`main.rs`-split issue.

The worked `@deprecate-triage-inbox` proposal is ratified with two scope
clarifications: its intake-stability blocker must land first, and its flag
replacement must make the new intake filing action explicit rather than using
an ambiguous bare `--file`.

Rejected alternatives:

- **Keep every synonym because agents can read help:** agents still pay for
  wrong guesses and template drift; this rejects canon §7 without a domain
  benefit.
- **Keep `apply` as a standalone manifest verb:** the current patch is an
  optimistic-concurrency transaction, not convergent desired state, so it
  does not meet the canon's `apply` exception.
- **Remove all `export`:** JSON remains useful for an explicit portable
  payload and external callers. Removing only lossy formats avoids falsely
  presenting CSV/Markdown as round-trippable data.
- **Create a `report`/`view` umbrella:** this would rename many distinct
  read contracts for little implementation deletion and create a larger
  compatibility migration than the retained carrying cost.
- **Remove `triage` immediately:** installed skills and scripts need a shipped
  warning window, and no repository may be stranded with inbox drafts.
- **Allow permanent convenience aliases:** aliases recreate the same surface
  ambiguity and documentation burden the decision removes.

## Date

2026-08-17
