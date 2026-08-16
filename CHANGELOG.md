# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Lane-structure design guide:** practical guidance for choosing serial
  conflict boundaries, collision tokens, parked work, and `unlaned` issues.

### Changed
- Replaced maintainer-specific metadata, examples, and infrastructure labels with neutral or fictional values.

### Fixed
- **`create --help`** now correctly describes title-derived default slugs, random opt-in and fallback behavior, and numeric collision suffixes.
- **`doctor --fix` remaining-findings summary** now counts every unresolved finding it lists, rather than grouped critical diagnostic categories.

## [0.13.0] - 2026-08-16

### Changed
- **BREAKING: versioned JSON envelopes.** Every `--json` success now returns
  `{ "schema_version": 1, "data": …, "warnings": [] }`; consumers must move
  all prior top-level result lookups to `.data` and read warnings from the
  envelope. Every JSON error now includes `schema_version` on stderr. Added
  `issuectl version [--json]` for one-call CLI/schema/skill drift audits.
- **`issuectl create` is now the primary issue-creation verb.** `issuectl new` remains a working alias.
- **Core clock injection.** Time-dependent core operations now use an injectable `Clock`, making mutation stamps, archive eligibility, and doctor date fallbacks deterministic in tests.

## [0.12.0] - 2026-08-16

### Added
- **Machine-readable help** — `issuectl --help --json` and every subcommand's
  `--help --json` emit a structured help document with subcommands, flags,
  arguments, accepted values, defaults, environment mappings, and runnable
  examples.
- **`issuectl skill list`** — enumerate the bundled `/issue`, `/issue-new`, and
  `/issue-intake` workflows with their Claude Code and Codex install targets,
  without inspecting or changing the derived pi.dev mirror.
- **`issuectl config path` / `issuectl config show`** — inspect the schema
  configuration path and each effective schema value. `show --json` reports
  per-value provenance as `source: "file"` for `issues/.schema.yaml` entries
  and `source: "default"` for built-in values.

## [0.11.0] - 2026-08-15

> Note: trailer-driven changelog compilation (`issuectl changelog`) was introduced
> mid-cycle in this release (`close --stamp`), so entries below were curated by hand
> as a one-time transition backfill. From 0.12.0 onward, closing an issue with
> `--stamp` populates release notes automatically.

### Added
- **`issuectl close --stamp`** — amends the current HEAD commit to append a
  `Fixes-Issue: @<slug>` trailer in exactly the format `issuectl changelog`
  compiles, so trailer-driven release notes accrue with zero manual discipline.
- **`comment` alias for `note`**, and `--message` / `--body` / `--body-file -`
  input on `note`/`comment` (previously the body was positional-only).
- **`issuectl new --lane` / `--lane-seq` / `--add-collision`** — an issue can be
  born into the scheduling DAG in one call, mirroring `update`.
- **`issuectl update --add-blocked-by` / `--remove-blocked-by`** (repeatable) —
  edit dependency edges from the CLI instead of hand-editing frontmatter.
- **`issuectl update --body-file` / `--description`** — set or replace an existing
  issue body (accepts `-` for stdin).
- **`epic tree`** — render epics and their children as a tree in the CLI.
- **pi.dev skill-corpus lifecycle** — provenance manifest, `skill pi-status`
  (drift classification), and `skill pi-prune` (safe orphan/missing cleanup),
  with cross-writer manifest locking.
- **`doctor --fix` merges `## Notes` into `## Comments`** automatically when an
  issue carries both sections.

### Fixed
- **`label … --remove … --json` no longer silently no-ops.** `label` now also
  accepts the `--add` / `--remove` flag form, and a malformed invocation under
  `--json` emits a proper error envelope with a non-zero exit instead of empty
  stdout with the mutation silently skipped.
- **`list --status done`** (and other closing statuses like `fixed` / `wontfix`)
  now returns matching closed and archived issues, instead of "No issues found".
- **`note` without `--as`** prints a clap missing-argument error rather than
  generic help.
- **`dag`** no longer excludes `in-progress` issues from the spawnable set, and no
  longer lists closed/terminal issues in its unscheduled output.
- **pi-corpus data-safety** — refuses directory-symlink traversal out of the
  corpus root; no longer misclassifies metadata errors as `Missing` (which could
  drop a manifest row); the "skills mirrored" hint is gated on the mirror block
  actually running.
- **CI flakiness** — deflaked the rate-limit and flock write-lock tests.

### Changed
- **`sync-commits` warns when the default range is empty on `main`** (a common
  silent trap where the default `merge-base..HEAD` becomes `HEAD..HEAD`); the
  warning surfaces in both text and `--json` output.
- **Action-verb `--json` results echo the mutated field** (`status` / `priority` /
  `labels`) so a caller can confirm a write without a follow-up `show`.
- **`note` / `close --as`** strips a single leading `@` from the author instead of
  rejecting it.
- **`dag` reservations** accept a `run_id` object shape, not only an array of holds.
- Internal: collapsed the single-implementation `ConfigSource` seam and made
  schema/transitions load return by value now that the per-request cache is gone.

## [0.10.0] - 2026-08-12

### Removed
- **The web UI and its entire HTTP surface (breaking).** `issuectl serve` —
  the local Trello-style kanban web board — is gone, along with the web
  server, all `/api/*` endpoints (issue list, board views, PATCH/POST write
  paths), the kanban frontend and its static assets, the live-reload file
  watcher / SSE edit-sync machinery, user-defined boards
  (`.issuectl/boards/`), and the server-only `RepoConfigCache` per-request
  schema/transitions cache. `issuectl` is now a pure AI-first CLI: the
  domain, mutate, schema, and all `cmd_*` paths are unchanged. The bundled
  `issuectl docs kanban` topic (and the now-empty `issuectl docs` command)
  are removed with it. Dropped the web-only dependencies (`axum`, `tokio`,
  `tokio-stream`, `futures-util`, `ammonia`, `notify`,
  `notify-debouncer-full`, `parking_lot`, `uuid`). (issue: `@remove-web-ui`)

## [0.9.0] - 2026-08-12

### Added
- **Authoring-time warning for the reserved `## Notes` section.** `issuectl
  new` and `issuectl body set` now warn (non-fatally, in both human and
  `--json` `warnings` output) when the supplied issue body contains the
  reserved legacy heading `## Notes` (which `doctor` migrates to
  `## Comments`), so the collision surfaces immediately instead of only at
  commit time via the pre-commit `doctor` hook. The write is never blocked.
  (issue: `@warn-reserved-notes-section`)
- **Codex-prompt variants of `/issue-new` and `/issue-intake`.** Both intake
  skills now install in both formats like `/issue`: a Claude skill under
  `.claude/skills/<name>/SKILL.md` and a Codex prompt under
  `.codex/prompts/<name>.md` (frontmatter stripped, body identical).
  `--agent all` installs both; the dogfood sync test now enforces all six
  copies. (issue: `@codex-prompt-variants`)
- **Dual-home skills into pi.dev's global corpus.** `issuectl skill install`
  and `issuectl init` now also write each Claude `SKILL.md` to
  `~/.pi/agent/skills/<name>/SKILL.md` (home-global, resolved via
  `skill::pi_skills_root`) so the skills are discoverable under the pi.dev
  harness (`/skill:<name>`). Only `SKILL.md` is mirrored (vendored filter);
  the repo-local Claude/Codex targets are unchanged, and the pi mirror is
  non-fatal (a failed home write never breaks the repo-local install).
  (issue: `@pidev-dual-home-skills`)
- **`doctor --fix` auto-merges `## Notes` into `## Comments`.** When an issue
  body contains both sections, `doctor --fix` now merges `## Notes`' entries
  into `## Comments` (document order preserved, `## Notes` dropped) instead of
  demanding a manual merge. Ambiguous shapes (multiple `## Notes`, or
  `## Notes` + multiple `## Comments`) still surface as manual-merge conflicts.
  (issue: `@doctor-fix-merge-notes-comments`)

### Fixed
- **Deterministic rate-limit test.** The token-bucket limiter's clock is now
  injectable (`SystemClock` in production, a frozen clock in tests), fixing a
  CI flake where `put_body_rate_limit_fires_with_retry_after` could see a
  burst request still return `200` instead of `429` under load. Production
  behaviour is unchanged. (issue: `@rate-limit-test-flaky`)

## [0.8.1] - 2026-08-10

