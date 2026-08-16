<!-- Installed by `project-canon skill install` — ai-first-cli-canon cli_version=0.1.1 schema_version=1. Generated from AGENTS-AI-FIRST-CLI.md; do not hand-edit, re-run to upgrade. -->

# AI-First CLI Design Principles

<!-- PROVENANCE / MAINTENANCE (project-canon).
Maintained home: this canon lives here, in `project-canon`, as the canonical `cli`-profile
canon. Per homebase ADR 0009 §2/§6, `project-canon` is now the maintained home of this
document; homebase and other consumers are to copy this file FROM here rather than the reverse.
Edit the canon here. The homebase-side cutover (making homebase copy from here and retiring its
own master copy) is a documented FOLLOW-UP, tracked separately — until it lands, avoid editing
the homebase copy so the two do not diverge. §1–§22 are a stable citation surface (never
renumbered); this note is the ONLY addition to the lifted content. -->

**Canon version: 2** (2026-08-10). These principles apply to all CLI tools in
this repo unless otherwise mentioned. The primary caller is often an AI agent
(Claude Code), not a human typing in a terminal. Some conventions differ from
human-oriented software — follow these deliberately.

The tool family is Rust/clap; language-specific idioms below (a `Clock` trait,
`build.rs`, Cargo crates) are named as the concrete Rust shape of an otherwise
language-agnostic rule. Section numbers §1–§18 are a **stable citation surface**
(tools reference them by number, e.g. `§9`, `§16`) and are never renumbered; new
principles append as §19+. The `**Applies when** / **Look for** / rationale`
blocks on §19+ are the machine-checkable summary a future conformance checker
reads; formal RFC-2119 requirement IDs and a per-tool capability manifest are
deferred to that checker (Phase 3), not encoded here. **Canon v2 is
deliberately aspirational**: several v2 mandates (mandatory `config`
subcommands, real `commit` provenance, the exit-code remap) make some existing
tools non-conformant by design — that gap is the backlog, not a doc error.

## 1. Strict input validation — no silent fixups

Validate strictly. Reject malformed, empty, whitespace-only, or otherwise
suspicious inputs with clear errors. Do not coerce, trim silently, or fall
back to defaults for obviously-wrong inputs. The AI caller is responsible
for sending well-formed input — surface problems as errors so it can fix
its output and retry.

Concretely:

- Empty or whitespace-only required arguments → error, not default
- Unknown options/flags → error, not ignored
- Out-of-range values → error, not coerced
- Report the actual invalid value in the error message — the AI can parse
  it and fix its input

Rationale: a lenient parser hides the caller's mistakes. An AI caller can
read the error, correct its output, and retry. Surfacing defects is
cheaper than papering over them.

## 2. Structured, parseable output

CLI tools should support machine-readable output alongside human-readable
output:

- Provide `--json` flag for structured JSON output where applicable
- Errors go to stderr, data to stdout — keep them separate
- Include metadata in output (status codes, URLs, counts) so the caller
  doesn't need to infer them
- Exit codes must be meaningful and **classified, not collapsed**:
  `0` = success, `1` = **caller-/domain-actionable** error (bad input,
  not-found, validation failure, a current-state conflict the caller can
  resolve), `2` = **system / internal** error (I/O, a missing dependency,
  a broken invariant — the caller retries later or escalates rather than
  editing its argv). The split is "can the caller act on this?", not
  literally "fix your argv": a `not_found` is `1` even though the fix may
  be *create it*, not *retype it*. Never collapse every failure to a
  single code — an agent that branches on "1 = my problem, 2 =
  the tool's problem" is actively misled when a tool returns `2` for a
  user-not-found. Argument-parse errors (clap's default exit `2` for a
  *usage* error) must be remapped to `1`; but clap's `--help` and
  `--version` displays are **exit `0`** events, not errors — the remap
  applies only to genuine parse/usage failures, never to help/version
  display. Route every error through **one central error→exit mapping**
  so classification is uniform across subcommands rather than reinvented
  per handler:

  | condition | exit |
  |---|---|
  | success | `0` |
  | clap `--help` / `--version` display | `0` |
  | `invalid_*`, `not_found`, `already_exists`, `validation_*`, usage/parse error, `dry_run_unsupported` (§11) | `1` |
  | `io_*`, `network_*`, `internal_*`, dependency-missing, invariant-violation | `2` |
  | cancellation (SIGINT / SIGTERM, §12) | `130` / `143` |

  Structured `error.code` (§10) carries the precise class; the exit code
  is the coarse routing signal derived from it.

Rationale: AI agents parse stdout programmatically. Mixed human/machine
output forces format sniffing.

### Logs: JSONL, one event per line

Logs (whether emitted to stderr, a file, or a journal) must be
**JSONL** — one self-contained JSON object per line, one event per
line. No multi-line records, no plain-text fallback in production
mode, no human-formatted prefixes wrapping JSON payloads. A grep, a
`jq`, or a `tail -F | jq 'select(...)'` is the canonical reading
tool.

Each log line carries **trace-shaped context** so logs are filterable
by the actors and resources involved:

- `user_id` / `tenant_id` whenever a request, job, or message is
  attributable to a user or tenant
- `trace_id` / `run_id` / `request_id` so multiple log lines from one
  logical operation can be correlated
- `message_id`, `receipt_id`, `attachment_id`, etc. — domain entity
  ids relevant to the event
- The originating subsystem/module (`target`, `component`) so cross-
  cutting filters work

Avoid embedding user-identifying context into free-form `message`
strings only — put it in dedicated fields. `grep '"user_id":42'` and
`jq 'select(.tenant_id == 7 and .level == "ERROR")'` should both work
without parsing prose.

Rationale: production debugging looks like "what happened to user
X's message Y" — that question is answered by structured filters,
not by reading prose. Per-line JSON also keeps logs streamable
(every line is a complete record) and resilient to truncation.

**The diagnostics toggle is `--verbose`, off by default, on stderr.**
The **diagnostic** JSONL log above is silent unless the caller opts in
with a global `--verbose` flag. Standardize all three across the family:
the flag *name* (`--verbose`, exactly — not `--debug`, `--log`, a bare
`-v`, or a multi-level `-vv`; no aliases either), the *default* (off), and
the *channel* (stderr).

Keep three channels distinct — this is the one place the log rule
intersects §10 and §12, so be precise:

