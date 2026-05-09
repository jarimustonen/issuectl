# Web control surface — design spike

Status: **design note (spike output)**, not an implementation contract.
Supersedes the narrow precursor @excessively-beneficial-owner ("start
implementation" button) by widening the question: how does the kanban
become a *control surface* for issue-related work, not just an
issue-text editor.

The brainstorm input came from a multi-LLM `/llm-collab` session
(Gemini 3.1 Pro, GPT-5.5, DeepSeek v4 Pro). The synthesis here is
mine; the convergence across models was strong enough to skip a
build-on round. A subsequent `/llm-review` (four models) and
`/assess-findings` pass produced the revisions captured in
`history/review-web-control-surface.md`; this version of the design
note has integrated those.

**Hard assumption: `workmux` is present.** `workmux` is the local
agent multiplexer this project already uses (the `/worktree` skill
calls it). It owns the primitives that an action runner would
otherwise have to reinvent: worktree+tmux-window creation
(`workmux add`), prompt delivery to a running agent
(`workmux send`), agent-status query (`workmux status --json`),
status-waiting (`workmux wait`), terminal capture
(`workmux capture`), and lifecycle cleanup (`workmux merge`,
`workmux remove --gone`). The design below treats `workmux` as a
required dependency for the `worktree`-shaped action kinds, on
the same footing as `git` itself. Generic `kind: exec` actions
have no `workmux` dependency.

## 1. Problem framing

`issuectl serve` ships a kanban with bidirectional read/write sync
(`docs/design/web-edit-sync.md`). Edits land in `issues/<slug>/item.md`
through a single mutation protocol: `flock(.issuectl/write.lock)` +
`expected_version` (canonical hash of frontmatter+body). The server is
loopback-only by default; per-process CSRF token; non-loopback writes
gated by `--allow-remote-writes`.

We want one more verb on top of that: **trigger work**. From a card,
the user wants to click and have an agent (Claude Code in practice,
but the design must not be Claude-specific) start working on the
issue, in the right repo/worktree, with the right context.

The original ticket framed three layered candidates:

1. **Agent trigger** — web → running serve process → its parent
   agent terminal receives a prompt.
2. **Worktree spawn** — server runs the equivalent of the `/worktree`
   skill (workmux + `git worktree` + tmux + `claude`).
3. **Schema-defined scripts** — `.issuectl/actions.yaml` declares a
   curated list of named commands the web can invoke.

The brainstorm exposed several others. The interesting design move is
not picking one of the three but realising that (2) and (3) compose,
and (1) is mostly a misconception.

## 2. Mechanism candidates

Eight plausible mechanisms, deliberately listed before the rubric to
make the rubric defensible.

**`workmux` re-frames several of these.** The original spike enumerated
mechanisms as if from a blank slate. With `workmux` assumed, the
"agent-side runner over SSE" candidate (§2.3) is partly an *existing
local primitive*, not a future protocol: `workmux send`,
`workmux status`, `workmux wait`, and `workmux capture` already
provide prompt delivery, agent-status query, blocking wait, and
output capture against worktrees the user has open. The candidates
are listed below as originally drafted; commentary on which
`workmux` makes redundant follows in §2.9.

### 2.1 In-process subprocess spawn

Server reads an action manifest, picks the matching `argv[]`, and
calls `std::process::Command::spawn()` directly from the HTTP handler.

- **IPC**: none — fork+exec from the axum task.
- **Flock**: the spawned child holds nothing of the server's; if it
  later mutates the issue it goes through the existing CLI which
  takes the issue flock briefly.
- **Reach**: single machine, child lifetime tied to whatever process
  group `serve` is in. Orphans on restart unless the spawn double-forks.
- **Failure mode**: the server now owns process lifecycle: zombies,
  output buffering, cancellation, retry, log retention. None of that
  is what an HTTP handler is for.

### 2.2 Filesystem run queue + runner

Server writes a JSON run descriptor to a queue directory; a separate
`issuectl runner` process (or an embedded runner inside `serve`) claims
runs by atomic rename and executes them. State machine
`queued → running → complete | failed | cancelled | lost`.

- **IPC**: filesystem. Single queue lock on the queue directory.
- **Flock**: run-queue lock is a *separate* file from the issue
  flock. The runner only takes the issue flock when it (or a child)
  calls `issuectl --json …` to mutate.
- **Reach**: durable across `serve` restarts. Multiple local runners
  with capability filters work today; cross-machine *only* if both
  ends agree on a shared filesystem with reliable `rename(2)`
  semantics, which we should not promise.
- **Failure mode**: real ones (stale `running/` after runner crash,
  orphan logs, accidental commit of artefacts), all addressable with
  PID + heartbeat fields and a sweep on startup.

### 2.3 Agent-side runner subscribed over SSE

A long-running process — `issuectl runner` started in the same tmux
window where the user actually wants Claude to land — connects to
`serve` via SSE (or WS) and consumes a stream of run events. The
runner owns the terminal/session; the server only routes work.

- **IPC**: HTTP SSE downstream + REST upstream for status/claim.
- **Flock**: same as 2.2 — the runner mutates issues via the CLI.
- **Reach**: this is the only candidate that makes "send to my
  already-running Claude" tractable, because *the runner* decides
  how to inject (stdin, `--resume <session>`, append-prompt file).
  Multi-agent and (with explicit auth) cross-machine fall out
  naturally.
- **Failure mode**: needs runner registration / pairing, capability
  advertisement, reconnection. More moving parts than 2.2 alone.

### 2.4 Webhook to a local listener

Action manifest declares `kind: webhook` + URL. Server POSTs the run
payload to a local port the agent-tooling exposes.

- **IPC**: HTTP loopback to a third-party listener.
- **Reach**: only as good as the listener; many tools don't have one.
- **Failure mode**: SSRF-shaped if the URL isn't restricted to
  `127.0.0.1`; auth is consistently botched in this category. Real
  status/cancellation requires a callback protocol nobody has built.

### 2.5 Unix socket / named pipe

Agent runner listens on `$XDG_RUNTIME_DIR/issuectl/<repo>/agent.sock`;
server writes JSON-line frames.

- Stronger local-only semantics than HTTP loopback (file-mode-bit
  scoping); slightly faster; portability story is uneven (Windows
  named pipes are a different API; WSL works).
- Browser still can't talk to it directly, so the server is still in
  the loop. This is 2.3 with a different transport — not a
  separate primitive.

### 2.6 OS URL handler (`issuectl://run?...`)

Browser navigates to a custom-scheme URL; OS dispatches to a
registered handler that launches the agent.

- **Reach**: launches on the *client* machine, not the dev box. If
  the user accesses the kanban from an iPad that hits a remote
  `--allow-remote-writes` server, the URL fires on the iPad. That
  is the wrong place.
- **Failure mode**: per-OS installer, browser confirmation prompts,
  no return channel for status. Not a fit for a single Rust binary.

### 2.7 Parent-terminal injection (the original mechanism #1)

Web → server → parent agent process / TTY of `issuectl serve`.

- The parent process is often gone (`serve` is started detached, or
  the user killed the launching shell).
- TTY injection (`TIOCSTI`, ioctls into the controlling terminal) is
  disabled or restricted on every modern OS for a reason: it is a
  privilege-escalation primitive. Where it's still allowed it is
  unreliable across multiplexers.
- Even when "parent" is well-defined, the pipe-pane / `tmux send-keys`
  trick depends on a tmux session naming convention we haven't
  imposed, and silently no-ops on plain shells.

The intent behind this mechanism is real and good: *I want the
prompt to land where my Claude session already is, not in a fresh
window I have to switch to.* The right way to satisfy it is 2.3 — a
runner the user explicitly starts in that session — not parent-PID
trickery.

### 2.8 SSE event on the existing `/events` channel

Reuse the kanban's event stream, add an `ActionRequested` payload,
let any subscriber pick it up.

- This is functionally a degenerate 2.3 where every browser tab
  *also* sees action requests. Bad: the `/events` stream is for
  state push, not capability dispatch. Mixing introduces "did anyone
  pick that up?" semantics on a channel that's meant to be
  fire-and-forget broadcast.
- Reject on cohesion grounds for *dispatch*. **Lifecycle events**
  (run created/started/complete) are still broadcast-shaped and the
  review found multiplexing them into the existing channel is the
  right call (one new `RunUpserted` payload variant); see §7.

### 2.9 What `workmux` subsumes

| Candidate | Status under the workmux assumption |
| --- | --- |
| 2.1 in-proc spawn | still relevant for `kind: exec` (no terminal); workmux not involved |
| 2.2 filesystem run queue | **kept** — workmux owns agent multiplexing, not run queueing |
| 2.3 agent-side runner over SSE | **subsumed for v0.6.0**: `workmux send`/`status`/`wait` give us the same primitives locally without inventing a runner protocol. The SSE-runner shape is now strictly the *cross-machine* future story. |
| 2.4 webhook | unchanged (low-priority extension) |
| 2.5 unix socket | **subsumed**: workmux already provides the local IPC abstraction; rolling our own is gratuitous |
| 2.6 URL scheme | unchanged (rejected) |
| 2.7 parent-TTY inject | **subsumed in the right way**: "send to my already-running Claude" is `workmux send <name>` against a worktree the user already has open. No TIOCSTI, no PID guessing. |
| 2.8 mix into `/events` | **adopted** for run-lifecycle events; rejected for dispatch |

The shape of the design therefore narrows: build the run queue and
the action manifest, point one of the action kinds at `workmux`, and
the "agent multiplexer" tier is *not our code*.

## 3. Rubric

| Mechanism | Security | UX (latency / visibility / cancel) | OS port. | Reach | Impl cost | Blast radius |
| --- | --- | --- | --- | --- | --- | --- |
| 2.1 In-proc spawn | Medium — argv-only manifest helps, but server owns child lifecycle | Low latency, output buffering ad-hoc, cancel = kill PID | Good | Single machine, single runner | Low | High if `kind: shell` ever lands |
| 2.2 FS queue + runner | High — server only writes JSON; runner is the only exec surface | Medium latency (debounce or polling), durable status, cancel via state file | Excellent | Multi-runner local; **not** cross-machine | Medium | Low — exec in one place |
| 2.3 Agent-side runner over SSE | High when paired with token auth; loopback-only by default | Best UX — runner owns the terminal, status flows back live | Good (HTTP) | Multi-agent, cross-machine with explicit auth | **Local: zero (workmux);** future cross-machine: medium-high | Bounded by workmux/runner's own capabilities |
| 2.4 Webhook | Mixed — listener auth is the weak link | Depends entirely on listener | Good | Whatever the listener supports | Low to add to a manifest, but ecosystem is empty | Medium |
| 2.5 Unix socket | High (file-mode scoping) | Same as 2.3 | Uneven on Windows | Local only | Medium | Low |
| 2.6 URL scheme | Low — runs on client machine | Mediocre — no return channel | Per-OS installer | Wrong machine in remote case | High (packaging) | Medium |
| 2.7 Parent-TTY inject | **Reject** — privilege escalation primitive | Looks magical when it works, fails silently when it doesn't | Bad | One specific terminal | Medium | High |
| 2.8 Mix into `/events` | Conflates broadcast with dispatch | No claim/ack | Good | n/a | Low | Medium — wrong channel for the verb |

The rubric does not pick a winner on its own; "implementation cost" and
"reach" trade against each other. The argument for the recommended
shape is in §5.

## 4. Architecture: actions, runs, runners

Three named concepts, deliberately separate.

**Action** — a *declaration*, declared in `.issuectl/actions.yaml`. A
named verb the kanban offers. Has a `kind` (`worktree` | `exec`),
metadata (label, description), template variables, concurrency
limits, and a permission to accept extra browser-supplied
instructions.

**Run** — an *instance*. Created when the user clicks an action button
or `issuectl run` is invoked. Has a stable id, a snapshot of
`(action, slug, expected_version, instructions)`, materialised
artefact files, a state, and logs. Stored in a queue directory.

**Runner** — an *executor*. A loop inside `issuectl serve
--enable-actions` (in v0.6.0) that claims runs and dispatches them.
For terminal-backed kinds it shells out to `workmux`; for `kind:
exec` it does an `argv` spawn. Execution pulls argv from the
action declaration; the run never ships argv from the wire.

```
.issuectl/actions.yaml                              (declarations, committed)
<git-common-dir>/issuectl/runs/                     (durable run state, NOT committed)
   preparing/<run_id>.json    (review §1.1 fix; not yet visible to runner)
   queued/<run_id>.json
   running/<run_id>.json
   complete/<run_id>.json | failed/<run_id>.json | cancelled/<run_id>.json | lost/<run_id>.json
   logs/<run_id>.{stdout,stderr}                    (only for kind: exec; tmux runs use workmux capture)
   artifacts/<run_id>/
       context.md
       prompt.md
       instructions.txt
   queue.lock
```

The state directory lives under `git rev-parse --git-common-dir`,
not literal `.git/`. In a linked worktree `.git` is a *file* with a
`gitdir:` pointer, so a naive path join breaks; the common-dir
path is shared across all linked worktrees of one repo and is
where shared run history belongs. (Review F3.) Fallback for
non-git directories or when git plumbing fails:
`$XDG_STATE_HOME/issuectl/<repo-fingerprint>/runs/`.

This is *not* `.issuectl/runs/` (committed) for a deliberate
reason: browser-supplied free-text instructions land here as
artefacts. Committing them by accident is a real failure mode.

### 4.1 Manifest shape (illustrative)

Three action kinds in v0.6.0: `kind: workmux` (spawn a fresh
worktree+window with an agent), `kind: workmux-send` (deliver a
prompt to an *existing* workmux worktree the user has open), and
`kind: exec` (generic argv-only command, no terminal).

```yaml
version: 1
actions:
  implement:
    label: "Start implementation"
    description: "Create a worktree and start an agent on this issue"
    kind: workmux
    prompt: ".issuectl/prompts/implement.md"
    base: "main"                       # validated as a git ref (R14)
    branch_template: "{{slug}}"        # default; passed as the BRANCH_NAME positional
    agent: "claude"                    # tells workmux which agent to launch in the pane
    accept_extra_instructions: true    # R12: UI textarea toggle; not a permission boundary
    on_start_runner:                   # best-effort status flip in the RUNNER (review F1, R8)
      set_status: in-progress
      best_effort: true                # R8: if it fails after agent started, warn — don't fail the run
    concurrency:
      per_issue: 1                     # R4: 1 is the default for every kind; opt out per action

  send-to-active:
    label: "Send to active worktree"
    description: "Deliver an extra prompt to an open agent for this issue"
    kind: workmux-send
    # target is resolved at click time (see §4.2.2); no `target:` field
    prompt: ".issuectl/prompts/follow-up.md"
    accept_extra_instructions: true
    concurrency:
      per_target: 1                    # R4: only one in-flight send per worktree
      cooldown_ms: 1000                # R4: brief debounce window

  context-dump:
    label: "Print context"
    kind: exec
    command: ["issuectl", "context", "{{slug}}"]
    concurrency:
      per_issue: 1                     # R4: even harmless-looking exec defaults to 1
    # no terminal; logs captured to logs/<run_id>.{stdout,stderr}
```

Template variables resolve only against a closed allowlist:
`{{slug}}`, `{{run_id}}`, `{{repo_root}}`, `{{prompt_file}}`,
`{{context_file}}`, `{{instructions_file}}`. Whole-argument
substitution only; no interpolation into shell strings. Browser-
supplied "extra instructions" *never* enter argv — they land only
in `instructions_file`, which the agent reads.

**Concurrency defaults (R4).** All three kinds default to one
in-flight run per natural scope (`per_issue` for `workmux`/`exec`,
`per_target` for `workmux-send`). Manifest authors who genuinely
want parallelism opt in explicitly (`per_issue: unbounded`,
`per_issue: 4`, etc.). The previous draft defaulted `workmux-send`
and `exec` to unbounded; that was unsafe — double-clicks spawn
duplicate `cargo test` runs or interleave prompts in the agent's
input buffer.

**Canonical workmux naming (R7).** When the runner calls
`workmux add`, the workmux name is always `issuectl-<run_id>` —
not the slug. The branch name comes from `branch_template`
(default `{{slug}}`). The mapping `(slug → workmux name)` for any
in-flight run is recorded in the run JSON, which is what
`workmux-send` consults to find a target — see §4.2.2.

### 4.2 Action kinds: `workmux`, `workmux-send`, `exec`

#### `kind: workmux`

The runner's execution path:

```
1. (optional, when on_start_runner.set_status set; R8)
   issuectl --json update <slug> --status in-progress \
                                  --expected-version <run.issue_version>
   ↳ failure → record warning on the run, continue (best_effort)
2. workmux add \
     --name issuectl-<run_id> \
     --prompt-file <abs path artifacts/<run_id>/prompt.md> \
     --base <validated_base> \
     -- <branch_name>
   ↳ failure → run → failed (state_reason: workmux_add_failed)
3. workmux wait issuectl-<run_id> \
                --status done \
                --timeout <action.timeout>
   ↑ blocking child of the runner. Cancellation = SIGTERM this
     child + the cancel escalation in §5.3. Do **not** poll
     `workmux status --json` in a loop (R1).
4. observe wait's exit:
     0     → run → complete
     timed out → run → failed (state_reason: agent_timeout)
     non-0  → run → failed; record stderr for diagnostics
```

`workmux add` does the worktree-create + tmux-window +
prompt-injection + agent-launch in one call. `workmux wait` is
what tells us when the agent is genuinely done — no
"tmux client exited so we marked it complete while Claude kept
running."

Cleanup (`workmux merge`/`workmux remove`) is **not** part of the
runner's path. The user merges or discards on their own schedule.
A previous draft of this section had `workmux merge` as a
post-completion step; that is a source-control mutation, not
cleanup, and never belonged in an automatic path. (R9.)

##### Pre-flight, version pinning, status translation (R5)

Action availability is checked by:

1. `workmux --version` succeeds and reports a version `≥ MIN_WORKMUX`
   (current floor: workmux 0.1.x line that emits the
   `status`/`branch`/`elapsed_secs`/`pane_id` JSON shape we depend
   on). Bump the floor whenever workmux changes a field name or
   enum value.
2. The configured `agent` exists in `PATH`.

Both results surface through `GET /api/actions` so the UI can
disable unavailable actions with an explicit reason.

The runner translates `workmux status --json` payloads through
an explicit enum *before* surfacing to the UI or transitioning a
run. The translation must be written against **actual workmux
output**, not the names this design wishes were used. As of
`workmux 0.1.202` the observed `status` values include `working`
and `done`; the translation table is:

```rust
enum WorkmuxObservedState {
    Working,        // agent active
    Done,           // agent finished
    Idle,           // agent waiting for input (if workmux ever emits this)
    Missing,        // no record for this name
    Unknown(String), // any other string
    ParseError(String),
}
```

UI pill mapping:

| Observed | Pill | Run transition |
|---|---|---|
| `Working` | "running" | stay running |
| `Done` (after observed `Working`) | "complete" | terminal: complete |
| `Idle` | "waiting" | stay running |
| `Missing` after previously observed | "lost" | terminal: lost |
| `Unknown(s)` | "running (unknown: s)" | stay running, log warning |
| `ParseError(s)` | "running (degraded)" | stay running, retry with backoff |

Unknown enum values and parse errors **never** terminally
transition a run. A `workmux` upgrade that adds a new state must
not silently mark every active run failed.

##### Argument hardening (R14)

`workmux add` is invoked with `--` separating flags from the
branch positional, and `<validated_base>` has been checked via
`git check-ref-format --branch <base>` before the call. This
catches manifest typos like `base: "main "` (trailing space) or
`base: "--orphan"` (accidental flag), even though manifest
authors are trusted to write correct YAML overall.

#### `kind: workmux-send`

```
1. resolve target: latest run for this slug whose workmux name is
   alive (per `workmux status`). If none, the action is unavailable
   for this slug — the UI offers to fall back to `kind: workmux`.
2. workmux send <resolved_target> --file <artifacts/<run_id>/prompt.md>
3. on success → run → DELIVERED (not complete, see below)
   on failure → run → failed; record stderr
```

**Terminal state is `delivered`, not `complete` (R3).** The action
is "done" only in the sense that the prompt reached the agent's
input buffer. The agent has not necessarily read it, acted on it,
or finished anything. Calling this `complete` would be dishonest.
The UI pill says **Delivered**, with a tooltip "agent outcome
unknown — open the tmux window to see what happens."

Internally, the run-state machine has `delivered` as a separate
terminal state alongside `complete`/`failed`/`cancelled`/`lost`.
Wire format treats `delivered` as a peer of `complete` for SSE
purposes.

The user clicks → the prompt lands in the worktree they already
have open in their tmux. No new window appears. This is the
v0.6.0 answer to "send to my already-running Claude."

**Concurrency: `per_target: 1` with a 1-second cooldown (R4).**
Sends are not idempotent: two follow-ups can interleave in the
agent's input. A user who genuinely wants to enqueue multiple
follow-ups can override `per_target` in the manifest, but the
default refuses a second send until the first is `delivered`,
plus a debounce to absorb double-clicks.

**Target resolution (R7).** The action does not take a `target:`
template field. The runner resolves the target at click time by
looking up the most recent live `kind: workmux` run for the same
slug in `running/<id>.json`, reading its `workmux_name`
(`issuectl-<run_id>`), and verifying it via `workmux status`. If
no such target exists, `GET /api/actions` reports the action
`available: false, unavailable_reason: "no active worktree for
this issue"` and the UI offers `kind: workmux` instead. This
removes the ambiguity of the previous draft, which used
`target: "{{slug}}"` and quietly broke if the workmux name had
diverged from the slug.

#### `kind: exec`

Argv-only commands with no terminal. The runner spawns into a
process group (`setpgid`/`setsid` on Unix; review F8/F9), captures
stdout/stderr to `logs/<run_id>.{stdout,stderr}` with a per-stream
size cap (default 10 MiB, configurable per-action), and waits for
exit. Cancellation kills the process group. Suitable for
non-interactive verbs like context dumps, lint runs, batch issue
operations.

#### What we explicitly do *not* offer in v0.6.0

- `kind: shell` — shell-string commands, even from trusted manifests.
- `kind: webhook` — POST to a local listener URL.
- Free-form prompts originating from the web that are inserted into
  argv. Browser text only ever reaches an agent via
  `instructions_file`.
- Auto-detection of arbitrary tmux sessions. `workmux-send` only
  resolves targets that are tracked as in-flight `kind: workmux`
  runs.

#### Footgun: `kind: exec` invoking `workmux add` directly (R16)

A manifest author who writes

```yaml
custom-spawn:
  kind: exec
  command: ["workmux", "add", "--name", "...", "..."]
```

will get **broken lifecycle semantics**: the runner waits on the
short-lived `workmux add` process and marks the run `complete`
the moment it returns, while the spawned agent runs for hours in
a tmux pane the runner is not tracking. Logs come from
`workmux add`'s ~zero-byte stdout, not from the agent. Cancellation
kills the long-since-exited `workmux add`, not the agent.

Use `kind: workmux` if you want workmux lifecycle. We do not
lint or refuse this configuration — it's the same broken-design
shape as any opaque `exec` invoking a daemon-spawning tool.

This list matters. The whole reason `kind: workmux` is a structured
kind, not a `kind: exec` calling `workmux` from YAML, is that the
runner needs to do specific things around it: pre-flight version
check, blocking on `workmux wait`, status translation via the enum
above, log reads via `workmux capture`, and structured cancellation.
Pushing all of that into a YAML recipe trades a one-time Rust
integration for ongoing manifest complexity in every repo.

## 5. The contract: who holds which lock when

The single most important property of this design is **the issue
flock and the run-queue lock are different files, and no code path
holds both simultaneously**.

```
.issuectl/write.lock                     ← existing issue mutation flock
<git-common-dir>/issuectl/runs/queue.lock ← new run-queue lock
```

If the issue flock is ever held across an agent's lifetime, it
becomes a DoS primitive against the kanban, the CLI, and other
agents. The review's most important structural feedback (F1/F2)
was that the original draft held the issue flock too long: across
the context-bundle render *and* the optional `on_start: set_status`
mutation, with no rollback if the render later failed. The revised
sequence below keeps the issue flock to a hash-and-snapshot window
and moves status mutation into the runner.

### 5.1 Enqueue path (web → run created)

The trick is the `preparing/` queue state from the GPT-5.5 review:
the run is *durably reserved* in the queue *before* any other
side effect, so failures are always surfaced to the user as a
visible run record, never as torn issue state.

```
client                              server
  |  POST /api/actions/implement/runs                |
  |  X-Issuectl-CSRF: <tok>                          |
  |  Idempotency-Key: <uuid>                         |  (review F10, R15)
  |  { slug, expected_version, manifest_digest,      |  (R6)
  |    instructions }                                |
  |  ------------------------------------------>     |
  |                                                  |
  |                 1. validate CSRF + Host          |
  |                 2. validate action_id ∈ manifest |
  |                 3. compare manifest_digest       |  (R6: stale-preview)
  |                    against current resolved      |
  |                    digest                        |
  |                    → 409 manifest_changed if     |
  |                    different (UI must re-fetch   |
  |                    /api/actions and re-render    |
  |                    the modal)                    |
  |                 4. flock(queue.lock)             |
  |                    a. check Idempotency-Key      |
  |                       (TTL 1h, scoped per        |  (R15)
  |                        action_id+slug)           |
  |                       → return existing run if   |
  |                       already accepted; expired  |
  |                       keys mint a fresh run      |
  |                    b. check per-issue concurrency|
  |                       (queued+preparing+running) |
  |                       → 409 already_running      |
  |                    c. write preparing/<id>.json  |
  |                       (slug, action, idem, etc.) |
  |                 5. release(queue.lock)           |
  |                                                  |
  |                 6. flock(write.lock)             |
  |                    a. locate_issue(slug)         |
  |                    b. read item.md, hash         |
  |                    c. compare expected_version   |
  |                       → 409 version_mismatch +   |
  |                       transition preparing→failed|
  |                    d. SNAPSHOT issue (in mem)    |
  |                 7. release(write.lock)           |
  |                                                  |
  |                 8. RENDER context bundle from    |  (outside flock per F2)
  |                    snapshot to artifacts/.tmp-<id>|
  |                                                  |
  |                 9. flock(queue.lock)             |
  |                    a. atomic rename              |
  |                       artifacts/.tmp-<id>        |
  |                       → artifacts/<id>           |
  |                    b. atomic rename              |
  |                       preparing/<id>.json        |
  |                       → queued/<id>.json         |
  |                       (or → failed/ on error)    |
  |                10. release(queue.lock)           |
  |                                                  |
  |                11. publish RunUpserted on        |
  |                    multiplexed /events channel   |  (F12 resolution)
  |  202 Accepted { run_id, status: "queued" }      |
  |  <------------------------------------------     |
```

Notes on the revised sequence:

- **`on_start: set_status: in-progress` is no longer in the enqueue
  path.** It moves to the runner (§5.2 step 3). If the runner
  cannot mutate (e.g. version-mismatch because the user just
  closed the issue from another tab), the run lands in `failed`
  with `state_reason: issue_changed`, the issue is untouched, and
  the user sees the failure as a run record rather than as a
  silent status flip.
- **Time the issue flock is held**: read item.md, hash, compare
  version, snapshot to memory. No process spawn, no rendering, no
  multi-file walks. (Review F2.)
- **The queue lock is acquired twice** but always uncontended with
  the issue flock and never held across `.await` or render work.
- **No manifest-digest re-check.** This design ships without a
  repo-trust hashing layer; the manifest is treated as authored
  code on the same footing as `Makefile`, npm scripts, or git
  hooks. (See §6 for the explicit trust statement.)
- **Idempotency** is enforced at the queue lock: a duplicated
  POST with the same key returns the original run id rather than
  creating a second run. (Review F10.)

### 5.2 Claim + execute path

The runner picks up a queued run; what it does next depends on
the action `kind`. All three paths share the same outer envelope.

```
runner (in-proc inside `serve --enable-actions`)
  loop:
    1. flock(queue.lock)
    2. pick first queued/<id>.json (oldest first; capability filter
       is a no-op in v0.6.0 since there's only one runner)
    3. atomic rename → running/<id>.json
       write runner_id, host_id, started_at, pid (kind: exec only)
    4. release(queue.lock)
    5. RECHECK cancel_requested (review F24);
       transition → cancelled if set
    6. dispatch on action.kind:
         workmux:       see §5.2.1
         workmux-send:  see §5.2.2
         exec:          see §5.2.3
    7. on terminal state: flock(queue.lock); rename → terminal dir;
       release(queue.lock); publish RunUpserted
```

The runner never holds the issue flock during step 6. If the
action mutates the issue (e.g. on-start status, end-of-run commit
list), it shells out to `issuectl --json update
--expected-version …`, the same path the CLI uses; that call's
`flock` is short and bounded.

#### 5.2.1 `kind: workmux`

```
6.a. (optional, when on_start_runner.set_status set; R8)
     issuectl --json update <slug> --status in-progress \
                                   --expected-version <run.issue_version>
     ↳ failure: do NOT fail the run. Record a warning on the
       run JSON and continue. Status mutation is best-effort
       because the agent has not started yet; failing the run
       here would deny the user their click for a non-essential
       side effect.
6.b. workmux add --name issuectl-<run_id> \                  (R7)
                 --prompt-file <abs path artifacts/<id>/prompt.md> \
                 --base <validated_base> \                   (R14)
                 -- <branch_name>                            (R14)
     ↳ failure → run → failed (state_reason: workmux_add_failed)

   At this point, if 6.a recorded an `in-progress` status flip
   that the agent will not actually realise, the warning on
   the run JSON is the user-visible signal. (The previous draft
   had this as a "torn state" hazard — R8.)
6.c. workmux wait issuectl-<run_id> \                        (R1: NO polling loop)
                  --status done \
                  --timeout <action.timeout>
     ↑ runner blocks on this child process. Cancel = SIGTERM
       this child (see §5.3 escalation).
     ↳ exit 0  → run → complete
       timed out → run → failed (state_reason: agent_timeout)
       non-0  → run → failed; record stderr
```

Logs are read on demand (`GET /api/runs/<id>/logs` →
`workmux capture issuectl-<run_id> -n <lines>`, with HTTP-side
timeout and bytes cap per R13); we don't maintain a stdout file
for this kind.

Cleanup is **not** part of the runner's path (R9). The user
runs `workmux merge` or `workmux remove --gone` on their own
schedule. A previous draft listed `workmux merge` as a
post-completion step, which was both contradictory with §11 and
unsound — auto-merging unreviewed agent work is a footgun.

#### 5.2.2 `kind: workmux-send`

```
6.a. resolve target (R7): scan running/<id>.json for the most
     recent live `kind: workmux` run with this slug; verify via
     `workmux status <name>`. If none → run → failed
     (state_reason: no_active_target).
6.b. workmux send <resolved_target> \
                  --file artifacts/<run_id>/prompt.md
6.c. exit code 0 → run → DELIVERED (R3 — peer terminal state to
     `complete`; UI pill says "Delivered")
     nonzero      → run → failed (record stderr)
```

This kind delivers a prompt to an agent we don't control. The
receiving agent decides what to do with it. The terminal state
is `delivered`, not `complete`, because we have no honest way to
know whether the agent acted on the prompt — see §4.2.2 for the
rationale. Cancel is a no-op once `workmux send` has returned.

#### 5.2.3 `kind: exec`

```
6.a. spawn argv into a new process group
     (setpgid on Unix; review F8/F9):

     let mut cmd = Command::new(&argv[0]);
     cmd.args(&argv[1..]).stdout(stdout_log).stderr(stderr_log);
     unsafe { cmd.pre_exec(|| { libc::setsid(); Ok(()) }); }

6.b. wait for exit (with timeout if action.timeout set);
     on cancel_requested: kill(-pgid, SIGTERM); poll;
     after grace period, kill(-pgid, SIGKILL).
6.c. exit-code disposition:
        0                              → complete
        nonzero ∉ action.success_exit_codes → failed
```

Per-stream log size cap (default 10 MiB) with truncate-and-tag
sentinel. (Review F14.)

### 5.3 Cancellation

`POST /api/runs/<id>/cancel`:

```
1. flock(queue.lock)
2. read current state of <id>:
     queued       → atomic rename → cancelled/, publish, return 200
     preparing    → return 409 (try again in a moment)
     running      → set cancel_requested in running/<id>.json
                    via tmp+rename; return 200
     terminal     → return 200 no-op (review F30)
3. release(queue.lock)
```

The runner observes `cancel_requested` and dispatches by kind:

**`kind: exec`** — kill the recorded process group:
```
kill(-pgid, SIGTERM)
sleep grace_period (default 5s)
if still alive: kill(-pgid, SIGKILL)
```
PID start-time is validated before signaling (review F29) so that
PID reuse cannot kill an unrelated process.

**`kind: workmux`** — explicit escalation ladder (R2). `workmux
send` is **not** documented as a cancel primitive, and
`workmux 0.1.x` does not interpret special escape strings.
Sending the literal four characters `<C-c>` would land in the
agent's input as text. Honest semantics:

```
1. SIGTERM the runner's `workmux wait` child process.
   This unblocks the runner immediately.
2. Best-effort interrupt of the agent: workmux send
   <name> "/cancel" (or whatever convention the agent
   accepts as "stop"). The exact string is configurable
   per-action via `cancel_signal:` (default empty → no
   interrupt sent, agent keeps running).
3. After grace_period (default 30s), if `workmux status`
   still reports the agent as working: workmux remove
   --force <name>. This kills the tmux pane and is
   destructive — the agent's in-flight work is lost.
4. Run transitions → cancelled.
```

The user-visible promise is "we asked the agent to stop and tore
down the pane after 30s". We do **not** promise to gracefully
unwind in-flight tool calls or save state — `workmux remove
--force` is the backstop.

**`kind: workmux-send`** — cancel is a no-op once `workmux send`
has returned (the action is already terminal at that point).

### 5.4 Stale-run reaper

On `serve` startup and every N minutes thereafter, iterate
`running/`. If `host_id == this_host_id` and the recorded
`runner_id` is no longer alive, mark `lost`. (`host_id` is
generated once per host into `$XDG_STATE_HOME/issuectl/host-id`,
not derived from hostname — hostnames are unstable.) For
`kind: workmux` runs, additionally reconcile against
`workmux status`: if the runner died but the workmux worktree
is alive and the agent reports `done`, transition to `complete`
rather than `lost`.

Cross-host runs (`host_id != this_host_id`) are left alone — that's
the future multi-machine story and we don't speculatively reap.

**Known limitation (R18):** `host_id` is a generated UUID, not a
durable machine identifier. If `$XDG_STATE_HOME/issuectl/` is
wiped (cleanup script, fresh OS install, ephemeral home) while
runs are in-flight, those runs are issued a new `host_id` on
next `serve` start and the old records become permanently
"cross-host" from the reaper's perspective — they will not be
marked `lost`. The user can clear them with
`issuectl runs gc --orphaned` (part of the spin-off F21 work).
Replacing the generated id with `/etc/machine-id` /
`IOPlatformUUID` would close this leak; we judged the
platform-specific code not worth the rare failure mode.

## 6. Trust, security, blast radius

### 6.1 The repo-trust decision: no gating

The design **explicitly does not** include a `issuectl trust`
content-hash gate. `.issuectl/actions.yaml` and the prompt files
it references are treated as **authored code** on the same
footing as `Makefile`, npm scripts, `pre-commit` hooks, or git
hooks: if you clone a repo and run the tool inside it, the repo's
checked-in instructions can run code on your machine.

This is a deliberate, named trade-off. The review's F4/F5/F23/F33
findings flagged that without a content-hash gate, a `git pull`
that swaps `actions.yaml` underneath an already-running `serve`
process can silently change what actions execute. That is
correct, and the project accepts it: the trust prompt's
ergonomic cost (re-prompt on every legitimate manifest edit,
trains users to click-through) is judged worse than the silent-
swap risk it nominally protects against, given the threat model
below.

### 6.2 Threat model the design *does* defend against

- **Web → server injection** (a malicious browser tab, an npm
  postinstall hitting loopback, a browser extension): per-process
  CSRF token, `Host` allowlist matching the actual bound
  socket, no shell-string interpolation of any web-supplied
  field. Browser-supplied "extra instructions" land in a *file*,
  never in argv (review F31).
- **Network exposure**: loopback default; `--allow-remote-writes`
  is an opt-in for issue mutations; `--allow-remote-actions` is
  a *separate* additional opt-in not shipped in v0.6.0.
- **Filesystem corruption from broken `flock` semantics**:
  startup-time FS detection refuses to run on NFS / Dropbox /
  iCloud / SMB without `--unsafe-shared-fs` (review F25).

### 6.3 Threat model the design does *not* defend against

Stated plainly so nobody is surprised:

- **Malicious manifests in cloned repos.** Cloning an untrusted
  repo and running `issuectl serve --enable-actions` is RCE.
  Same as `make`, same as `npm install`, same as opening the
  repo in an IDE that auto-runs config files. The user is
  responsible for reading `actions.yaml` and any referenced
  prompt files before enabling actions in a given repo.
- **The `make`/npm analogy is not perfect (R19).** `make foo` is
  invoked from a terminal, per-target, with the user consciously
  typing the command. `serve --enable-actions` turns checked-in
  YAML into a clickable web button labelled "Start
  implementation". That label is **not** a security boundary: a
  malicious repo can map any benign-looking action to any argv.
  The web-button modality lowers the friction of running
  repo-authored code compared with typing `make`, and labels can
  hide the payload. Mitigations: the action modal previews the
  resolved argv (§7), and `manifest_digest` binding (§5.1 step 3
  / R6) prevents preview-vs-execution drift. Treat action
  buttons as executable code, not as ordinary kanban affordances.
- **`git pull` that swaps actions underneath a running `serve`.**
  The freshly-pulled manifest is in effect on the next click.
  This is the F4 finding accepted as out-of-scope. R6's
  digest-binding catches the *stale-preview* sub-case (the user
  reviewed argv X in the modal, the server runs argv Y) but not
  the broader "user clicks a button whose semantics changed
  since they last looked." A web-UI banner on manifest-content
  change (§7) is the visibility primitive for that case.
- **Trust boundary is transitive (R17).** "Manifest is authored
  code" extends to **every file the manifest references** — the
  prompt files in `.issuectl/prompts/`, anything those include
  (today's templates have no include syntax; if v0.7 adds one,
  trust extends along the include chain), and any submodule whose
  HEAD provides those files. To prevent the manifest from
  *reading* arbitrary files via `prompt:` declarations, manifest
  file references must be repo-relative, must not contain `..`,
  and must resolve under the repo root after `realpath` —
  symlinks pointing outside the repo are rejected at parse time.
- **Prompt-injection content** in `.issuectl/prompts/*.md`. A
  malicious prompt can instruct the agent to exfiltrate secrets
  or modify code outside the issue's scope. We document this as
  a known risk; the same risk applies to any agent reading any
  in-repo file.

The argv-only / template-allowlist mechanics do not change this
trade-off either way. They protect the *web → server* boundary,
not the *manifest → executor* boundary. The relevant security
boundary is therefore explicit: **the user vouching for the
repo by running `issuectl serve --enable-actions` inside it**.

### 6.4 Visibility (R11)

Visibility lives in **two places**, with the UI as the primary
surface because that is where the user is when they decide to
click.

**Web UI (primary).** The action modal previews the resolved
argv before submission (§7), and the kanban surfaces a yellow
banner when `manifest_digest` from the live `/api/actions` differs
from the digest the page initially loaded with. This is exactly
the case `git pull` mid-serve produces: the page is stale, the
user is one click away from running argv they haven't reviewed.
The banner says "Actions changed since this page loaded —
refresh to see the current set" and offers a refresh button.

**Terminal banner (supplemental).** On `serve --enable-actions`
startup, print a one-time banner before binding the listener:

```
issuectl: action surface enabled for /Users/jari/Sources/foo
  3 actions defined in .issuectl/actions.yaml:
    implement       kind: workmux       resolved: workmux add --base main … claude
    send-to-active  kind: workmux-send  resolved: workmux send <target> --file …
    context-dump    kind: exec          resolved: issuectl context <slug>
  Loopback only (127.0.0.1:7878).
```

Banner shows the **fully resolved argv**, not a truncated
"workmux add …", because a truncated preview can hide
`--dangerously-skip-permissions`-shaped flags. This is **not** a
gate — `serve` proceeds regardless. It exists for users who
launched `serve` interactively from a terminal; for users who
run it under launchd/systemd, the UI banner is the only
visibility they will ever see, which is acceptable because the UI
banner fires on every *change*, not just first-run. Suppress the
terminal banner with `--quiet-actions-banner` once you've read
it.

**Note on state tracking.** Detecting "manifest content has
changed" requires an in-memory hash maintained by `serve`, *not*
persisted state on disk and *not* a per-repo trust file. This is
not the trust gate readmitted: nothing is approved, no per-repo
state is recorded, and the digest is meaningful only for the
current `serve` process's lifetime. (This is the
state-tracking-contradiction the reviewer flagged — resolved by
making the hash purely in-process.)