### Added
- **`issuectl dag`: stable intra-lane ordering via an optional `lane_seq`
  key.** A new optional numeric frontmatter field `lane_seq: <int>` is
  consulted **after** the `blocked_by` topological order but **before** the
  slug lexical tie-break, so a lane's soft human precedence no longer has to
  be faked as a `blocked_by` edge (or left to alphabetical slug order).
  Absent → previous behaviour. Written with `issuectl update --lane-seq <int>`
  / cleared with `--no-lane-seq`. Additive optional v1 field (projected into
  `canonical_hash` only when set; no `SUPPORTED_SCHEMA_VERSION` bump). (issue:
  `@dag-stable-intralane-order`)
- **`issuectl dag`: `lane: unlaned` parallel-safe sentinel.** A first-class
  "confirmed parallel-safe" marker, distinct from an **absent** lane
  (unclassified): issues tagged `lane: unlaned` are each independently
  spawnable and **never serialized with siblings** that share the sentinel —
  the opposite of a normal shared lane. (issue: `@dag-unlaned-parallel-sentinel`)

### Changed
- **Release path switched to the `ossctl` engine (`/oss-release` →
  `ossctl release plan|cut`).** `ossctl release cut` owns the crates.io
  publish of both crates (adapter `cargo-publish`), the `vX.Y.Z` tag, and
  the binary dist trigger, per the approved `OSS-RELEASE.md` contract — it
  publishes **the version already in `Cargo.toml`**, so the `version` bump
  and `CHANGELOG` finalize are made in the `release: X.Y.Z` commit **before**
  the cut (they are not done by ossctl). cargo-dist
  `.github/workflows/release.yml` is kept as a
  binary-only backend (GitHub-Release binaries + shell installer + Homebrew
  tap), fired by the tag ossctl pushes. Retired the broken
  `.github/workflows/publish-crates.yml`, whose `release: [published]`
  trigger never fired (cargo-dist publishes the Release with `GITHUB_TOKEN`,
  which does not fire downstream workflows) — the crates.io publish now
  lives in `ossctl release cut`, wiring out the standing manual-trigger gap
  (verified by `ossctl release plan` dry-run; the first real release under
  this path is its end-to-end proof). Docs updated (`OSS-RELEASE.md`,
  `AGENTS.md`, `CONTRIBUTING.md`). No behavioural change to the `issuectl`
  binary. (issue: `@wire-oss-release-as-release-path`)

### Changed
- **`issuectl dag` treats an `in-progress` issue as spawnable.** Design
  correction: `in-progress` means *started, not done* — not "someone is on
  it right now". `dag` is consulted only when nothing is actively running
  ("what's next?"), so under that invariant an in-progress head is an idle,
  half-done, *resumable* candidate that must be surfaced, not hidden.
  Preventing two workers on the same issue is the caller's reservation/claim
  responsibility (feed the held lane/collision tokens back via
  `--reservations`), not the dag's. This supersedes the earlier (unreleased)
  `!underway` exclusion. (issue: `@dag-inprogress-is-spawnable`)

## [0.8.0] - 2026-08-10

### Added
- Two optional per-issue scheduling fields — `lane:` (a spawn-time
  mutual-exclusion group) and `collision:` (extra shared "hot file" tokens)
  — plus `issuectl dag [--json]`, which renders the scheduling DAG by
  joining lane + a deterministic per-lane order + the `blocked_by` mirror
  with live status, computing head-of-line and spawnability on read
  (nothing stored). Set them with `update --lane` / `--no-lane` and
  `--add-collision` / `--remove-collision` (reserved from `set` /
  `--field`). `dag --reservations <file|-|json>` lets a caller feed in the
  lane/collision tokens in-flight runs hold so spawnability is accurate
  without issuectl coupling to any orchestrator. Both fields follow the
  typed `closed_by` precedent: absent-by-default and projected into
  `canonical_hash` only when set, so existing issues keep their version
  tokens. Consumers can retire a hand-maintained `## Execution DAG`
  markdown block in favour of the computed view. (issue:
  `@dag-scheduling-view`)

### Changed
- `issuectl new "<title>"` without `--slug` now derives a descriptive 2–3
  word kebab slug from the title (lowercase, stop-words dropped, apostrophes
  elided, ASCII words only) instead of minting a random
  `intensifier-adjective-noun`. The derived path has its own numeric-suffix
  dedupe (`base`, `base-2`, … up to 99). The random form stays reachable via
  the new `--slug-random` flag (mutually exclusive with `--slug`) and remains
  the automatic fallback when a title is unsluggable (empty, all stop-words,
  non-ASCII) or the derived namespace saturates. `--slug <x>` is unchanged.
  Intake (untrusted titles) and recurring occurrences keep the random form
  deliberately. (issue: `@default-slug-from-title`)

## [0.7.2] - 2026-08-10

### Added
- `issuectl close --comment/--note "<text>"` attaches a timestamped
  `## Resolution` block to the issue body in the same atomic write, composing
  with `--status` / `--as` / `--commit`. (issue: `@close-comment`)
- `closed_by` is now a typed first-class field rather than an `extra` map
  entry: it is folded into `canonical_hash` (back-compat preserved), validated
  by the schema, `doctor`-healed on active statuses, and surfaced in both
  `show --json` (top-level) and human `show` ("Closed by:"); human `close`
  echoes "(by <author>)" under `--as`. Legacy `extra["closed_by"]` migrates on
  read. (issue: `@intensely-blushing-galley`)

### Fixed
- `--json show`, `ls`, and `search` now surface `blocked_by` as a top-level,
  canonical `@`-prefixed list (plus a derived `blocks` reverse view on `show`)
  instead of burying it under `.extra`, where the top-level `.blocked_by` read
  as `null` on every path. Programmatic consumers reading `.blocked_by` now get
  the real value uniformly; a shared `project_blocked_by` projection keeps a
  single representation on the wire. (issues: `@show-json-omits-blocked-by`,
  `@json-blocked-by-null-top-level`)

## [0.7.1] - 2026-08-06

### Added
- **`skill install` (and `init`) now distribute the intake-flow skills.** In
  addition to the `/issue` skill, `issuectl skill install` and `issuectl init`
  install the `/issue-new` (filer) and `/issue-intake` (queue processor) skills,
  shipped as version-pinned `include_str!` templates and dogfood-guarded so the
  installed copies cannot drift from their templates — the same contract `/issue`
  already had. This lets the whole intake workflow travel with the binary to every
  project that installs it, instead of living only in this repo. Shipped in the
  Claude-Code format; a Codex prompt variant is a follow-up.

## [0.7.0] - 2026-08-05

### Added
- **Standard intake flow for bugs and feature-requests.** A first-class
  `issuectl intake` command group replaces the ad-hoc, label-encoded
  Telegram bug path. Filing side: `intake file` (guarded surface, idempotent
  on `(provenance, source_ref)`) and `intake withdraw`. Processing side:
  `intake queue` / `intake show`, and the disposition verbs `accept`,
  `defer`, `need-info`, `reject`, `cannot-reproduce`, `duplicate`,
  `obsolete`, `retype`, `reopen` — each a first-class domain mutation that
  validates its source state (intrinsic invariants that hold with or without
  `transitions.yaml`). New statuses `untriaged`, `deferred`, `needs-info`
  (all `active` class); new fields `provenance` (repo-configurable value
  set), `disposition_reason`, `duplicate_of`, `source_ref`, `deferred_until`.
  A complete default intake transition matrix plus a code-level type×status
  compatibility check (a bug completes as `fixed`, non-bug work as `done`).
  New `--json` error codes: `transition-illegal`, `duplicate-source-ref`,
  `protected-field`. (issue: `@standard-intake-flow`)
- **`issuectl intake migrate`** — a dedicated, dry-run-first, idempotent,
  per-issue-atomic pass that migrates legacy `needs-triage` / `deferred` /
  `via:telegram` label-encoded state onto the new statuses/fields, refusing
  ambiguity rather than guessing and never regressing closed or in-flight
  items. `intake queue` surfaces recognised legacy items (`legacy: true`)
  until migration completes. (issue: `@standard-intake-flow`)
- **`/issue-new` and `/issue-intake` skills.** `/issue-new` files a report
  with one validated CLI call; `/issue-intake` works the queue (drives
  `/worktree-bug-analysis` as its read-only analysis engine) and **replaces
  `/triage-bugs`**, which becomes a thin deprecating alias. (issue:
  `@standard-intake-flow`)
- `issuectl close` accepts an optional `--as <author>`, recording the closer
  as a first-class managed `closed_by` field — mirroring how `note`
  attributes an author, so lifecycle transitions can carry attribution.
  (issue: `@close-as-flag-asymmetry`)