- **stdout** always carries the command's *data* (the `--json` payload, or
  the §12 `--output=jsonl` result/progress stream). This is **never**
  gated by `--verbose` — a long-running command's primary event stream is
  emitted regardless. Turning on diagnostics must never corrupt or gate
  the parseable stdout payload.
- **stderr** is the fatal-error channel (§10) *plus*, and only when
  `--verbose` is passed, the opt-in diagnostic JSONL log. `--verbose` is
  purely additive to stderr; it does not move data off stdout and does not
  make stderr non-fatal for a non-verbose run.
- A server / drainer / long-running mode routes its *diagnostic* events
  through this same `--verbose` switch rather than inventing a per-tool
  logging flag — but its *protocol/result* output (§12) still lives on
  stdout and is always emitted.

Diagnostic log lines obey the §8 secret-redaction rule: `--verbose` must
never mean "dump secrets" — redact secret-valued fields the same way
`config show` does, since agents routinely paste stderr into transcripts.
(`--verbose` is an invocation-behavior flag, not a config value — see §8;
text-mode progress under `--verbose` still obeys the no-spinner rule of
§12.)

## 3. No interactive prompts

No `press y to continue`, no confirmation dialogs, no interactive Y/N
prompts, no TTY-dependent behavior. All commands must be non-interactive:
valid input succeeds, invalid input fails with a clear diagnostic and
non-zero exit.

- Destructive actions opted in via explicit flags (e.g. `--force`, `--yes`)
- One-shot execution: all inputs via arguments, output to stdout/stderr
- No pagers, no `less`, no `$EDITOR` invocations

Rationale: AI agents cannot respond meaningfully to interactive prompts.

## 4. Informative error messages

Error messages should contain enough context for the AI caller to
understand and fix the problem without additional investigation:

- Include the actual invalid value: `"Invalid target 'foobar'. Available: local, staging, demo, prod"`
- Include the expected format: `"URL must start with / or http"`
- For multi-step failures, indicate which step failed and why
- Stack traces and internal details go to stderr with `--verbose`, not by default

## 5. Composable commands

Design commands to work well in pipelines and with other tools:

- Fetch commands output to stdout by default (pipe-friendly)
- `--output FILE` as an alternative to stdout redirection
- Support stdin where it makes sense (e.g. reading URLs from a list); accept
  `-` as a filename to mean stdin
- Consistent flag naming across commands (`--target`, `--output`, `--json`)

## 6. CLI surface: noun-verb imperative, declarative `apply` as opt-in

Default to a **noun-verb imperative** surface: the resource comes first, the
action second (`<tool> job create ...`, `<tool> node list`, `<tool> job show
<id>`). This matches `gh` (`gh pr create`, `gh issue list`). For tools with
a single dominant resource (`cargo build`, `npm install`) a flat verb-first
surface is fine — don't invent a noun layer for a one-resource CLI.

A **declarative manifest** surface (`<tool> apply -f run.yaml`) belongs as an
*additional* entry point, not the primary one. Add it only when:

- The resource has enough fields that a file is easier than flags, **and**
- Convergent reconciliation (apply repeatedly → same state) is a real
  requirement, not aesthetic Kubernetes-mimicry

This restriction applies to the declarative verb `apply`, not to file-based
input generally. Imperative commands may and should accept `--file`,
`--body-file`, or `-` (stdin) when the payload is too large, structured,
or quoting-sensitive for flags (large markdown, JSON bodies, batch
creates). That is plain composition, not declarative state.

Rationale: AI callers compose CLI calls one at a time from a planning step;
each call should be self-describing in the argv. `gh pr create --title X
--body Y` is one transcript line the agent reads back to itself. A manifest
file splits intent across argv + file contents and adds a stat/parse step
for the agent. Manifests *are* the right answer when state-convergence is
the actual semantics (Terraform, kubectl) — just don't make them mandatory
when the operation is genuinely imperative.

## 7. Subcommand verbs: pick one set, no synonyms

Use exactly this verb vocabulary across all subcommands:

- `list` (zero-or-more, filterable) — never `ls`, `index`, `all`
- `show` (one, by id/slug) — never `get`, `view`, `describe`, `cat`
- `create` (new resource) — never `new`, `add`, `make`
- `update` (mutate existing) — never `edit`, `set`, `patch`, `modify`
- `delete` (remove) — never `rm`, `remove`, `destroy`

No verb may mean both "list many" and "show one" (the `kubectl get pods` /
`kubectl get pod foo` overload is exactly the ambiguity this rule rejects).

The five-verb vocabulary governs **CRUD-shaped operations on a resource**.
Outside that shape there are two closed exception sets, and nothing else:

1. **Lifecycle / meta verbs** — a fixed, closed set, each pinned by its own
   section: `apply` (declarative convergence — §6), `exec` (runs something
   rather than mutating a resource), `skill` (companion-skill installer —
   §15/§16), `version` (drift contract — §10), `doctor` (self-diagnostic —
   §18), `fmt` (idempotent canonicalizer — §20), `init` (idempotent
   bootstrap — §21). These are not resource CRUD; they act on the *tool* or
   its *whole record set*, so the resource-verb rule does not reach them.
   The set is closed — a new meta-verb needs its own numbered section before
   it is canon, not a local justification.
2. **Domain state-transition verbs** — a verb naming a real state transition
   that has **no CRUD equivalent** (`commit`, `push`, `fetch` in git;
   `won`/`lost`, `file-vat`, `close-year` in domain tools).

The criterion that separates a valid domain verb (set 2) from a §7
*violation*: does the verb name a transition that a selective `update`
genuinely cannot express? `close-year` (runs a multi-step closing process
with side effects) qualifies; `won` qualifies only if it does more than set a
`status` field. Do **not** rename a plain create/update/delete to a domain
word to look bespoke — `New` instead of `create`, `set-status` or `won`
instead of a selective `update <id> --status …`, `done` instead of `update
<id> --status done` are §7 **violations**, not exceptions. When in doubt,
ask "could this be `update <id> --field value`?" — if yes, it must be.

`update` semantics: by default a `update` command mutates only the fields
named on the command line (selective patch). A full-resource replace is
opt-in via `--replace-file` / `--replace` and must be documented as such.
This is patch semantics under one verb — there is no separate `patch`
command.