### 6.5 Other hardening that drops out cleanly

- **No `kind: shell`.** Shell-string commands are not a v0.6.0
  feature. Note: this is a *manifest schema* restriction, not a
  security boundary — a manifest can still declare
  `command: ["sh", "-c", "..."]` since `sh` is just an executable.
  The `kind: shell` ban prevents one common foot-gun (forgetting
  that `command` is argv, not a string), nothing more.
- **Argv only on the wire.** Action commands are `["argv", "as",
  "list"]`, never a single string. This is the protection the
  design *does* enforce: web-supplied fields cannot become shell
  metacharacters.
- **Template allowlist.** Closed set of variables; whole-argument
  substitution only. Free-text from the browser
  (`extra_instructions`) never enters argv — only a file path
  does.
- **Slug sanitisation for paths/branches.** `slug::is_valid`
  (`src/slug/mod.rs:99`) already rejects leading/trailing `-` and
  consecutive `--`, so option-injection via `--orphan` is blocked
  for issue slugs today. For values that flow into argv positions
  (paths, branch names), still use a `safe_slug()` helper and
  insert `--` separators in argv where the called program supports
  them. (Review F7.)
- **Environment policy split by kind (R10).** Generic `kind: exec`
  children inherit a *minimal* environment by default: `PATH`
  sanitised to remove repo-local directories;
  `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE`/`GIT_CONFIG`/
  `GIT_CONFIG_GLOBAL` unset before spawn; explicit allowlist per
  action for things like `ANTHROPIC_API_KEY`. Interactive
  `kind: workmux` and `kind: workmux-send` children get an
  *interactive-safe* allowlist that adds the variables an agent
  in tmux needs to function: `HOME`, `USER`, `LOGNAME`, `SHELL`,
  `TERM`, `LANG`, `LC_*`, `XDG_RUNTIME_DIR`, `SSH_AUTH_SOCK`,
  plus the per-action allowlist. Without these, `claude` in tmux
  silently fails. The set of inherited keys (not values) is
  recorded in the run JSON for audit.