- `issuectl new --body-file <path>` (with `-` for stdin) sets the initial
  body. (issue: `@new-body-flag`)

### Fixed
- Write verbs (`update` / `close` / `set` / `note` / `check` / `label` /
  `depend` / `body set`) now return a stable `error.code: "not-found"` in
  their `--json` envelope on a missing slug, instead of the generic
  `command-failed`, matching the read paths. (issue:
  `@mutation-not-found-classification`)
- The Refs-Issue reminder moved from the pre-commit hook to the commit-msg
  hook, so it reads the final message via `git interpret-trailers` and no
  longer false-fires on `-F` / stdin commits. (issue:
  `@refs-issue-hint-false-fire`)

## [0.6.6] - 2026-08-04

### Added
- `low` priority value — `--priority` now accepts `low`, `normal`, `high`
  (default still `normal`). Ordering is presentation-only; no priority
  ranking is implied. Repos needing finer gradations still widen the enum
  per-repo via `issues/.schema.yaml`. (issue: `@add-low-priority-value`)
- `issuectl new` accepts a **positional title** — `issuectl new "Some title"
  --type feature` now works, matching how `note` / `search` take positional
  text. `--title` remains the canonical flag; passing both or neither errors
  clearly. (issue: `@cli-ux-subcommand-friction`)

### Changed
- **`--expected-version` is now optional (opt-in) on `--json` writes.** The
  mutating verbs (`update`, `close`, `set`, `note`, `body set`, …) with
  `--json` no longer *require* `--expected-version`; the write proceeds
  without it, symmetric with the non-`--json` path (`flock` still prevents
  corruption in both). When `--expected-version` **is** passed it is still
  enforced as a compare-and-swap — a stale/mismatched token still fails —
  so callers that want lost-update protection opt in. Write result objects
  now carry the new canonical `version` at a stable top-level key, matching
  `show --json`, so the read-back round-trip is unambiguous. This supersedes
  design decision D4=B. (issues: `@json-close-requires-expected-version`,
  `@json-update-expected-version-ergonomics`)
- `issuectl note` flags are order-insensitive, and omitting a required
  `--as` now produces a targeted error naming `--as` instead of a generic
  usage error. (issue: `@cli-ux-subcommand-friction`)
- Passing a built-in list field positionally to `set` (e.g. `set <slug>
  related <ref>`) now errors with a hint naming the flags that actually work
  (`update --add-related` / `--remove-related`) instead of the
  self-contradicting `--related (repeatable)`. (issue:
  `@cli-ux-subcommand-friction`)

### Fixed
- `issuectl note --from-file` no longer rejects legitimate `##` / `###`
  Markdown headings outside code fences. Such headings are demoted
  (`##`→`####`, `###`→`#####`) when appended under the managed `## Comments`
  section, so structured note content is preserved without corrupting
  section parsing. (issue: `@note-from-file-rejects-headings`)

## [0.6.5] - 2026-07-27

### Added
- `assign <slug> <user>` subcommand — a convenience wrapper that routes
  through the existing typed `set --assignee` path (identical validation
  and idempotency, no new storage semantics). Use `assign <slug> --clear`
  to unassign. (issue: `@assign-subcommand-alias`)
- `create` is now accepted as an alias for `new`, matching the near-
  universal "create" verb (git / gh / kubectl / docker). (issue:
  `@verb-alias-discoverability`)
- `new` now accepts `--body` as an alias for `--description`.
  (issue: `@verb-alias-discoverability`)
- Running `issuectl body <slug>` (a bare slug where the `body` subcommand
  group expects a sub-subcommand) now emits a hint pointing at
  `body set <slug>` instead of a bare "unrecognized subcommand" error.
  (issue: `@verb-alias-discoverability`)

### Fixed
- `rename <old> <new>` now updates the renamed issue's own `slug:`
  frontmatter field, not just the directory name and inbound
  cross-references. Previously a renamed issue that carried a `slug:`
  field (e.g. one stamped by `doctor --fix`) was left holding the old
  slug, silently out of sync with its directory. (issue:
  `@rename-stale-self-slug`)

## [0.6.4] - 2026-06-03

### Fixed
- `doctor`'s `broken_attachment_refs` scan no longer flags
  `![alt](path)` / `[text](path)` syntax that appears inside markdown
  code spans or fenced code blocks (the author is naming a construct,
  not using one). The scan now walks the issue body with a CommonMark
  parser (`pulldown-cmark`) and only inspects link/image targets
  emitted as real `Tag::Link` / `Tag::Image` events.
- `doctor` also stops flagging repo-relative source-code cross-links
  (e.g. `[foo.ts:87-98](../foo.ts#L87-L98)`) as broken attachments.
  Targets whose path component contains `/` and whose fragment matches
  the `#L<n>` / `#L<n>-L<m>` line-anchor shape are treated as
  cross-file code pointers and skipped. Genuine sibling attachments
  (`![screenshot](missing.png)`) still error as before.

## [0.6.3] - 2026-06-01

### Changed
- Strengthen the descriptive-slug guidance so agents stop defaulting
  to the random `intensifier-adjective-noun` slug when an obvious
  short slug exists. The `/issue` skill's `Identifiers` section now
  leads with "prefer a descriptive 2-3 word slug derived from the
  title; random is the fallback" (previously it stated random as the
  norm and buried the policy in the Create action). The same flip is
  applied to the Codex prompt. The `issuectl new` clap help text and
  top-level usage example also frame `--slug` as the recommended path
  with random as the fallback instead of an "override".

## [0.6.2] - 2026-06-01

### Fixed
- `doctor --fix` no longer silently no-ops on alias coercion and
  `.issuectl/AGENTS.md` schema-block regeneration when any issue has
  an unmergeable `## Notes` / `## Comments` body. Notes/comments
  conflicts (and malformed/check-skipped `AGENTS.md`) are now
  surfaced as per-file findings rather than aborting the entire
  apply pass at preflight. The human end-of-run summary is coherent
  with the actual outcome (`Applied.` / `Partial — …` /
  `Refused — …`), and `--json --fix` on a non-zero exit now emits the
  documented `{"error":{"code":"doctor-blocked"|"doctor-partial"|…,
  "message":"…","details":{…}}}` envelope on stderr instead of a
  contradictory result object on stdout. Read-only `--json doctor`
  is unchanged — it still emits the full result on stdout regardless
  of exit code, so existing `issuectl --json doctor | jq …` scripts
  are unaffected. (issue: `@doctor-fix-noop`)

## [0.6.1] - 2026-06-01

### Changed
- `/issue` skill (and Codex prompt) now treats a newer-than-pinned
  `issuectl` binary as a signal to refresh the skill itself: the
  install/upgrade section recommends `issuectl skill install --force`
  and `issuectl doctor` so the agent's instructions and the repo's
  schema both catch up to the binary it's actually talking to.

## [0.6.0] - 2026-05-31

Two `/orchestrate` campaigns (14 + 9 worker nodes) landed 23 issues
under v0.6.0. Theme: CLI-mode improvements — no kanban / web-board
work in this release. See `docs/releases/v0.6.0.md` for the
human-readable digest.

### Added
- `issuectl rename <old-slug> <new-slug>` rewrites every reference
  to the renamed slug across the repo: `epic:`, `related:`,
  `blocked_by:`, `@slug` body references, recorded `commits:`, board
  configs, and cached prompt bundles. Doctor flags any leftover
  dangling references. Archived issues are renamed in place.
- First-class issue attachments and fixtures.
  `issuectl attach <slug> <file>...` copies files into
  `issues/<slug>/attachments/`; `--fixtures` targets
  `issues/<slug>/fixtures/`. Body references use relative paths so
  they survive renames and archive moves. Doctor warns on
  path-traversal patterns and oversized binaries.
- Issue lifecycle: stale detector and auto-archive.
  `issuectl stale [--days N]` reports open issues with no recent
  updates; `issuectl archive [--older-than N]` moves closed issues
  to `issues/archive/YYYY/MM/`. All read commands (`ls`, `show`,
  `search`, `metrics`, …) now consult both the active root and the
  archive root, so archived issues stay findable but don't clutter
  default listings. Renamed-then-archived issues are followed
  correctly.
- Heuristic local-only duplicate detection. `issuectl duplicates`
  reports likely-duplicate open issues using title-token overlap,
  shared labels, and body-token similarity — no AI, deterministic,
  offline, repository-local.
- Issue import/export. `issuectl import json|github` ingests issues
  from JSON dumps or GitHub Issues (via `gh`).
  `issuectl export json|csv|markdown` writes a portable snapshot of
  issues matching the given query.