Rationale: AI callers guess subcommand names from training-set patterns.
Even though `get` dominates the training corpus, the cost of one wrong-guess
retry is much smaller than the cost of an inconsistent verb vocabulary
across our own tools. The agent learns the rule once per tool family and
hits it every time after. We bias toward strictness over corpus-familiarity.

## 8. Configuration precedence: flag > env > file > built-in default

For **persistent configuration values** (API URLs, profiles, default
targets, timeouts, credentials), precedence is resolved **per configuration
key**: an explicit flag for that key overrides the environment variable for
that same key, which overrides the config file's value for that key, which
overrides the built-in default. Two independent keys may legitimately come
from different layers; one layer does not displace another wholesale.

- Lists and maps **replace** rather than deep-merge — the highest-priority
  source for that key wins in full
- Env var name mirrors the flag: `--api-url` ↔ `<TOOL>_API_URL`
- Config file location is **inspectable at runtime** via `<tool> config
  path`. The path itself may follow platform conventions (XDG on Linux,
  `~/Library` on macOS, `%APPDATA%` on Windows) — what matters is that
  the caller never has to guess
- `<tool> config show --json` prints the effective resolved config and
  where each value came from (`source: "flag" | "env" | "file" |
  "default"`). **Secret-valued keys are redacted by default**
  (`value: "<redacted>", secret: true`) — explicit `--show-secrets` is
  required to dump them, and emits a warning to stderr

`config path` and `config show` are **mandatory subcommands**, not optional
niceties — every tool that resolves any persistent configuration ships both.
(This is the family's single most consistent historical miss; treat its
absence as a conformance failure, not a gap you may leave open.)

**Data-root resolution is inspectable too — not only the config file.** Many
tools operate on a repo or data directory (an `issues/` tree, an accounting
home, a CRM records dir) and may be launched from an arbitrary cwd. The
*data root* gets the same never-guess / always-inspectable treatment §8 gives
the config file:

- The canonical selector is the single global flag **`--home <PATH>`** (one
  name across the family — not a `--repo`/`--home` synonym pair; §7's
  no-synonym rule applies to flags too), with the env var mirroring it as
  **`$<TOOL>_HOME`**.
- Precedence has **five ordered layers** — a superset of the config-key
  precedence, with directory *discovery* inserted just above the built-in
  default:

  ```
  --home  >  $<TOOL>_HOME  >  config-file `home` key  >  upward discovery  >  built-in default
  ```

  Upward discovery walks from cwd toward the filesystem root and stops at the
  **nearest** ancestor containing the tool's marker (the `.<tool>/` dotdir —
  see §21 — is the canonical marker; a tool that keys off a data dir like
  `issues/` documents that as its marker). Discovery stops at a filesystem
  boundary and does not cross into an unrelated repo.
- `<tool> config show --json` reports the **resolved data root and its
  source** — the source enum is `"flag" | "env" | "file" | "discovered" |
  "default"` (the config-key `"flag" | "env" | "file" | "default"` enum plus
  `"discovered"`) — and the matched marker path, so an agent launched from a
  subdirectory can confirm *which* root it is about to mutate before it writes
  anything.
- If a command needs a data root and none resolves, that is a §4 error with
  the canonical code **`data_root_unresolved`**, naming what was searched (the
  flag, the env var, the config key, the discovery marker) — never a silent
  operate-on-cwd or operate-on-nothing.

Rationale for the data root: §8 originally covered the *config file* path but
not the *data root*, and an agent launched from a subdirectory would silently
operate on the wrong repo (or none). The failure is invisible until after the
write, which is exactly the class of mistake the inspectable-source rule
exists to prevent.

**Invocation-behavior flags** (`--json`, `--dry-run`, `--force`, `--yes`,
`--verbose`, `--output`, positional resource identifiers) are **not**
config-file settings unless explicitly documented per command. They are
per-invocation choices, not persistent configuration.

Rationale: AI callers need to reason about *why* a value is what it is —
"the agent set `--api-url` but the run still hit prod" is debuggable only
if the source is inspectable. Mirroring flag↔env names removes a lookup
step. Reference: `aws` documents this precedence and exposes it via
`aws configure list`; copy that pattern. The secret-redaction default is
non-negotiable: AI agents routinely paste tool output into transcripts and
issue comments.

## 9. Output format is fixed, not TTY-detected

Output format is determined **only** by explicit flags (`--json`,
`--output=text|json|jsonl`), never by `isatty()`. Given the same inputs
and external state, stdout/stderr formatting does not change merely because
stdout/stderr is or is not a terminal. No color, no table-vs-line
switching, no progress bars based on terminal detection.

- Default format is human-readable text; `--json` opts into structured
  output; `--output=jsonl` opts into streaming events (see §12)
- Color is off by default; only `--color=always` and `--color=never`
  exist — there is no `--color=auto` (it would be either dead syntax or
  TTY-sniffing under another name)
- Pagination is never automatic — see §3

Rationale: TTY-sniffing makes CLIs non-reproducible. The agent's local
invocation, the CI invocation, and the user's terminal invocation must all
produce the same bytes given the same flags, or transcripts and tests
diverge from reality. `gh` and `kubectl` both ship TTY-detection that has
bitten users; avoid the trap.

## 10. Schema versioning, errors, warnings, and deprecation

JSON output is a versioned API surface, not free-form. Treat it
accordingly:

- Every `--json` payload (top-level and event-level for streaming) carries
  a `schema_version` field (integer, monotonic)
- Additive changes (new fields) do not bump the version. Breaking changes
  do: removing/renaming fields, changing field types, changing enum
  semantics, making optional fields required, changing nullability,
  changing event ordering guarantees, or changing the meaning of an
  existing field
- Every CLI implements `<tool> version --json` returning at least
  `{version, commit, schema_version, supported_schemas}` so the agent can
  detect drift between trained expectations and reality
- **The global `--json` flag works on every subcommand, `version`
  included.** No subcommand may accept only a local `--output json` (or
  any other private JSON toggle) while the rest of the tool takes the
  global `--json`. `version` is the canonical instance of this rule (it is
  the one an agent reaches for first and the one most often left as an
  `--output`-only special case), but the rule is general: one JSON switch,
  honored everywhere. The structured contract lives on the **`version`
  subcommand**; clap's parser-level `--version` flag (which prints plain
  text and cannot honor `--json`) is at most a text convenience alias — an
  agent that needs the payload always calls `<tool> version --json`, never
  `<tool> --version --json`