- **Resolved executable path** (from `which` against the
  sanitised `PATH`) is recorded in the run JSON.
- **Bounded extra instructions.** `accept_extra_instructions:
  true` (R12 — renamed from `allow_extra_instructions` because
  it is a UI textarea toggle, not a permission boundary) caps at
  8 KiB. This is a manifest-size sanity bound, *not* a
  blast-radius reduction (review F32): the agent has full repo
  capability regardless. Free-text never enters argv (review F31);
  it lands only in `instructions_file`.
- **Log rendering.** stdout/stderr surfaced in the UI is rendered
  as text, not HTML, with ANSI escape codes stripped (review F22).
  Agent output is untrusted.
- **No remote actions by default.** `--allow-remote-writes`
  *plus* `--enable-actions` does not imply remote actions; need
  an explicit `--allow-remote-actions` (which we do not ship in
  v0.6.0). Action read endpoints (`GET /api/actions`,
  `GET /api/runs/<id>/logs`) follow the same gate — they leak
  argv, prompt content, and instructions, so they must not be
  remote-readable just because issue reads are.
- **Filesystem detection** (review F25). At `serve --enable-actions`
  startup, `statfs` the run-state directory. If it's `nfs`,
  `fuse.dropbox`, `fuse.osxfuse`, `smbfs`, or any other
  known-bad-for-flock filesystem, refuse to start unless
  `--unsafe-shared-fs` is passed, with a banner explaining the
  silent-corruption failure mode.