- Schema-driven agent instructions in the context bundle.
  `issuectl context <slug>` now injects schema-declared constraints
  (enum value lists, estimate rules, reserved keys) into the
  rendered prompt as system instructions, so AI agents working from
  the bundle can't invent values outside the schema. Long enums are
  capped and summarized rather than dumped wholesale.
- Custom-field plumbing. Unknown frontmatter fields now flow through
  `Issue.extra` instead of being silently dropped, and
  `context::read_blocked_by` consumes the parsed model directly —
  closing the previous TOCTOU window from a redundant second
  `item.md` read.
- Declarative schema rules. `required_when` in `issues/.schema.yaml`
  expresses field-level conditional requirements (e.g. a closing
  status implies `closed:` must be set); validated by `doctor` and
  at mutation time. `status_aliases` / `type_aliases` map legacy
  enum values to current ones; `doctor --fix` auto-coerces them
  during migration. Alias targets are validated against the field
  enum and alias chains are rejected.
- `issuectl note <slug> --stdin` / `--from-file PATH` accept the
  note body from stdin or a file; the same hardening covers
  `issuectl body set`.
- `issuectl open <slug>` launches the issue's `item.md` in `$EDITOR`
  (or `--editor <CMD>`); `--dir` opens the issue directory instead.
- `validation.md` and `breakdown.md` doc-types added to the
  init-project planning-document template and the AGENTS.md
  convention list.
- Git-derived reporting commands. `issuectl activity [--since 7d] [--limit N]`
  lists recent commits that touched `issues/`, grouped back to slugs.
  `issuectl timeline <slug>` reconstructs status transitions from
  `git log -p` on the issue's `item.md` (uses a `:(glob)` pathspec so
  legacy `open/`/`closed/` layouts and archive moves are included).
  `issuectl changelog <ref>..<ref>` walks `Refs-Issue:` / `Fixes-Issue:`
  trailers in the range and emits a markdown release-note table grouped
  by issue type. `issuectl metrics [--since 30d]` computes throughput,
  median/p90/mean cycle time from frontmatter `created`/`closed`, and
  open/closed workload by effective assignee. All four commands honour
  `--json`. Git history is the event log — there is no event database;
  when rebases/squashes reshape it, frontmatter timestamps are
  authoritative.

### Documentation
- Document the rationale for the two-value priority enum
  (`normal`/`high`) in `docs/design/frontmatter-schema.md`, the
  `PRIORITIES` constant, and `issuectl new --help`: triage cost,
  why `low` and `critical` are intentionally omitted, and how to
  widen the enum per-repo via `issues/.schema.yaml`.

### Added
- Lightweight estimates. Optional `size: S|M|L|XL` (schema-enforced enum)
  or free-form numeric `estimate:` frontmatter on any issue — schema
  allows either, not both per issue. New `issuectl workload` aggregates
  open + in-progress issues by assignee, priority, cycle, and epic,
  summing point-equivalents (S=1, M=3, L=5, XL=8 for sizes; `estimate:`
  used verbatim) and surfacing how many issues are unestimated. New
  `issuectl burndown --cycle <name>` prints an ASCII burndown across
  the cycle's days; ISO-week labels (`YYYY-Www`) span Mon→Sun, other
  labels fall back to earliest-`created` → today. Closed issues
  subtract their points on the `closed:` date. `cycle current` is a
  valid `--cycle` value. Both commands honour `--json`.
- Optional `reviewer:` and `review_status:` frontmatter fields for teams
  that review through git/PRs but want issue-level review visibility.
  `review_status` is enum-validated (`requested` / `in-review` / `approved`
  / `changes-requested`); `reviewer` is a free-form username that `issuectl
  doctor` validates against the repo's known-user universe (any name that
  appears as `reporter`/`assignee`/`owner` on at least one issue) and
  surfaces under "Unknown reviewers" when it doesn't. The query language
  (`issuectl ls/search/export/bulk`, web `?q=`) gains `reviewer:`,
  `review_status:`, `reviewer:any`, `reviewer:none`; CLI queries also
  support `reviewer:me` / `assignee:me` / `owner:me`, resolved via
  `$ISSUECTL_USER` → `$GIT_AUTHOR_NAME` → `$GIT_COMMITTER_NAME` → `git
  config user.name`. Set the fields via `--field reviewer=alice --field
  review_status=requested` (no dedicated flags in this iteration).
- Canonical dependency tracking via `blocked_by:` frontmatter. The
  reverse `blocks` relationship is **derived at runtime** by scanning
  every issue's `blocked_by` — it is intentionally never stored, which
  avoids the drift class. New `issuectl depend add/remove <slug>
  --blocked-by <other>` mutation goes through the same flock + schema
  validation path as `update`. The mutation rejects self-blockers
  (`depend add foo --blocked-by foo`) and overlap between `--blocked-by`
  add / remove sets. The list-mutation slot (`add_blocked_by` /
  `remove_blocked_by`) is also wired into the JSON `PATCH /api/issues`
  body for parity. The query language gains `blocked_by:<slug>` /
  `:any` / `:none` (resolved per-issue) and `blocks:<slug>` / `:any` /
  `:none` (resolved from the precomputed repo-wide blocker graph;
  `query::matches_with` + `query::MatchCtx` carry the graph through).
  `issuectl doctor` keeps detecting missing blocker refs and dependency
  cycles via `blocked_by_cycles` and now reports self-dependencies
  separately under `blocked_by_self` so the error message points at the
  fix. The agent context bundle already surfaces the blocker summary in
  `Bundle.blocking_issues` (used by `issuectl context`). The schema's
  reserved-key list adds `blocked_by` with a hint pointing at `issuectl
  depend`, so `--field blocked_by=...` and the equivalent PATCH custom
  field are rejected with that hint.
- `issuectl schedule` subcommand for recurring / scheduled issues.
  Definitions live at `.issuectl/recurrences/<name>.yaml` (title,
  cron `schedule`, optional `type`/`priority`/`labels`/`assignee`/
  `reporter`/`description`). `issuectl schedule list` reports loaded
  definitions and their materialization state; `issuectl schedule
  run` materializes a new issue per due cron fire, stamping
  `recurrence_of: <template>` and `occurrence: <ISO8601>`
  frontmatter. The manifest at
  `.issuectl/recurrences/.manifest.yaml` dedupes occurrences and
  tracks each definition's `last_fire` cursor — closing an instance
  has no effect on the next one (per-occurrence file, never
  overwrite). First sight of a definition only "subscribes" the
  cursor at `now` so installing a new definition doesn't
  retro-materialize history; subsequent fires materialize on the
  next `run`. Catch-up is capped at 50 occurrences per run. Cron
  expressions accept either standard 5-field (`min hour DoM mon
  DoW`) or 6/7-field with explicit seconds. **All schedules are
  evaluated in UTC** — a future enhancement may add a per-def
  `timezone:` field; for now express times in UTC. The
  `recurrence_of` and `occurrence` keys are written as custom
  frontmatter, so a repo schema that forbids extra keys must
  declare them explicitly.
- Markdown Definition-of-Done validation. New `issuectl-core::body` module
  parses `- [ ]` / `- [x]` task lists in the canonical H2 sections
  `## Acceptance Criteria`, `## Tests Run`, and `## Implementation Notes`
  (fence-aware: checkboxes inside fenced code blocks are content, not
  task-list items). A new `issuectl ready <slug>` command reports the
  completion state of those sections (exits 0 when Acceptance Criteria is
  fully checked, 1 otherwise; `--json` for machine output). On a transition
  *into* a closing status, an unchecked Acceptance Criteria section now
  surfaces a warning on stderr / in `UpdateOutcome.warnings`; set
  `dod.strict: true` in `issues/.schema.yaml` to upgrade the warning to a
  blocking error. Existing per-rule `requires_acceptance_criteria_checked`
  remains an opt-in always-block.
- `issuectl cycle` subcommand for Linear-style lightweight cycles
  (iterations). Issues opt in via an optional `cycle:` frontmatter label
  (e.g. `cycle: 2026-W22`) — no schema-side cycle catalog, no start/end
  dates. `issuectl cycle current` prints today's ISO-week label;
  `issuectl cycle plan <name>` lists planned issues; `issuectl cycle
  status [<name>]` rolls up open/closed counts (defaults to the current
  cycle, `--all` lists every distinct cycle). All subcommands honour
  `--json`. The literal `current` is accepted as an alias for the
  current-cycle label so scripts can avoid a second `cycle current`
  call.