- The `commit` field must be **real build provenance, not a placeholder.**
  Stamp the git SHA the binary was built from at build time (a `build.rs`
  that reads the SHA, or the equivalent for the toolchain). Two concrete,
  checkable rules make this enforceable:
  - When provenance exists, `commit` is the **full 40-character hex SHA**
    (not a short form, not a tag, not a free-form string). Shipping the
    literal `"unknown"` or an empty string is forbidden — it makes the
    whole drift-detection contract inert, because the agent cannot
    correlate observed behavior with a source revision.
  - When a build genuinely has no git context (a released tarball, a
    vendored source drop, a crates.io archive), `commit` is exactly
    **`null`** — the explicit "no provenance available" value — paired with
    a sibling `build_provenance` object of fixed shape:
    ```json
    {"commit": null,
     "build_provenance": {"kind": "tarball", "note": "no .git in source archive"}}
    ```
    `kind` is one of `"git"` (SHA present), `"tarball"`, `"vendored"`,
    `"ci-injected"`; `note` is a human string. `null` is legal **only**
    for these no-git builds — a git-buildable source that simply failed to
    stamp is a release-blocking bug, not a `null` case.

  So "never ship an unpopulated `commit`" means: never *omit the key* and
  never ship a placeholder string — either a real 40-hex SHA or an explicit
  `null` + `build_provenance`. Note that flipping an existing tool's
  `commit` from `"unknown"` to `null` is a **nullability change** under
  §10's own rules and takes a `schema_version` bump.

**Error envelope under `--json`.** Failures emit a structured error
object to **stderr** (not stdout — see §2):

```json
{
  "schema_version": 1,
  "error": {
    "code": "invalid_target",
    "message": "Invalid target 'foobar'. Available: local, staging, demo, prod",
    "invalid_value": "foobar",
    "expected": ["local", "staging", "demo", "prod"]
  }
}
```

**Warnings are not errors.** Under `--json`, non-fatal warnings (e.g.
deprecation) belong in a `warnings: []` array inside the **stdout** JSON
payload — not on stderr. This keeps stderr fatal-only *by default* and avoids
forcing the agent to format-sniff. (The one opt-in exception is the `--verbose`
diagnostic log of §2, which the caller explicitly adds to stderr; without
`--verbose`, stderr stays fatal-only.) In text mode, warnings go to stderr
prefixed with `warning: ` so they're trivially distinguishable.

**Deprecation policy.** Deprecated flags and commands emit a structured
warning on every use, naming the removal version (or commit/tag window if
the tool has no semver releases). Suppress with
`<TOOL>_NO_DEPRECATION_WARNINGS=1`. Deprecations live for at least one
release window before removal. A deprecation alone never changes exit
code.

Rationale: agents pin against observed CLI behavior. Without a schema
version, the agent can't tell "field missing because absent" from "field
missing because renamed in v2". The error envelope makes failure parseable
the same way success is parseable.

## 11. Dry-run, idempotency, and retry safety

**Dry-run.** Every command that creates, updates, or deletes a resource
supports `--dry-run`. Dry-run:

- Performs all input validation and read-only checks that the real run does
- Emits the planned mutations using a **planning envelope** distinct from
  the real-run result envelope:
  ```json
  {
    "schema_version": 1,
    "dry_run": true,
    "would": [
      {"action": "create", "resource": "run", "input": {...},
       "known_effects": {"status": "would_create"},
       "unknown_until_apply": ["id", "created_at", "url"]}
    ]
  }
  ```
- Never partially applies — either prints the full plan or errors

If a truthful dry-run is not possible (token rotation, OAuth login,
race-sensitive ops, commands whose result depends on server-generated
state the dry-run cannot reserve), the command **fails explicit**:
exit 1 with `{schema_version, error: {code: "dry_run_unsupported",
reason: "..."}}`. A fake dry-run is worse than no dry-run — it gives
the AI caller false confidence.

**Idempotency and retry safety.** AI callers retry. The retry path must
not turn a successful first call into a confusing failure.

- For network-backed `create`, **support a caller-supplied idempotency
  key** (`--idempotency-key <opaque>`): the second call with the same key
  returns the original result, not a conflict. Echo the key in the JSON
  output. Recommend this pattern wherever the backend supports it (Stripe,
  AWS, and most modern APIs do)
- Where idempotency keys are not available, offer symmetric opt-ins:
  `--if-not-exists` on `create` (succeed silently if it already exists,
  return the existing resource) and `--if-exists` on `delete` (succeed
  silently if absent)
- `delete` of a missing resource defaults to a clear error, but the
  `--if-exists` flag exists for the AI retry use case
- `update` is selective by default (only fields named — see §7); a retried
  update is naturally idempotent

The point is the agent should always have a way to say "I don't care
whether you already did this; converge to this state and tell me the
final result." Different commands offer that affordance through
different mechanisms; offer at least one.

Rationale: "did my last call succeed?" must be answerable without
ambiguity-prone error-message string matching. Idempotency keys are the
industry-standard answer where the network is involved; the symmetric
flags are the local-tool answer.

## 12. Long-running operations: streaming events and progress queries

Operations that take more than a few seconds need a way for the caller —
human or agent — to know they are still alive and how far along they are.
The format is part of the command contract, not a runtime decision.

**Streaming mode.** A long-running command declares its output format up
front:

- `--output=jsonl` (or `--jsonl`) emits one JSON event per line to stdout,
  each carrying `schema_version`, `event` (`"progress"`, `"log"`,
  `"result"`, `"error"`, `"cancelled"`), and a monotonic `seq`
- Terminal events are mutually exclusive: exactly one of `result`,
  `cancelled`, or `error` ends the stream. The absence of a terminal
  event means the process crashed mid-stream; consumers treat that as
  `error`
- `--json` (single document) is forbidden for primarily long-running
  commands — pick `--output=jsonl` or design the command around a
  separate progress query (below). A command must not silently switch
  format based on elapsed runtime
- Text mode prints brief one-line-per-step progress to stderr — **never**
  spinners, ANSI cursor movement, or carriage-return-overwrite progress
  bars. These rules apply in both human and agent modes; we deliberately
  forfeit the spinner UX for format predictability

**Progress query.** For commands that run as a daemon, background job, or
detached process — where the caller is not streaming the output — every
such command exposes a paired progress query:

- `<tool> <noun> show <id>` (or `<tool> <noun> status <id>`) returns the
  current state, `schema_version`, the last `seq` emitted, and a recent
  event window
- Agents poll this instead of waiting on a stream. Human callers run it
  on demand

**Signals.** The streaming process traps both `SIGINT` and `SIGTERM`
(AI sandbox timeouts use `SIGTERM`, terminal Ctrl-C uses `SIGINT`) and
emits a final `{"event": "cancelled"}` event before exit when feasible.
Exit codes for cancellation: **130 for SIGINT, 143 for SIGTERM**. These
are declared exceptions to §2's `0/1/2` policy; document them in the
tool's `--help`.

Rationale: AI callers read incrementally and need to distinguish "still
working" from "hung". A spinner is invisible to a subprocess reader; a
JSONL event is parseable, filterable, and survives `tee` to a log. For
background jobs the agent can't stream, the progress-query subcommand is
the same answer in pull form.

## 13. Large outputs go to a file the agent can query

A `list` command that returns 10 000 rows blows out an AI agent's context
window. The conventional answer in human CLIs is paging or
`--limit`/`--cursor`; both push complexity onto the caller and force
repeated calls. The AI-first answer is different:

**Default to inline output for small results, and offer
`--output FILE.jsonl` or `--output FILE.db` (SQLite) for results that
might not be small.** When writing to a file:

- JSONL: one record per line, each carrying `schema_version` — agent
  reads with `jq`, `grep`, `head`, `wc -l`
- SQLite: structured schema with primary keys and indexes the command
  documents — agent reads with `sqlite3 file.db "SELECT ... WHERE ..."`
- The command prints to stdout (or `--json` stdout) only metadata about
  the file: path, count, schema_version, optionally a SQL/jq query hint
  the agent can use as a starting point

This replaces traditional pagination entirely for the AI use case. The
agent never gets the full result blob into context; it issues targeted
queries against the file. For genuinely huge results, SQLite is
preferred (indexed lookups, `LIMIT/OFFSET`, joins across multiple
exports). For moderate streaming results, JSONL is enough.

`--limit` is still useful as a guardrail against accidentally requesting
huge inline output, but it is not the primary mechanism.

Rationale: AI context is the binding resource. Twenty agent turns asking
`tool list --cursor abc123` is worse than one turn that writes a SQLite
file and three turns of focused SQL. The standard `--output FILE` from
§5 already exists; this section makes it the recommended pattern for any
result that might be large.

## 14. `--help` is agent-first, structured, and drill-down

`--help` is the first thing an AI agent reads when it doesn't know a
command. Optimize it for that reader. Humans benefit too.

- **Top-level `<tool> --help`** lists subcommands with one-line
  descriptions, and the small set of global flags (`--json`,
  `--output`, `--verbose`, `--version`). It does **not** dump every
  flag of every subcommand
- **Drill-down**: `<tool> <subcommand> --help` is the next layer —
  full flag list, accepted values, defaults, the env-var name for each
  flag (per §8), and exit-code semantics. Further nesting works the
  same way: `<tool> job create --help` is independent of
  `<tool> job --help`
- **Machine-readable help**: every `<tool> ... --help` accepts `--json`
  and emits a structured description of subcommands, flags, args,
  defaults, env-var mappings, accepted-value enums, deprecation status,
  and the `schema_version` of the help payload itself
- **Examples**: each subcommand's help includes at least one working
  example as text (humans), and an `examples: []` array of
  `{description, argv}` pairs under `--json` (agents). Examples are
  copy-pasteable and use the canonical verb vocabulary from §7

Rationale: agents lookup a command, fail, retry — this loop is much
shorter if the help they read is structured (no prose scraping) and
drilled (no flag-firehose). For humans, the same drill-down is just good
UX. The schema-versioned `--help --json` is what makes §10's "schema as
API surface" promise complete: now the *surface itself* is queryable, not
just the data.

## 15. `skill` subcommand: install companion AI-skills

Every CLI ships with a `skill` subcommand whose job is to install
Claude-Code-style skills (`SKILL.md` files with frontmatter) that teach
an AI agent how to drive this CLI in real workflows. The skill files are
the agent's *operating manual* for the tool — distinct from `--help`
(reference) and the schema (data shape).

- `<tool> skill list` — shows available skills shipped with this tool,
  one-line descriptions
- `<tool> skill install [<name>]` — copies the skill(s) into the active
  Claude Code installation (`~/.claude/skills/` by default,
  `--target <dir>` for other agent runtimes); installs all when no name
  given
- `<tool> skill show <name> --json` — prints the skill content without
  installing, so an agent can read it inline if needed

The skills themselves live alongside the tool's source (in-repo) so they
version with the binary. The CLI is responsible for keeping skill text
and CLI surface in sync (a tool whose `skill list` references a removed
flag is a release-blocker, same as a broken `--help`).