## 7. UX shape

A single click on a card surfaces an action menu derived from
`/api/actions`. Selecting an action opens a small modal with:

- the action's label/description,
- an editable "extra instructions" textarea (when
  `accept_extra_instructions: true`),
- a preview of the **fully resolved argv** (R19 — this is a
  load-bearing security affordance, not just UX nicety; the user
  reviews argv before clicking),
- for `kind: workmux`, also the rendered prompt template's first
  N lines with a "view full" expander so the user can review what
  the agent will receive,
- a "Run" button.

After submit:

- transient toast "queued #abc123" with the run id,
- a per-card "runs" inset showing the most recent N runs with
  status pills. Pills are kind-aware:
  - `kind: workmux` reflects the workmux state via the
    translation table in §4.2.1: `working` → "running", `done` →
    "complete", `idle` → "waiting", anything else → "running
    (unknown)" (R5);
  - `kind: workmux-send` shows **"delivered"** as the terminal
    pill, with a tooltip "agent outcome unknown — open the tmux
    window to see what happens" (R3);
  - `kind: exec` shows `queued` / `running` / `complete` /
    `failed` / `cancelled` / `lost`.
- click into a run for full status + log tail. For `kind: exec`
  this is `GET /api/runs/<id>/logs` against our log files; for
  `kind: workmux` this is the result of `workmux capture
  issuectl-<run_id> -n <lines>`, ANSI-stripped, with a hard 2 s
  server-side timeout, a max-bytes cap on the response, and a
  ceiling of 5000 on `?lines=` regardless of what the client
  asks for (R13).