- **Inbox drafts**: `issuectl new --inbox` drops the new issue under
  `issues/inbox/<slug>/` instead of the canonical flat root, so half-baked
  ideas have a low-friction landing zone. `issuectl triage` (no args) lists
  every inbox draft; `issuectl triage <slug>` promotes one to
  `issues/<slug>/`. Inbox drafts are hidden from `ls` by default; `mutate`
  verbs still work on them (so a draft can be iterated in place before
  triage).
- **Slug prefix matching**: `issuectl show extremely` (and every other
  per-slug command) now resolves `extremely` to `extremely-quiet-otter`
  when the prefix is unique. An ambiguous prefix lists the candidates;
  no-match errors as before. The expansion lives in
  `repo::resolve_slug_input`, called from the central `locate_issue_full`
  path so every command benefits without per-command plumbing.
- **`issuectl pick`**: fuzzy-pick an issue for piping into other commands.
  Without QUERY, lists open issues for interactive selection; with QUERY,
  filters by substring across slug + title + labels. A unique match prints
  immediately (non-interactive). The interactive menu goes to stderr so
  stdout stays clean for `issuectl pick | xargs issuectl show`.
- **`issuectl completions {bash,zsh,fish,powershell,elvish}`**: prints
  shell completion scripts (via `clap_complete`). Paired with the hidden
  `issuectl _complete <kind>` helper (`slugs`, `slugs-all`, `statuses`,
  `labels`, `users`), shell users can wire dynamic value completion.
- **`issuectl scan-todos`**: walks repository source and reports
  `TODO(issue: <slug>)` markers, classifying each hit as `tracked`,
  `stale` (slug → closed issue), `unknown`, or `untracked`.
  `--create-inbox` materialises a fresh inbox draft per untracked hit so
  the user can later `issuectl triage` and refine it.
- `issuectl bulk '<query>'` applies one mutation to every issue matching a
  query (same syntax as `ls`/`search`), in a single batch the user commits
  together. Supports `--set key=value` / `--clear key` (built-in fields route
  through their typed slots, anything else is a custom field) plus
  `--add-label` / `--remove-label` / `--add-related` / `--remove-related`.
  `--dry-run` prints the affected slugs and a per-issue unified diff without
  writing. The whole batch runs under a single repo-wide lock
  (`mutate::bulk_update`): every target is validated before any write lands,
  so a bad value writes nothing and no concurrent writer can race between a
  target's validation and its write.

### Changed
- Unified `--json` output envelope across mutating commands. Clap
  usage errors now also flow through the JSON envelope, and the
  partial-success contract for batch operations is documented:
  per-target outcomes carry explicit success/failure markers.
- `issuectl new --slug <existing>` now fails with an actionable error that
  names the colliding slug and path and suggests retrying with a different
  `--slug` or omitting it for a random auto-generated one (was a terse
  `target directory already exists: <path>`).
- `/issue` skill now derives a descriptive 2-3 word `--slug` from the issue
  title on create, falling back to the random `intensifier-adjective-noun`
  slug only when no obvious short slug exists. The CLI default (random slug
  when `--slug` is omitted) is unchanged.

## [0.5.2] - 2026-05-13

Bug-fix and quality-of-life release driven by the downstream monorepo
adoption feedback plus an internals cleanup wave. No breaking changes.
Highlights:

- One-command bootstrap: `issuectl init`.
- Git-native commit linking: `Refs-Issue:` / `Fixes-Issue:` trailers
  + `issuectl sync-commits`.
- `doctor --fix` no longer all-or-nothing — flat-layout migration
  runs regardless of unrelated schema findings, and
  `issues/.schema.yaml` bootstrap is unconditional.
- Frontmatter parsing hardened against fenced YAML in bodies; short
  hashes (`315194e2`) are always quoted; UTF-8 titles render cleanly
  in `issuectl list`; `agents init` logs which schema it used; doctor
  flags gitignored canonical files.
- Internals: thread-local config cache replaced with explicit
  injection; `parse_section` returns structured diagnostics; PATCH/PUT
  writes get `AbortController`/timeout; canonical version tokens carry
  a `sha256:v1:` scheme marker.