Rationale: `--help` tells an agent *what* a command does; a skill tells
it *when and how to use it in a multi-step workflow* — when to combine
with which other commands, which gotchas to avoid, what the success
criteria look like. The skill is also the natural place to encode
non-obvious idioms (e.g. "always pass `--output FILE.jsonl` when the
result might exceed N rows" — §13). Shipping skills from the tool itself
means every agent that installs the tool gets the operating manual in
one step, rather than asking the agent to discover patterns by trial.

## 16. `skill print`: stream skill content without installing

Pair the installer of §15 with a print subcommand that streams the
canonical skill text to stdout:

- `<tool> skill print <name>` — writes the `SKILL.md` (frontmatter +
  body) for `<name>` to stdout, exit 0. Unknown name → §10 error
  envelope on stderr, exit 1
- `<tool> skill print <name> --json` — emits a structured payload
  `{schema_version, name, cli_version, schema_version_skill, content,
  path_in_repo}` so the agent can route the body separately from the
  metadata
- Output is byte-identical to what `skill install` would have written
  to disk for that name. There is no "rendered" vs "raw" distinction
- No side effects: no file writes, no network. Print is the read-only
  twin of install

This is the natural complement to `<tool> skill install` (§15):
*install* persists the operating manual on the agent's runtime so it
loads on every future session; *print* streams it once into the
current conversation. Use install for the agent's own machine; use
print in CI, sandboxes, ad-hoc remote shells, or when the agent
discovers the tool mid-task and needs the workflow guidance
immediately without modifying the runtime.

Concretely, an agent that has just learned `<tool>` exists and wants
to drive it correctly runs `<tool> skill print <main-skill>` once and
reads the body into its working context — no install step, no
filesystem mutation, and (by §17) the version it gets matches the
binary it is about to invoke.

Rationale: `skill install` is the right answer when the skill should
persist across sessions, but it requires write access to a runtime
directory the agent may not own (CI runners, locked sandboxes, remote
shells). `skill print` makes the same operating manual available as
pure stdout — composable with `cat`, `jq`, `tee`, and the agent's
own context-loading mechanisms. It also gives `<tool> skill install`
a trivial reference implementation: install is `print | write-to-disk`.

## 17. Skill–CLI version synchronization

The companion skill of §15/§16 is a versioned artifact. Its workflow
guidance, flag names, and example invocations must match the CLI
surface that will execute them — a skill that references a removed
flag is no better than a broken `--help`. Treat skill text and CLI
surface as one release unit.

- **Frontmatter version fields.** Every shipped `SKILL.md` declares
  two versions in its frontmatter:
  - `cli_version:` — the CLI release the skill body was written
    against (e.g. `cli_version: "0.6.3"`)
  - `schema_version:` — the skill-format version (the §10 contract
    applied to the skill payload itself, so agents can detect breaking
    changes to the skill format independently of the tool's data
    schema)
- **`skill print` is version-pinned to the running binary.** `<tool>
  skill print <name>` always returns the skill that ships *with the
  currently installed binary* — i.e. its `cli_version` equals
  `<tool> --version`. It never reads a stale copy from disk. If the
  binary cannot resolve a matching skill (corrupt install, partial
  upgrade), it errors with `{code: "skill_version_mismatch"}` rather
  than serving an older copy
- **`skill install` warns on drift.** `<tool> skill install <name>`
  compares the target directory's existing skill (if any) against the
  CLI's bundled version. If the on-disk `cli_version` is older than
  the running binary's version, install proceeds but emits a §10
  warning naming both versions; if it is *newer* (agent upgraded the
  skill ahead of the binary), install errors unless `--force` is
  passed
- **`<tool> version --json` exposes the contract.** Per §10 the
  version payload already carries `version`, `commit`, and
  `schema_version`. Extend it with `skills: [{name, cli_version,
  schema_version}]` so the agent can audit skill freshness against
  the running binary in one call, no filesystem walk needed
- **CI gate.** A release pipeline that ships a CLI surface change
  (added/removed/renamed flags, changed verb vocabulary, changed
  `--help --json` payload) must regenerate or bump the bundled
  skill(s) in the same commit. The check is mechanical: diff the
  `--help --json` snapshot against the previous release, and fail the
  build if any skill's `cli_version` is older than the new binary's
  version. This is the same release-blocker discipline §15 already
  imposes on `skill list`

The principle is one-way: the **binary is the source of truth, the
skill follows it.** Skills never drift ahead of the binary in
production; they may lag by one bump only inside an active development
loop, never across a release tag.

Rationale: an agent that reads a stale skill will compose calls
against flags that no longer exist, then debug against a `--help`
that contradicts the skill — a worst-case loop that burns context and
produces wrong commits. Pinning `skill print` to the running binary
removes the discrepancy at read time; the frontmatter version fields
let the agent reason explicitly about which release it is following;
the install-time warning catches the offline case where the agent
installed a skill once and the CLI has since moved on. Together these
make "is my workflow guidance current?" a one-call question instead
of a multi-step audit.

## 18. `doctor` subcommand: read-only self-diagnostic

Every CLI ships a `doctor` subcommand that runs the tool's full
internal self-check and reports each check's status. Doctor is the
agent's first move when a command fails for non-obvious reasons —
"is the install broken, is the data corrupt, is the config wrong, is
a dependency missing?" — and it must answer that question without the
agent having to know which subsystem to interrogate.

- `<tool> doctor` — runs all checks; one human-readable line per
  check (`OK`, `WARN`, `FAIL` + short message) and a final summary
  `summary: N ok, M warn, K fail`
- `<tool> doctor --json` — emits the §10 structured form:
  ```json
  {
    "schema_version": 1,
    "checks": [
      {"id": "schema.issues", "status": "ok",
       "message": "12 issues validated"},
      {"id": "skill.sync", "status": "warn",
       "message": "skill 'issue' is cli_version 0.6.2, binary is 0.6.3",
       "fix_suggestion": "tool skill install issue --force"}
    ],
    "summary": {"ok": 11, "warn": 1, "fail": 0}
  }
  ```
- Exit code: **0** if all checks are `ok` or `warn` only; **1** if any
  check is `fail`. Deprecation-style warnings never flip the exit code
  (consistent with §10)
- **Read-only by default.** Doctor never mutates state. The corrective
  twin is `<tool> doctor --fix`, which runs the same checks and then
  applies the safe subset of `fix_suggestion`s. `--fix` is opt-in per
  invocation, never the default, and emits the planning envelope from
  §11 first if combined with `--dry-run`

The canonical set of check categories is small and stable:

- **Schema validation** — every on-disk data file the tool owns
  validates against its declared schema (e.g. `issuectl doctor` walks
  `issues/*/item.md` frontmatter against `issues/.schema.yaml`)
- **Dependencies** — every binary the tool shells out to is on `PATH`
  at a supported version; missing or out-of-range versions
  `FAIL` with `fix_suggestion` naming the install command
- **Skill sync** — for every installed companion skill (§17), the
  on-disk `cli_version` matches the running binary; mismatch is
  `WARN` with `<tool> skill install <name> --force` as the suggestion
- **Configuration integrity** — every required key from §8 resolves
  (flag/env/file/default), no orphan references (e.g. a config
  pointing at a deleted profile), no secret-shaped values stored in
  non-secret keys
- **Data integrity** — domain-specific structural checks: orphan
  files, broken cross-references, stale lock/marker files, indices
  that disagree with the underlying records

Every check has a stable `id` (so the agent can pin which checks it
expects to see), a `status`, a one-line `message` naming the actual
state observed, and — for `WARN`/`FAIL` — a `fix_suggestion` that is
either a concrete command the agent can run, or a brief diagnostic
hint when no automated fix is safe.

Rationale: AI agents debug by hypothesis testing, and `doctor` is the
cheapest hypothesis: "is the tool itself healthy?" One command, one
structured answer, with per-check `id`s the agent can correlate
against the failure it just saw. The read-only default matters
because the agent will run `doctor` reflexively after errors — it
must not have side effects in that loop. The `--fix` twin exists for
the explicit "yes, apply the suggested repairs" case, separated by a
flag so neither use accidentally triggers the other. The structured
output makes `doctor` composable with the rest of the agent's
toolchain (`jq '.checks[] | select(.status == "fail")'`), the same
way every other §10-conformant payload is.

## 19. Deterministic clock: inject time, never read it ad hoc

**Applies when** a tool stamps `created` / `updated` timestamps, derives
filenames or ids from the current time, or otherwise lets wall-clock time
leak into its output.

Output that embeds `now()` is not reproducible: two runs of the same command
produce different bytes, which breaks golden tests, transcript matching, and
caching. Note the scope: §9 promises stable output "given the same inputs and
external state", and wall-clock time *is* external state — so §9 does not by
itself pin the clock. §19 closes that specific seam. Fixing the clock is
**necessary but not sufficient** for byte-determinism: random ids/UUIDs,
hash-map iteration order, filesystem `readdir` order, locale, and timezone are
other sources the tool must also pin (sort before serializing, seed or inject
id generation) before it may claim a command is byte-deterministic. §19 owns
the clock; it is not a determinism certificate on its own.

- **Thread a clock seam through the core, don't call `now()` inline.** The
  domain logic takes its current time from an injected clock abstraction (in
  Rust, a `Clock` trait with a real `system_clock` and a `FakeClock` for
  tests), so time is a parameter, not an ambient read. This is what makes core
  logic byte-deterministic under test (and pairs with the core/cli split of
  §22).
- **Expose one hidden global override for reproducible runs.** The flag is
  **`--frozen-time <RFC3339>`** (exactly one name — not a `--frozen-time` /
  `--now` synonym pair; §7's no-synonym rule applies), hidden from the
  top-level `--help` firehose but documented in `--help --json`, freezing the
  clock for a single invocation. Golden tests, deterministic fixtures, and
  cache-key derivations set it; normal callers never do. Timestamps are RFC-
  3339 UTC. **Caveat for id/filename-deriving commands:** because a frozen
  clock can produce colliding timestamp-derived ids or filenames, `--frozen-
  time` is intended for fixtures and tests, not routine production writes, and
  must never be used to forge the persisted `created`/`updated` provenance of
  a real record. A command whose ids collide under a frozen clock should
  detect the collision and error rather than overwrite.
- **Advertise determinism where a command guarantees it.** A command whose
  output is byte-stable given fixed inputs and a fixed clock (having also
  pinned the other sources above) should say so in its help, so an agent knows
  it can safely cache or diff the result.

**Look for:** a single hidden `--frozen-time` global; an injectable clock
abstraction in the core crate rather than scattered `Utc::now()` calls;
golden/byte-stable test fixtures; and, for a determinism *claim*, evidence the
other nondeterminism sources are pinned too.

Rationale: reproducibility is an AI-first property — transcripts, caches, and
tests must match reality run-to-run. The clock is the most common source of
non-determinism in an otherwise-pure command, and it is invisible until a test
flakes or a cache thrashes. Making time an injected parameter (not an ambient
read) turns "why did the output change?" from a debugging session into a
non-question.

## 20. `fmt`: idempotent canonicalizer for on-disk records

**Applies when** a tool owns human-editable on-disk records — markdown with
YAML frontmatter, git-native data files — that both humans and the tool
write, and whose diffs must stay clean and merge-safe.

§18 `doctor` validates *structure and integrity* against the schema, but a
schema-valid record can still be non-canonical (key order, array ordering,
whitespace, line endings do not affect schema validity). A tool that owns
records humans hand-edit needs a distinct step that *rewrites* them into
canonical form so diffs are minimal and merges don't conflict on cosmetic
differences.

- `<tool> fmt` rewrites the tool's records into canonical form — stable key
  order, normalized whitespace, and sorting **only of arrays whose order is
  not semantic** — and is **idempotent**: running it twice produces no second
  change (`fmt` of already-canonical input performs no file changes).
  - *Order-insensitive* arrays (a set of labels, a `blocked_by` list where
    order carries no meaning) may be sorted for a stable diff.
    *Semantically-ordered* arrays — workflow steps, ledger entries, a ranked
    priority list, a command's argv — **must preserve their order**; sorting
    them corrupts meaning. Which arrays are which is a per-field property the
    tool's schema declares; `fmt` sorts an array only where the schema marks
    it order-insensitive.
- **`fmt` must not change any field that records semantic modification time or
  provenance** (an `updated:` / `modified_at` / `_meta.updated` field,
  whatever the tool names it). Canonicalization is not a semantic edit;
  bumping the modification field on every reformat would make it lie and
  pollute history. This is a behavioral contract, not tied to a specific field
  name.
- **Atomicity.** `fmt` parses and canonicalizes *all* targeted records before
  writing any, and replaces each file atomically (write-temp-then-rename), so
  a malformed record or a crash mid-run never leaves a half-rewritten tree. A
  parse failure on any record aborts the whole run with a §4 error and no
  writes.
- **`fmt --dry-run`** emits the §11 planning envelope (the set of files that
  *would* change) and writes nothing — the standard preview every mutating
  command owes (§11).
- `<tool> fmt --strict` is the CI mode: it makes **no** changes and exits
  non-zero if any record is not already canonical (§2 classification — a
  formatting violation is a caller-actionable error, `1`; an I/O or parse
  failure while checking is `2`). `--strict` checks *only canonicalization* —
  exactly the conditions a plain `fmt` would fix, preserving the invariant
  that `fmt` then `fmt --strict` always passes. It does **not** fold in
  unrelated lints (secret scans, integrity checks) — those live in `doctor`
  (§18), so the two responsibilities stay separately composable.
- Offer the ecosystem hooks that keep records canonical automatically, as
  `fmt`-namespaced installer subcommands (in the §15 `skill install` family of
  sanctioned installer verbs): `<tool> fmt install-hooks` for a pre-commit
  hook and, for tools whose records are merged across branches, `<tool> fmt
  install-merge-driver`. Note the merge driver *canonicalizes after* git's
  own three-way merge resolves the content — it is not a replacement merge
  algorithm and never reconciles conflicting edits itself.

**Look for:** a `fmt` verb documented as idempotent; schema-declared per-field
sort-safety; an explicit "does not touch the modification/provenance field"
guarantee; atomic all-or-nothing writes; a `--dry-run` plan and a `--strict`
no-write CI mode scoped to formatting only; optional `fmt install-hooks` /
`fmt install-merge-driver`.

Rationale: clean-diff and merge-safety are a distinct, recurring obligation
for record-owning tools that `doctor` (validate) and `update` (mutate one
resource) do not cover. A canonicalizer that bumped the modification field,
re-ordered a semantic array, or wasn't idempotent would create the very churn
it exists to remove; the constraints are what make it safe to wire into every
commit.

## 21. `init`: idempotent, no-clobber bootstrap

**Applies when** a tool scaffolds an on-disk home — schema files, an
`AGENTS.md` / policy doc, a companion skill install — that must exist before
the tool's other commands can run.

`init` is a real lifecycle verb that §7's CRUD vocabulary doesn't cover, in
the same class as the sanctioned `apply` / `exec`. Bless it, and pin down its
contract so every tool's `init` behaves identically:

- `<tool> init` writes the initial on-disk layout under the data root: the
  `.<tool>/` marker dir (§8), the schema/config scaffold, and the repo-local
  agent policy doc (`.<tool>/AGENTS.md` or equivalent).
- **The target home is resolved explicitly, never silently from cwd.** `init`
  targets the data root that §8 resolution selects (`--home` / `$<TOOL>_HOME`,
  or — for `init` specifically, which *creates* the marker discovery would look
  for — the cwd only when neither is set and the tool documents that fallback).
  It must not scaffold into a surprising directory: if the resolution is
  ambiguous it errors (§4) asking for an explicit `--home` rather than guessing.
- **Companion-skill install is not implicit.** Because §15 installs skills into
  the *agent's* runtime (`~/.claude/skills/`), outside the data root, `init`
  does **not** silently mutate that global directory. It either prints the
  recommended `<tool> skill install` follow-up, or performs it only under an
  explicit `--install-skill` flag. Bootstrapping the data home and mutating the
  agent runtime are two consented acts, not one.
- **Idempotent and safe to re-run.** A second `init` over a home already valid
  *for this tool* is a no-op that fills in only missing scaffold pieces — never
  a reset, never discarding existing records.
- **The clobber boundary is defined, not tool-discretionary.** A home is
  *already-initialized* (→ no-op / fill-in) when it carries this tool's marker
  and a compatible version; it is *conflicting* (→ §4 error, no writes) when the
  target is non-empty but is **not** this tool's home — a foreign marker, an
  incompatible/newer schema version, or a would-be-created file that already
  exists with different content. A genuine re-scaffold of a conflicting home is
  opt-in behind explicit `--force` (§3), and even `--force` never deletes domain
  *records* — only the managed scaffold files it owns.
- **Atomic and reporting.** `init` does not leave a half-initialized home on
  crash (stage, then commit the layout). Under `--json` it reports what it
  created vs. what already existed as `{"created": [paths], "existed": [paths],
  "skipped": [paths]}`, so an agent distinguishes a fresh scaffold from a re-run
  without diffing the filesystem; `--dry-run` emits the §11 planning envelope
  and writes nothing.

**Look for:** an `init` documented as "safe to re-run / refuses to clobber";
an explicit already-initialized-vs-conflicting rule; explicit `--home`
targeting; `--install-skill` (not implicit global skill writes); `--force` for
deliberate re-scaffold that spares records; a created/existed report and
`--dry-run` in the `--json` output.

Rationale: bootstrapping is a distinct, recurring lifecycle step, and its
danger is precisely that an agent may run it reflexively (or twice) against an
already-populated home, or from the wrong directory. The idempotent +
no-clobber + explicit-target contract makes a stray `init` harmless, which is
exactly the property an AI caller — prone to retry — needs from the one command
whose job is to write the initial state.

## 22. Internal layout: library-first `core` / `cli` split

**Applies when** a tool is expected to be unit-tested deeply, reused as a
library, or grown past a single-file command. This is the family's dominant
internal convention (5 of 7 audited tools), named here so new tools start in
the right shape rather than refactoring into it later. Unlike §§1–21 this is
an *internal* convention (it shapes the codebase, not the agent-facing
surface — so it is graded as a `SHOULD`, never a hard readiness gate); it
earns a section because it is what makes the deterministic-core testing of §19
and the shared plumbing below actually achievable.

- Split the workspace into `crates/<tool>-core` — pure domain logic, no clap,
  no direct I/O, dependency-light — and `crates/<tool>-cli` — argument
  parsing, rendering, side effects. Larger tools may layer the core further
  (model / wire / domain / ledger / … ) but the `core` ↔ `cli` boundary is the
  load-bearing one.
- The core is where the injected `Clock` of §19 lives, so domain behavior is
  byte-deterministic under test without spinning up the CLI.
- A single-crate layout is acceptable for a genuinely small, single-purpose
  tool (a thin server, a one-command utility); it is a deliberate choice to
  document, not a default to drift into.
- **Converging the shared plumbing.** Because the §2 error→exit map, the §10
  envelope + `version` payload, and the §8 `config`, §15 `skill`, and §18
  `doctor` scaffolds are identical obligations across every tool, a shared
  crate carrying them removes the "each tool reimplemented the plumbing
  slightly differently" drift at the root. Call it `<family>-cli-common`
  (a placeholder name — pick the family's real prefix; a leading hyphen is not
  a valid Cargo package name). New tools should reach for the shared crate
  before hand-rolling the envelope again. Keep it a thin plumbing crate
  (errors, envelope, `version`/`config`/`doctor`/`skill` scaffolds behind
  feature flags) — not a grab-bag that couples every tool's release cycle.

**Look for:** a `crates/<tool>-core` + `crates/<tool>-cli` (or `<tool>`)
workspace split in `Cargo.toml`; domain logic free of clap/I/O imports;
timestamps taken from an injected clock rather than `now()`.

Rationale: the split is what lets the domain logic be tested deterministically
and embedded elsewhere, and it is the precondition for factoring the canon's
own boilerplate (envelope, `version`, `config`, `doctor`, `skill`) into one
shared crate instead of seven near-copies. Naming it as canon means new tools
inherit the testable, convergence-friendly shape from commit one.