- "Attach in tmux" link for `kind: workmux` runs renders a
  copy-pasteable `tmux attach -t <session> \; select-window -t
  issuectl-<run_id>` derived from `workmux status`.
- run lifecycle events flow on the **existing `/events`** SSE
  channel as a new `RunUpserted` payload variant, not on a
  separate `/events/runs` stream. (Review F12 / DISCUSS resolution
  in `history/review-web-control-surface.md`.)
- **Manifest-changed banner (R11):** the kanban polls
  `manifest_digest` from `/api/actions` (or receives it on
  `RunUpserted` events). If the live digest differs from the
  digest captured at page load, a yellow banner appears at the
  top: "Actions changed since this page loaded — refresh to see
  the current set." Clicking refresh re-fetches `/api/actions`
  and re-renders the action menu. R6 (manifest_digest binding on
  POST) is what *enforces* the user resolves this before
  clicking; the banner is the visibility layer.

For v0.6.0, "Attach in tmux" is a copy-pasteable hint, not a deep
link. The OS-launcher problem is real but not on the critical
path — the user is at the terminal anyway.

## 8. Reach: single-user, multi-agent, cross-machine

| Concern | v0.6.0 | Later |
| --- | --- | --- |
| Single user, single agent at a time | ✓ in-proc runner inside `serve`, dispatches to `workmux` | — |
| Single user, multiple agents on one box | ✓ via `workmux` — multiple worktrees with separate windows; runner serialises only the *enqueue* path, not agent execution | — |
| **"Send to my already-running Claude"** | ✓ `kind: workmux-send` to a worktree the user has already opened (in this kanban or via `/worktree`) | — |
| Cross-machine via shared filesystem | **refused at startup** unless `--unsafe-shared-fs` (review F25) | reject as out of scope |
| Cross-machine via auth'd HTTP | not in v0.6.0 | `issuectl runner connect <url> --token …`, gated by `--allow-remote-runners`. Out-of-process `issuectl runner` claiming jobs over HTTP. |
| Multiple `serve` processes on one machine | not in v0.6.0 | Probably never useful for a single-user tool |