### Added
- `issuectl init` — one-command bootstrap for a fresh repo. Runs the
  schema scaffold, `.issuectl/AGENTS.md`, and the `/issue` skill
  (Claude + Codex by default; override with `--agent`). Pre-commit
  hook (`--with-hooks`) and YAML merge driver (`--with-merge-driver`)
  are opt-in. Idempotent: re-running on an already-initialized repo
  reports each step as "already exists" and exits 0. `--json` emits
  a structured `steps[]` summary with per-artifact status, typed
  effects (e.g. `git_config` writes), and machine-parseable
  `next_steps[]`. `--force` regenerates the managed block in
  `.issuectl/AGENTS.md` while preserving user prose, and overrides
  refusal to clobber an existing differing
  `merge.issuectl-yaml.driver` git-config value. Quotes the binary
  path in the merge-driver invocation so installs survive paths with
  spaces. (#totally-protective-wing)
- `issuectl sync-commits` walks `git log <range>` (default
  `<merge-base of HEAD and main/master>..HEAD`) and appends commits
  to each issue's `commits[]` based on `Refs-Issue:` / `Fixes-Issue:`
  trailers in the message body. Idempotent — `write::add_commit` now
  skips entries whose hash (or hex-prefix-equivalent abbreviation)
  is already present, so re-running the same range is a no-op.
  `--dry-run` previews the plan; `--no-branch-fallback` disables the
  implicit "branch named after a known slug → attribute commits to
  that slug" attribution. `Fixes-Issue:` triggers a stderr `Hint:`
  suggesting `issuectl close <slug>` rather than auto-transitioning.
  The opt-in pre-commit hook also prints a non-blocking reminder
  when the current branch resolves to a known slug, nudging the
  user toward `Refs-Issue:` trailers. (#strikingly-absorbing-cows)
- `issuectl agents init` now logs which schema source it used —
  `Using project schema at issues/.schema.yaml.` or `Using built-in
  default schema (issues/.schema.yaml not found).` — so the silent
  default-fallback path is no longer invisible. `--json` includes a
  `schema_source: "default" | "project"` field in the same envelope.
  (#eminently-dramatic-anger)
- `issuectl doctor` warns when canonical issuectl-tracked files
  (`.issuectl/AGENTS.md`, `issues/.schema.yaml`) match a `.gitignore`
  pattern. Asymmetric footgun: works locally, fails for teammates and
  CI. Surfaced in `--json` as `gitignored_paths: [...]`.
  (#simply-workable-umbrella)

### Changed
- `commits[].hash` quoting is now applied at the AST level (only to
  the `commits` sequence), not via post-serialization line rewriting.
  Top-level / nested user `extra` fields named `hash` are no longer
  silently coerced to strings.
- `read_item` now goes through the strict shared splitter
  (`item_text::split`); previously it kept its own naive scanner that
  could disagree with the parser/doctor and corrupt files on write.
- `doctor` no longer passes `--no-index` to `git check-ignore`, so
  tracked files matching a `.gitignore` pattern are not falsely
  flagged as "agents on other machines won't see it".
- `item_text::split` strips a leading UTF-8 BOM so editor-prepended
  `\u{feff}` no longer breaks frontmatter detection.
- `agents init` schema-source detection is now a typed `SchemaSource`
  enum tested directly; the previous string-compare branched
  silently on unknown variants.
- `truncate` returns the empty string for `max_len = 0` instead of
  `"…"`; documented that the helper counts scalar values, not
  terminal-display columns.
- `issuectl doctor --fix` no longer refuses the flat-layout migration
  when schema-shape findings (schema violations, broken cross-refs,
  dependency cycles, status/timestamp consistency) are present.
  Layout migration is a directory rename and is independent of
  frontmatter content; gating it on schema cleanliness was the
  largest single adoption blocker reported in downstream 0.5.1 feedback
  (240 dirs need layout migration, 216 schema violations — `--fix`
  would refuse until every violation was hand-fixed first against the
  pre-migration layout). Schema findings still drive exit-1 so they
  remain visible as forward work, surfaced against post-migration
  paths. Layout-fatal preflight blockers (flat-layout conflicts,
  duplicate slugs, slug present in both legacy folders, conflict
  markers, unparseable frontmatter, missing `item.md`, symlinked
  dirs, `## Notes` / `## Comments` ambiguity, malformed AGENTS.md,
  schema parse error) still bail with no writes. (issue:
  `@staggeringly-important-zoo`)
- `issuectl doctor --fix` now bootstraps `issues/.schema.yaml`
  unconditionally, even when other preflight blockers refuse the
  rest of the apply pipeline. The read-only output already
  advertised auto-creation on first `--fix`; gating bootstrap on an
  empty blocker list broke that promise. The operation is
  idempotent. (issue: `@unreasonably-attractive-star`)
- **JSON envelope:** `--json --fix` runs that hit a preflight
  refusal can now report `apply_outcome.fix_applied: true` when only
  the schema bootstrap landed. The `(stop_phase: "preflight",
  fix_applied: true, schema_bootstrapped: true)` combination is
  intentional and reflects the unconditional bootstrap behaviour.
  Scripted callers that previously read `fix_applied` as "every
  pending fix ran" should branch on `stop_phase` instead — `"ok"`
  remains the only phase that means "no blockers, pipeline
  completed".
- `issuectl doctor` collapses long warning lists (more than ten
  entries) to a one-line count by default. Re-run with `--verbose`
  to print the full list. The previous behaviour filled the screen
  with the same 240-line layout-migration list every iteration of
  "fix-something-rerun-doctor" loops. (issue:
  `@ridiculously-outrageous-fold`)

### Fixed
- `issuectl list` no longer panics on non-ASCII titles. The table
  truncation helper now operates on Unicode scalar values instead of
  byte indices, so titles like `Käyttäjän kirjautuminen…` render
  cleanly. Regression of the 0.3.1 byte-boundary issue that resurfaced
  when the table renderer was refactored.
  (#marginally-receptive-kettle)
- Frontmatter splitter is now fence-aware: a `---` line inside a
  fenced code block (` ```yaml ... ``` `) can no longer be mistaken
  for the closing frontmatter marker. Doctor's "unknown frontmatter
  key" warnings no longer fire on `shortname:` / `course_id:` /
  similar lines that live inside body code blocks. Routed all callers
  through the strict shared splitter in `item_text` so the reader,
  writer, formatter, and merge driver all agree on the same boundary.
  (#virtually-callous-rainstorm)
- Frontmatter writes now force-quote `commits[].hash` values so YAML
  1.2 implicit typing cannot coerce a short hash like `315194e2`
  into a float (`31519400.0`). Previously such hashes round-tripped
  losslessly only by accident of `serde_yaml`'s emission heuristics;
  the contract is now explicit.
  (#thoroughly-kaput-pocket)

### Internals
- Replaced the thread-local `RepoConfigCache` activation slot
  (`repo_config::enter` / `current` / `ActiveGuard`) with explicit
  dependency injection. Every mutate entry point
  (`update_issue`, `new_issue`, `update_body`, `close_issue`,
  `note_issue`, `toggle_checkbox`, `do_new`, `boards::load`) and
  the per-request server read path
  (`repo::load_issues_with_warnings_via`) now takes a
  `&dyn ConfigSource` parameter. The CLI passes `&UncachedConfig`;
  the server passes its `Arc<RepoConfigCache>` directly into
  `spawn_blocking`. Removes the `!Send` ambient guard, the
  spawn-blocking worker-reuse footgun, and the
  thread-local-vs-static accident risk; the cache now reaches the
  load site through the type signature so the failure mode for
  "forgot to install" is a compile error rather than a silent
  fallback to uncached parsing. `schema::load` and
  `transitions::load` no longer consult any cache and always
  re-parse — they remain as the CLI default; callers that want
  caching go through `ConfigSource::schema` / `::rules`.
  (#hugely-madly-haircut)
- `body_sections::parse_section` now returns a structured
  `ParsedSection { found, blocks, warnings }` (with a
  `duplicate_section_count()` accessor) instead of a bare
  `Vec<Block>`. The previous shape collapsed five distinct outcomes —
  section absent, present-but-empty, all-headings-malformed,
  swallowed-by-unclosed-fence, duplicate sections — into a single
  empty vec, which sister tickets (`decide`, `agent-run`) cannot
  tell apart. `ParseWarning::{MalformedBlockHeading, UnclosedFence,
  DuplicateSection}` carries the diagnostics. `MalformedBlockHeading`
  also carries `folded_into_previous_block: bool` so consumers can
  distinguish "content was preserved in the prior block's body" from
  "content orphaned before the first valid block." Success-case
  behaviour is unchanged: `.blocks` matches the prior return value
  byte-for-byte and a missing section still produces no warnings.
  (#totally-placid-push)
- Web client (`board.js`): PATCH and PUT write requests now run with
  an `AbortController` and a 30 s timeout each. A hung server
  previously left `pending_writes[slug] > 0` forever and queued every
  same-slug SSE event in `deferred_events` until the user reloaded;
  on timeout the request now aborts, the queue drains, and the user
  gets a "timed out" toast / save-status. A `pagehide` listener
  also aborts every outstanding fetch so a tab close stops the client
  from waiting on a response it can't act on. **Note:** aborting the
  client fetch does not stop the server: if the axum handler already
  received the request body and started a `spawn_blocking` mutation,
  that write will still land on disk. The aborted client just stops
  waiting and drains its pending-write state. (#absolutely-aberrant-caption)
- `parser::deser_epic` now strict-errors on malformed `epic:` shapes
  (sequence, mapping, bool, tagged value, empty string) instead of
  silently coercing them to `None`. The malformed value previously
  flowed through to disk via the raw mapping but never reached
  `canonical_hash`, leaving a blind spot for optimistic concurrency.
  `null` / absent / non-empty string / legacy numeric (for
  `doctor --fix` migration) remain accepted. Wraps through the existing
  typed-frontmatter fallback, so affected files surface as
  `fm_typed_error` and route to `MutateError::Corrupt`.
  (#especially-bumpy-way)
- Documented the watcher stale-snapshot race in
  `parse_slug_state` (concurrent PATCH bursts can publish V1 after
  V2 at a higher seq). Recovery is not spontaneous: the cached
  client state sticks at V1 until the user's next mutation hits a
  409 carrying the server's current state (which the SPA's
  conflict-recovery path re-syncs), or another filesystem event
  re-publishes. Decision is to monitor rather than fix preemptively
  — the window is narrow and local-loopback single-user usage rarely
  tickles it. Mitigations (hub-level version dedup, client-side
  recent-version cache, read-flock around parse) are catalogued in
  the doc comment. (#incredibly-real-hour)
- Canonical version tokens now carry a scheme marker:
  `sha256:v1:<64hex>` instead of `sha256:<64hex>`. Tokens are still
  compared as opaque strings on the hot path; the marker exists for
  forensics (logs and bug reports can distinguish schemes at a
  glance) and as the foundation for a later `classify(token)` helper
  if/when a v2 transition needs a typed "old-scheme" error path.
  **Deploy note:** existing browser sessions hold pre-upgrade tokens
  of the form `sha256:<hex>`; the first write attempt after upgrade
  will see a 409 `VersionMismatch`. The SPA's existing conflict
  handler refreshes from the server's `current` payload, so users
  experience this as a one-time "this issue changed externally"
  toast on their first save, not as lost work. Operators rolling
  out the new binary during active sessions should expect this
  one-shot 409. (#singularly-melodic-haircut)

## [0.5.1] - 2026-05-10

Cleanup release for the `issues/AGENTS.md` scaffold and skill version
hygiene. No behavior change in the CLI core.

### Changed
- `issues/AGENTS.md` template (installed by `issuectl skill install`)
  rewritten as a minimal pointer to the `/issue` skill and
  `.issuectl/AGENTS.md`. The pre-v0.5.0 scaffold described the
  numbered `<NN>-<slug>/` layout with `open/`/`closed/` subdirs and a
  sequential numbering section — none of which apply since v0.2.0
  (slugs) and v0.5.0 (flat layout). The new template is a few lines
  pointing readers at the supported tooling.
- `/issue` skill and Codex prompt templates now include a version-check
  section. The on-disk skill is pinned to the `issuectl` version that
  wrote it (`{{ISSUECTL_VERSION}}` substituted at install time); on
  first invocation in a session the agent runs `issuectl --version`
  and prompts the user to upgrade if the runtime is older. `skill
  install` and `skill print` both render the template with the current
  version.

### Added
- `issuectl doctor` flags a stale `issues/AGENTS.md` (pre-v0.5.0
  scaffold detected via legacy markers — `## Issue Numbering`,
  `open/<NN>` paths, etc.) and `--fix` rewrites it with the current
  pointer template. Customized `issues/AGENTS.md` files without legacy
  markers are left alone. Reflected in `--json` as
  `legacy_issues_agents_md` (read-only flag) and
  `apply_outcome.issues_agents_md_rewritten` (post-fix).
- `issuectl doctor` reports when `.issuectl/AGENTS.md` is absent so
  users can opt in via `issuectl agents init` (informational only;
  `--fix` does not create the file because the policy is opt-in).
  Reflected in `--json` as `agents_md_missing`.

## [0.5.0] - 2026-05-10

The "writable, agent-safe kanban" release. The web board moves from
read-only browser to a full editing surface; the CLI gains a
mutation/validation toolkit (doctor, fmt, set/note/check/label/apply,
transition rules, schema, AGENTS.md); query, search, and `?q=` share
one language; and the file format is hardened so concurrent edits via
web, CLI, `$EDITOR`, and `git pull` all converge.

**Breaking change:** repos using the legacy `issues/{open,closed}/<slug>/`
layout must run `issuectl doctor --fix` once. The new canonical layout
is flat — `issues/<slug>/item.md` — with status carried only in
frontmatter. `IssueSummary` (returned from `GET /api/issues` and
embedded in SSE `IssueUpserted` events) gained a non-optional `version`
field; consumers using `deny_unknown_fields` need to add it. The
`renumber` flow and `<NN>-` prefixes are gone.

### Added — Web edit/sync
- **Writable web kanban.** Drag-and-drop between status columns
  (closing-status picker on drop into Closed), inline frontmatter
  edits, and a body editor (textarea + preview, localStorage drafts,
  three-way merge UI on conflict). Optimistic UI with revert-on-failure
  and toast notifications.
- **Live updates.** Server-Sent Events (`/events`) push board mutations
  to all open browsers. EventHub (parking_lot mutex over seq+ring),
  notify-debouncer-full file watcher with consecutive-failure backoff,
  Last-Event-ID resume with scan-on-lagged semantics, `--watch-poll-ms`
  fallback, and a `Degraded` banner when the watcher gives up.
- **Concurrency-safe writes.** Every mutation goes through one shared
  `mutate.rs` (CLI and server), guarded by an `flock` on
  `.issuectl/write.lock`, with `expected_version` optimistic
  concurrency. CSRF + Host-header validation on the HTTP surface.

### Added — Agent-safe CLI
- **`issuectl doctor`.** Full validation suite (invalid slugs,
  duplicates, missing item.md, orphan epic refs, frontmatter parse
  errors, schema/transition violations) with `--fix` for the legacy
  layout migration and installable git hooks. Single-pass scanner;
  structural Findings → Actions → ApplyOutcome pipeline with
  `stop_phase` discriminator and partial-outcome preservation on
  mid-run failures. Non-zero exit on apply errors.
- **Focused mutation verbs.** `set`, `note`, `check`, `label`, and
  transactional `apply <patch.yaml>` (multi-field + body ops under one
  flock with rollback). All support `--dry-run` (unified diff, no
  write) and require `--expected-version` with `--json`. `note` writes
  timestamped blocks into standardized body sections (Comments /
  Decisions / Agent Runs / Reopen Notes); idempotent `set_checkbox`.
- **`issuectl fmt`** — canonicalize frontmatter ordering and YAML
  style. Optional YAML merge driver to make `git pull` collapse
  reorder-only conflicts cleanly.
- **Status transition rules.** `.issuectl/transitions.yaml` declares
  legal status edges; `update`/`set`/web all enforce them. Per-type
  body section linting rejects `--type` changes that leave required
  sections missing, with a hint listing the headings to add. CLI
  accepts custom statuses defined by `.schema.yaml`.

### Added — Discoverability & agent integration
- **Shared query engine** used by `issuectl ls`, `issuectl search`, and
  the web `/api/issues?q=` endpoint. Syntax: `field:value`,
  `-field:value` (negation), `text:"phrase"`, bareword (treated as
  `text:`), `field:any`/`field:none`, and relative dates
  (`updated:<-14d` strict, `<=-14d` inclusive, anchor: today in local
  timezone). Backslash-escapes (`\:`, `\\`, `\ `, `\"`, `\-`) inside
  unquoted values. Multiple terms AND together; no OR/parens in v1.
  The HTTP `?q=` endpoint enforces a 4096-byte / 64-term cap and
  surfaces parse errors as a JSON error envelope. Existing flag-based
  `ls` invocations remain backwards-compatible — `ls -s fixed` still
  implies open-only unless `--all`/`--closed` is explicit; only a
  *positional* query opts out of the open default.
- **`issuectl context <slug>`** — deterministic agent context bundle
  (issue + parent epic + blockers + related + acceptance criteria +
  recorded commits + schema rules) as markdown or JSON. JSON includes
  the same `version` token as `show --json` for one-shot
  `--expected-version` use. Cache to `.issuectl/cache/agent/<slug>/`
  with `--write`. Read-only.
- **`issuectl prompt <template> <slug>`** — repo-local prompt
  templates at `.issuectl/prompts/<name>.md` with `{{key}}`
  substitution against the context bundle. Any `## H2` heading in the
  body is reachable via its snake-cased name.
- **`.schema.yaml`** for declaring required/optional frontmatter
  fields, custom field types, custom statuses, and per-type required
  body sections. Lifecycle classification (open vs. closing status) is
  derived from the schema, not hard-coded.
- **`.issuectl/AGENTS.md`** — committed agent policy file.

### Changed
- **Repo layout is flat:** `issues/<slug>/item.md`. Status lives only
  in frontmatter; the `open/` and `closed/` subdirs are gone. The
  `folder` reported in `--json` is computed from status. `doctor --fix`
  performs the migration and rewrites `epic:`, `related:`, and
  `blocked_by:` legacy refs alongside `#NN` body refs to `@<slug>`.
- **Workspace split** into `issuectl` (binary) and `issuectl-core`
  (library) with constants relocation, centralized custom-field-key
  validation, and `do_new_locked` extracted from `main.rs` into a
  domain module. CLI golden-test harness for error output.
- `IssueSummary` (HTTP + SSE) gained a non-optional `version` field
  carrying the canonical content hash. The web client uses it as
  `expected_version` for drag-and-drop PATCHes without per-card GETs.
- Server caches the schema and transitions config in serve mode.

### Fixed
- **Canonical hash now covers `title` and all unknown frontmatter
  keys**, so reorder-only and unknown-key edits no longer produce
  spurious "modified externally" conflicts and version tokens stay
  stable across CLI, server, and watcher.
- **`mutate::new_issue` publishes the upsert event before releasing
  the flock**, closing a window where a subscriber could observe the
  file before the event landed.
- `issuectl ls` no longer drops the first character of the H1 title
  in CLI display.
- Reopening a closed issue auto-appends a `## Reopen Notes — <today>`
  section in the same write.
- `doctor` re-checks `critical_blockers` after the flat-layout
  migration before any NN-rename, runs `rename_notes_to_comments`
  after lifting legacy dirs (so freshly-flat dirs get the one-shot
  rename), and preserves partial outcomes on mid-loop failure.
- `legacy_number_from_mapping` data-loss when `number:` and `slug:`
  coexisted in legacy frontmatter.

### Removed
- `issues/{open,closed}/` subdirectories (folded into the flat layout
  by `doctor --fix`). The legacy migration path is preserved — repos
  on 0.2.0/0.3.x just need to run `issuectl doctor --fix` once.

### Internals
- Refactor of the doctor apply pipeline into Findings/Actions/Outcome
  with golden-JSON tests; lifecycle classification derived from
  schema; multi-LLM review-driven hardening across roughly 30
  spin-off issues. See `@exorbitantly-ill-apples` for the full
  trail.

## [0.3.1] - 2026-05-06

The 0.3.0 release pipeline published binaries built from the
pre-restoration commit (missing the doctor parse-warnings surfacing
shipped on top). 0.3.1 republishes from the actual head with all
intended changes included; no behavior changes vs. the source tree
that 0.3.0 was supposed to ship.

### Fixed
- `issuectl doctor` now surfaces YAML parse warnings in both the text
  report (new "Parse warnings:" section) and JSON output (new
  `parse_errors` field), instead of silently printing to stderr while
  the summary claimed "Repository OK". Skipped for legacy `<NN>-slug`
  dirs since the migration pass rewrites their frontmatter.

## [0.3.0] - 2026-05-06

This release adds a local web board for browsing issues visually and a
bundled docs command. `issuectl doctor --fix` is preserved as the
upgrade path for repos still on the legacy `<NN>-<slug>` layout.

### Added
- `issuectl serve` — a local, read-only web board (Trello-style) for
  the current repo's `issues/`. Open + closed columns, issue detail
  view with rendered markdown, and side docs from any sibling `*.md`
  files in the issue directory. Defaults to `127.0.0.1:7878`;
  `--host`/`--port` flags available, with a warning when bound to a
  non-loopback address. JSON API under `/api/issues[/...]` for
  programmatic use. Defense-in-depth security headers (CSP,
  X-Content-Type-Options, Referrer-Policy, X-Frame-Options) on every
  response, slug validation before any filesystem access, and rejection
  of symlinks that escape an issue directory.
- `issuectl docs [topic]` — bundled long-form documentation. First
  topic `kanban` covers the web board (usage, scope, security, routes).
  New topics drop into `templates/docs/` and register in `src/docs.rs`.
- `/issue` skill: install instructions for teammates who land in a
  repo using issuectl but haven't installed it yet (Homebrew, Cargo,
  shell installer); pointer to `issuectl serve` + `issuectl docs
  kanban` for visual browsing.

### Fixed
- `issuectl serve`: hardened against XSS via raw HTML in markdown
  bodies (ammonia sanitization), tightened CSP, and rejected symlink
  escape attempts in the side-docs endpoint.

## [0.2.0] - 2026-05-05

This release replaces sequential issue numbering with random word slugs.
**Breaking change** to repo layout: existing repos must run `issuectl
doctor --fix` once to migrate.

### Added
- `issuectl doctor` — repository health-check that detects legacy
  `<NN>-<slug>/` directories, invalid/duplicate slugs, missing `item.md`,
  and orphan epic references. Read-only by default; `--fix` performs
  the one-shot migration: renames dirs to slug-only, drops `number:`
  from frontmatter and writes `slug:`, and rewrites `#NN` body
  references to `@slug` form (scoped to `issues/`). `--json` for
  machine-readable output.
- `src/slug` module with adjective-adjective-noun generator and shared
  validation. Wordlists vendored under `src/slug/wordlists/` with
  attribution in `NOTICE` (EFF Long Wordlist CC-BY 3.0, moby
  Apache 2.0, `names` crate MIT).
- Detection of legacy dirs even when `item.md` lacks a `number:`
  field — falls back to parsing the dirname pattern, and tolerates
  missing/malformed YAML frontmatter. Verified end-to-end against
  real-world repos (~161 and ~240 legacy issues) where some items
  had no frontmatter at all.

### Changed
- **Issue identifier is now a random `adjective-adjective-noun` slug**
  (e.g. `quiet-brave-otter`) instead of a sequential integer. With
  ~500 × ~500 × ~1000 wordlists the collision space (~250M) is large
  enough that distributed/worktree workflows can land in any order
  without renumbering. Frontmatter field `number:` is replaced by
  `slug:`. Directory layout is `issues/{open,closed}/<slug>/`
  (numeric prefix dropped). `issuectl new` returns `slug` (string)
  in `--json` output instead of `number` (integer). All commands
  (`show`, `update`, `close`) accept slugs.
- Body cross-references use `@slug` instead of `#NN`. Markdown
  headings (`# Title`) are no longer rewritten as references.
- Slug claim at issue creation is now atomic (`mkdir`-based) so
  concurrent `issuectl new` runs cannot overwrite each other.
- Slug validation is unified across the crate; the `doctor` migration
  refuses to write paths that would escape `issues/`.

### Removed
- `issuectl renumber` — collisions are no longer possible by
  construction, so the band-aid is gone. Use `issuectl doctor --fix`
  for the one-shot migration from numbered repos.

### Fixed
- `doctor --fix` now migrates legacy directories correctly even when
  `item.md` has no `number:` (or no frontmatter at all) — previously
  these were left in place while the loader still printed
  legacy-numeric warnings, producing a contradictory "Repository OK"
  summary.

## [0.1.0] - 2026-05-02

Initial public release.

### Added
- `issuectl new` — create issues and epics with strict validation, automatic
  numbering, and kebab-case slug generation (preserves Finnish characters).
- `issuectl update` — edit frontmatter fields (status, assignee, owner,
  priority, epic, labels, related, commits) with round-trip preservation
  of unknown keys and field order.
- `issuectl close` — set a closing status and atomically move the issue to
  `closed/`. Defaults to `fixed` for bugs, `done` otherwise.
- `--root <PATH>` global flag to operate on an external repo without
  changing cwd.
- Strict input validation via `clap` `PossibleValuesParser` for `--type`,
  `--priority`, `--status`. Empty/whitespace-only string arguments are
  rejected with clear errors.
- `--json` output for `list`, `show`, `search`, `stats`.
- `issuectl skill install --agent claude|codex|all` to bootstrap a target
  repo with the `/issue` skill template (Claude Code at
  `.claude/skills/issue/SKILL.md`, Codex at `.codex/prompts/issue.md`,
  or both).
- `issuectl skill print --agent claude|codex` to preview the template
  on stdout without writing to disk.
- 95 unit and integration tests covering pure helpers, frontmatter
  round-trip, command flows (tempdir-backed), and renumber edge cases.
- End-to-end manual verification against a real-world ~144-issue
  monorepo with duplicate-numbering edge cases (fixture removed
  before publication).

### Changed
- **`issuectl renumber` is now minimal by default.** Unique issue numbers
  are preserved; only duplicates are renumbered, with the first by sort
  order keeping its number and the rest spilling above the current max.
  This drops 130 dir-renames to 22 on the real-world ~144-issue monorepo (~144
  issues, 19 duplicate numbers). The previous compact-1..N renumbering
  is no longer available — file an issue if you need it back.
- `issuectl renumber` now scans the **whole repo** for `.md` references
  by default (skipping `.git`, `target`, `node_modules`, `.cargo`,
  `dist`, `build`) instead of only `issues/`, so monorepo cross-references
  in `CLAUDE.md`, per-crate `AGENTS.md`, etc. stay consistent. Use
  `--scope <PATH>` (repeatable) to limit.
- `issuectl renumber --dry-run` previews the plan and ambiguous-reference
  list without modifying anything.
- `issuectl renumber --pin NUMBER=SLUG_SUBSTRING` (repeatable) tells the
  resolver which dir in a duplicate group keeps the original number.
  Substring is matched against the slug within the group; errors if it
  matches zero or multiple dirs (with the candidates listed). This
  matters when the repo's docs reference a duplicate number meaning a
  specific dir, not the alphabetically-first one. Wishlist item #12.
- `issuectl --json renumber` (with or without `--dry-run`) emits a
  structured report with the plan, ambiguous-mapping table, and
  per-step counts. Useful for pipelines and `sed`-script generation —
  agent feedback specifically asked for this.
- `--json` now also covers `new`, `update`, and `close`. The skill
  templates have been rewritten so every `issuectl` example uses
  `--json`, since the consumer is an AI agent that should never have
  to parse human-formatted output.
- `AGENTS.md` at the repo root captures the rule that any CLI surface
  change must update `templates/issue-skill.md` and
  `templates/issue-prompt.md` in the same commit, and re-run
  `issuectl skill install --agent all --force` so the in-repo
  `.claude/` and `.codex/` copies don't drift.
- Renumber's post-run report now lists each old number's spillover
  mapping (`#14 now maps to: #14 (kept) + #123 + #124`) and provides a
  ripgrep one-liner to find body-text references that need manual
  review.
- Re-opening a closed issue (setting an active status from `closed/`)
  now clears the `closed:` field automatically.
- Skill template (`templates/issue-skill.md`) rewritten to delegate
  Search/List/Show/Create/Update/Close to `issuectl` instead of raw
  filesystem operations.
- `issues/AGENTS.md` now drops legacy `# NN.` heading prefixes
  consistently with `renumber`'s behavior.

### Fixed
- `split_text` lost the blank line between frontmatter and body on
  round-trip, and produced a stray newline as the body for issues with
  empty bodies. Both fixed.
- Renumber's `rewrite_issue_dir_paths` recompiled directory regexes
  per (file × line × dir-map entry), which on real-world data with
  ~150 markdown files and ~20 dir-map entries produced ~330k regex
  compilations and never finished. Hoisted to once per file (~15s on
  the real-world ~144-issue monorepo).

### Removed
- `issuectl dedup` stub — moved to a future release until properly
  implemented.

[Unreleased]: https://github.com/jarimustonen/issuectl/compare/v0.5.2...HEAD
[0.5.2]: https://github.com/jarimustonen/issuectl/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/jarimustonen/issuectl/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/jarimustonen/issuectl/compare/v0.3.1...v0.5.0
[0.3.1]: https://github.com/jarimustonen/issuectl/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/jarimustonen/issuectl/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jarimustonen/issuectl/releases/tag/v0.2.0
[0.1.0]: https://github.com/jarimustonen/issuectl/releases/tag/v0.1.0