Filesystem-queue cross-machine looks tempting (shared NFS, a
checked-in symlink, etc.) and we should explicitly *not* try. NFS
`rename(2)` is not atomic the way local POSIX is; a queue built on
that breaks at exactly the wrong time.

## 9. Recommended v0.6.0 slice

**In:**

1. `.issuectl/actions.yaml` parser with strict schema, schema
   `version: 1`. Future versions parse-or-refuse with a clear
   error (review F19).
2. `kind: workmux` action. Runner: `workmux add`,
   `workmux wait` (no polling, R1), log-on-demand via
   `workmux capture`. Pre-flight checks `workmux --version
   ≥ MIN_WORKMUX` and that the configured `agent` exists in
   the sanitised PATH. Status translation enum (R5) shields
   the runner from workmux schema drift.
3. `kind: workmux-send` action. Runner: target resolution from
   live `running/` records (R7), `workmux send <target> --file`,
   terminal state `delivered` (R3). `per_target: 1` with 1 s
   cooldown by default (R4).
4. `kind: exec` generic action with argv-only commands, closed
   template-variable allowlist, process-group spawn (`setsid`),
   per-stream log size cap (10 MiB default, configurable),
   minimal environment policy (R10). `per_issue: 1` default (R4).
5. Run queue under `git rev-parse --git-common-dir`/issuectl/runs/
   (with XDG fallback for non-git repos) and the lock + state-
   machine + `preparing/` reservation described in §4 / §5.
6. Embedded runner inside `issuectl serve --enable-actions`,
   detached via `setsid` so `serve` restart does not kill running
   children (review F9). No external `issuectl runner` binary
   yet.
7. HTTP API (loopback-only by default):
   - `GET /api/actions` — manifest, availability flags, resolved
     argv preview, `manifest_digest` (R6).
   - `POST /api/actions/<id>/runs` — enqueue. Requires
     `Idempotency-Key` (TTL 1h, scoped per action+slug — R15)
     and `manifest_digest` (R6: 409 `manifest_changed` on
     mismatch).
   - `GET /api/runs?slug=<slug>` and `GET /api/runs/<id>` — status.
   - `GET /api/runs/<id>/logs/{stdout,stderr}` — log tails for
     `kind: exec`; for `kind: workmux` proxies to
     `workmux capture` with 2 s server timeout, max-bytes cap,
     and `?lines=` ceiling of 5000 (R13).
   - `POST /api/runs/<id>/cancel` — cancel; dispatches the
     escalation ladder in §5.3 (R2).
   - `GET /events` — `RunUpserted` payload variants flow on the
     existing `EventHub`/SSE channel (review F12 resolution),
     with the same `subscribe_since`/`instance_id` race-free
     handoff as issue events (review F11).
8. CLI parity:
   - `issuectl run <action> <slug> [--instructions ...]
     [--expected-version ...] [--json]`.
   - `issuectl runs list|show|cancel|logs`.
9. **Visibility surface (§6.4):** terminal banner on
   `serve --enable-actions` startup with fully resolved argv
   per action; web-UI yellow banner on `manifest_digest` change
   between page load and current; modal preview of resolved
   argv before submission. None of this gates execution.
10. Stale-run reaper using `host_id` + `started_at`-aware PID
    checks; for `kind: workmux`, reconciles against
    `workmux status --json` (review F29).
11. Filesystem detection at startup; refuse to start on
    NFS/Dropbox/etc. without `--unsafe-shared-fs` (review F25).
12. `.issuectl/AGENTS.md` (when it lands) is included in rendered
    context bundles. Treated as informational policy text, not
    enforcement.

**Deferred (own follow-up issues):**

- External `issuectl runner` binary + capability advertisement +
  pairing token. Useful when we want a runner that survives
  `serve` restart cleanly.
- `--allow-remote-actions` + remote runner registration over
  HTTP. (Cross-machine story.)
- `kind: shell` (gated, dangerous). Almost certainly never.
- `kind: webhook` for tools that natively expose a local
  listener.
- Three-way "open existing run / cancel and start new / queue
  anyway" UX when per-issue concurrency is hit. v0.6.0 just
  rejects with `409 already_running` and the UI surfaces a link
  to the running run.
- Worktree/run GC story (`issuectl runs gc`, `workmux remove
  --gone` integration); see SPIN-OFF F21 in the assessment.
- `/loop` / scheduled actions. Out of scope for control surface.

**Explicitly not building, ever:**

- Parent-terminal injection.
- Any in-process `sh -c` of strings derived from web requests.
- A second SSE channel just for runs.

## 10. Trade-offs the design accepts

- **`workmux` is a hard dependency for `kind: workmux`/
  `workmux-send` actions.** This is intentional, not incidental.
  A `kind: exec` escape hatch exists for users who don't have
  `workmux` (or who prefer `zellij`/`kitty`/`screen` and write
  their own script). Locking ourselves into `workmux` for the
  primary worktree action is the price we pay for not
  reimplementing agent multiplexing inside `issuectl`.
- **Run state lives under `<git-common-dir>/issuectl/runs/`, not
  `.issuectl/runs/`.** Browser-supplied instructions land here as
  artefacts; committing them by accident is a real failure mode.
  Discovery cost is paid once via `issuectl runs show`.
- **The schema-defined-actions approach (option 3 in the original
  spike framing) ships *first*, not as a fallback.** Free-form
  prompt-from-web-into-server-into-agent is not on the path. The
  underlying ask is "click and start work"; named actions deliver
  that without taking on a remote-shell-shaped surface.
- **`workmux` and `workmux-send` are structured kinds, not
  `kind: exec` recipes.** The runner needs to do specific things
  (pre-flight version pinning, blocking on `workmux wait`,
  status-enum translation, log proxying via `workmux capture`,
  structured cancellation escalation); pushing these into YAML
  trades a one-time integration for ongoing manifest complexity
  in every repo. Honest trade-off named in the round-1 assessment
  as F28-DISCUSS, resolved here in favour of structured kinds.
- **Status mutation is a runner concern, not an enqueue concern.**
  This was the original design's worst structural bug (F1/F2);
  the revised §5.1 makes the run record durable before any side
  effect, which means failures always surface as visible run
  records rather than torn issue state.
- **No second SSE channel.** Run lifecycle multiplexes onto
  `/events`. (DISCUSS F12 → resolved towards multiplex.)
- **`actions.yaml` lives in the repo, not in
  `$XDG_CONFIG_HOME`.** Shared, version-controlled actions are
  the whole point of the feature.
- **No content-hash trust gate.** `actions.yaml` is treated as
  authored code (same as Makefile/npm scripts/git hooks). The
  user is responsible for reading it before running
  `serve --enable-actions` in a freshly-cloned repo. The
  visibility surface in §6.4 (modal argv preview + UI banner on
  manifest change + terminal banner) makes drift visible without
  gating it. Review findings F4/F5/F23/F33 are accepted as
  out-of-scope risks given this trade-off — the alternative
  (re-prompt on every legitimate manifest edit) trains users to
  click-through and was judged worse.
- **`manifest_digest` binding on POST is not a trust gate.**
  R6 from the round-2 review adds a `manifest_digest` field to
  `GET /api/actions` and requires it on `POST .../runs`. This
  prevents the user from clicking a button whose argv changed
  between page load and submit; it does *not* approve anything,
  store per-repo state, or prompt the user. Same shape as
  `expected_version` for issues. Importantly small, importantly
  not the trust gate readmitted.

## 11. Open questions for follow-up review

1. **`workmux` is now load-bearing.** The design assumes its
   presence on user machines and its API stability. Revisit if
   `workmux` ever gets retired or substantially repackaged.
   Mitigation: action availability checks `workmux --version`
   and surfaces "workmux not found" in the kanban rather than
   crashing.
2. **Snapshot timing of the context bundle.** Render at enqueue
   (preserves the state the user clicked on) vs at claim
   (preserves freshness). §5.1 picks enqueue. Reversible
   per-action via a future manifest field if a real use case
   appears.
3. **`AGENTS.md` enforcement vs documentation.** Today we treat
   it as text appended to the context bundle. If it ever needs
   to *gate* actions ("agents may not run `kind: shell`"),
   that's a structured-policy file, not markdown. Defer.
4. **Concurrency defaults are now uniformly 1** (R4 resolution):
   `kind: workmux` → `per_issue: 1`, `kind: workmux-send` →
   `per_target: 1` + 1 s cooldown, `kind: exec` → `per_issue: 1`.
   Manifest authors opt in to parallelism explicitly. Open
   sub-question: is `per_issue` even the right axis for `exec`?
   Some `exec` actions are repo-global (`cargo test`), not
   per-issue. Add `per_repo` and `per_action` axes if the v0.6.0
   user feedback shows real use.
5. **How to surface action availability when a dependency
   disappears.** `workmux --version` at startup is fine, but
   each kanban request also needs to detect "workmux uninstalled
   while serve was running". Probe at action-list read with TTL
   cache (e.g. 30 s) plus on-demand recheck on enqueue.
6. **`workmux-send` target resolution under multi-run history**
   (R7 follow-on). The design resolves targets by scanning live
   `running/<id>.json` for the most recent `kind: workmux` run
   matching the slug. If the user has multiple worktrees open
   for the same issue (rare but possible — long-running plus a
   parallel investigation branch), "most recent" picks one
   silently. Future UX iteration: surface a picker in the modal
   when more than one live workmux run matches.

## 12. Spin-off issues to file (recommendations)

After user checkpoint, the following issues are worth filing as the
v0.6.0 implementation slice — none should be opened by the spike
itself.

1. **Action manifest + run queue + embedded runner** — schema
   parser, queue state machine with `preparing/`, lock contract,
   `kind: exec` execution. The atomic primitive everything else
   depends on.
2. **`kind: workmux` + `kind: workmux-send` action kinds** —
   wraps `workmux add`/`send`/`status`/`wait`/`capture` into
   callable action kinds. Depends on (1). Supersedes the
   precursor @excessively-beneficial-owner.
3. **CLI parity: `issuectl run` + `issuectl runs …`** — depends
   on (1).
4. **Filesystem detection + `--unsafe-shared-fs`** — small,
   self-contained, depends on (1).
5. **Startup actions banner + `--quiet-actions-banner`** —
   small, depends on (1) and the manifest parser.
6. **Worktree/run GC story** — `issuectl runs gc`, integration
   with `workmux remove --gone`, retention policy. Spin-off F21.
7. **External `issuectl runner` + capability + pairing** — the
   v0.7.0+ story, filed as a follow-up.

The narrow precursor @excessively-beneficial-owner stays open and
gets superseded by (2). Whoever picks up (2) is responsible for
closing the precursor with a `Source: <issue>` cross-reference.

---

## References

- `docs/design/web-edit-sync.md` — mutation protocol the run queue
  inherits the threat model from. The `EventHub` race-free
  handoff (§5.5 there) is reused for `RunUpserted` lifecycle
  events.
- `docs/design/body-sections.md` — `## Comments` is the canonical
  body section for free-form notes; the cross-reference on
  @excessively-beneficial-owner uses it.
- `history/review-web-control-surface.md` — round-1 review +
  assessment. F1–F3, F6, F8–F12, F14, F17–F19, F22, F25, F29,
  F31–F32 integrated; F21 SPIN-OFF; F26, F28 resolved by
  `workmux`; F4, F5, F23, F33 accepted as out-of-scope risks
  given the no-trust-gate decision in §6.
- `history/review-web-control-surface-r2.md` — round-2 review +
  assessment (post-workmux + post-trust-removal). R1–R15, R17,
  R19 integrated above; R16 and R18 documented in-place as
  known footguns/limitations rather than mechanically fixed.
- `workmux --help` — local agent multiplexer; assumed dependency.
  Subcommands the design relies on: `add`, `send`, `status`,
  `wait`, `capture`, `merge`, `remove`.
- `.issuectl/prompts/implement.md` — already-shipped prompt
  template the `kind: workmux` action renders against the
  `issuectl context <slug>` bundle.
- @excessively-beneficial-owner — narrow precursor; superseded in
  scope by this note, kept open for the follow-up implementation
  ticket.
- @profoundly-domineering-wound (closed `2026-05-08`) — landed
  the agent context bundle this design depends on.
- @markedly-terrific-angle — planned `.issuectl/AGENTS.md`; cited
  but not depended on by v0.6.0.
