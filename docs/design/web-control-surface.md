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
    base: "main"                       # passed to `workmux add --base`
    # branch name = slug; workmux derives window name from it
    agent: "claude"                    # tells workmux which agent to launch in the pane
    allow_extra_instructions: true
    on_start_runner:                   # status mutation runs in the RUNNER, not enqueue (review F1)
      set_status: in-progress
      require_expected_version: true
    concurrency:
      per_issue: 1

  send-to-active:
    label: "Send to active worktree"
    description: "Deliver an extra prompt to a currently-open agent for this issue"
    kind: workmux-send
    target: "{{slug}}"                 # workmux worktree name; defaults to slug
    prompt: ".issuectl/prompts/follow-up.md"
    allow_extra_instructions: true
    concurrency:
      per_issue: unbounded             # send is idempotent-ish; let the user spam

  context-dump:
    label: "Print context"
    kind: exec
    command: ["issuectl", "context", "{{slug}}"]
    # no terminal; logs captured to logs/<run_id>.{stdout,stderr}
```

Template variables resolve only against a closed allowlist:
`{{slug}}`, `{{run_id}}`, `{{repo_root}}`, `{{prompt_file}}`,
`{{context_file}}`, `{{instructions_file}}`. Whole-argument
substitution only; no interpolation into shell strings. Browser-
supplied "extra instructions" *never* enter argv — they land only
in `instructions_file`, which the agent reads. (Review F31.)

### 4.2 Action kinds: `workmux`, `workmux-send`, `exec`

#### `kind: workmux`

The runner's execution path is roughly:

```
1. workmux add \
     --name <run_id_or_slug> \
     --prompt-file <artifacts/<run_id>/prompt.md> \
     --base <action.base> \
     <slug>                          # branch name
2. (optional, when on_start_runner.set_status set)
   issuectl --json update <slug> --status in-progress \
                                  --expected-version <run.issue_version>
3. workmux wait <run_id> --status done --timeout <action.timeout>
   ↓ status callbacks update running/<id>.json with workmux status JSON
4. on completion: workmux merge <run_id>      (or leave it; F21 punts the auto-cleanup question)
```

This is the entire `worktree` story. `workmux add` does the
worktree-create + tmux-window + prompt-injection + agent-launch in
one call. `workmux wait` is what tells us when the agent is
genuinely done (review F15: no more "tmux client exited so we
marked it complete while Claude kept running"). Logs are read on
demand via `workmux capture <run_id> -n <lines>`; we do not
maintain our own stdout file for this kind.

Pre-flight check at action-availability time: `workmux --version`
succeeds and the configured `agent` exists in PATH. Both surface
through `GET /api/actions` so the UI can disable unavailable
actions with an explicit reason.

#### `kind: workmux-send`

```
1. workmux send <action.target> --file <artifacts/<run_id>/prompt.md>
2. mark complete immediately (send is fire-and-forget; receiver decides what to do)
```

The user clicks → the prompt lands in the worktree they already
have open in their tmux. No new window appears. This is the
v0.6.0 answer to "send to my already-running Claude." The action
fails if the named worktree doesn't exist (`workmux send` returns
non-zero); the UI surfaces the error and offers to fall back to
`kind: workmux` (open a fresh worktree).

`per_issue: unbounded` is the right default for this kind:
sending follow-up prompts is benign and frequently rapid-fire.

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
- "Auto-detect tmux session and launch there" — explicit
  `workmux-send target` only.

This list matters. The whole reason `kind: workmux` is a structured
kind, not a `kind: exec` calling `workmux` from YAML, is that the
runner needs to do specific things around it: pre-flight version
check, status polling via `workmux wait`, log reads via
`workmux capture`, and lifecycle integration with `workmux merge`/
`remove --gone`. Pushing all of that into a YAML recipe trades a
one-time Rust integration for ongoing manifest complexity in every
repo.

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
  |  Idempotency-Key: <uuid>                         |  (review F10)
  |  { slug, expected_version, instructions }        |
  |  ------------------------------------------>     |
  |                                                  |
  |                 1. validate CSRF + Host          |
  |                 2. validate action_id ∈ manifest |
  |                 3. RECHECK manifest+prompt       |
  |                    digest against trust file     |  (review F4/F5)
  |                    → 409 manifest_changed if     |
  |                    re-trust required             |
  |                 4. flock(queue.lock)             |
  |                    a. check Idempotency-Key      |
  |                       → return existing run if   |
  |                       already accepted           |
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
- **Manifest digest is re-checked** at step 3 against the
  trust-file digest *plus* the digest of any prompt files the
  manifest references. (Review F4/F5; see §6.)
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
6.a. (optional, when on_start_runner.set_status set)
     issuectl --json update <slug> --status in-progress \
                                   --expected-version <run.issue_version>
     ↳ failure → run → failed (state_reason: issue_changed)
6.b. workmux add --name <run_id> \
                 --prompt-file artifacts/<run_id>/prompt.md \
                 --base <action.base> \
                 <slug>
     ↳ failure → run → failed (state_reason: workmux_add_failed)
6.c. heartbeat loop:
       workmux status <run_id> --json → write to running/<id>.json
       sleep N seconds
     until status ∈ {done, failed, idle-too-long} OR
           cancel_requested becomes true
6.d. on cancel_requested: workmux send <run_id> "<C-c>" then poll
     status until exit
6.e. terminal disposition:
       done       → complete/
       failed     → failed/
       cancelled  → cancelled/
```

Logs are read on demand (`GET /api/runs/<id>/logs` →
`workmux capture <run_id> -n <lines>`); we don't maintain a
stdout file for this kind. Cleanup of the worktree itself is
*not* automatic in v0.6.0 — the user runs `workmux merge` /
`workmux remove --gone` from their own workflow. (See §11.)

#### 5.2.2 `kind: workmux-send`

```
6.a. workmux send <action.target> --file artifacts/<run_id>/prompt.md
6.b. exit code 0 → complete; nonzero → failed
     (target_not_found, workmux_unavailable, etc.)
```

This kind is fire-and-forget by definition: there is no agent
process *we own* to wait on. The receiving worktree's agent
decides what to do with the prompt. Cancel is a no-op once
`workmux send` has returned.

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

The runner picks up `cancel_requested` at the next heartbeat /
poll boundary in §5.2. For `kind: workmux` this triggers
`workmux send <run> "<C-c>"`; for `kind: exec` it kills the
process group. PID validation against start-time happens before
any signal is sent (review F29).

### 5.4 Stale-run reaper

On `serve` startup and every N minutes thereafter, iterate
`running/`. If `host_id == this_host_id` and the recorded
`runner_id` is no longer alive, mark `lost`. (`host_id` is
generated once per host into `$XDG_STATE_HOME/issuectl/host-id`,
not derived from hostname — hostnames are unstable.) For
`kind: workmux` runs, additionally reconcile against
`workmux status --json`: if the runner died but the workmux
worktree is alive and the agent is `done`, transition to
`complete` rather than `lost`.

Cross-host runs (`host_id != this_host_id`) are left alone — that's
the future multi-machine story and we don't speculatively reap.

## 6. Trust, security, blast radius

Three distinct trust boundaries, each enforced separately:

1. **Repo trust.** `.issuectl/actions.yaml` is in-repo and therefore
   carried by `git checkout` and `git pull`. A drive-by clone of a
   malicious repo followed by `issuectl serve --enable-actions` is
   an RCE if we auto-trust. **Do not auto-trust, and do not treat
   trust as set-and-forget** (review F4).

   The trust file
   (`$XDG_CONFIG_HOME/issuectl/trusted-repos.json`) records, per
   trusted repo: the canonical repo common-dir path, *the SHA-256
   digest of `.issuectl/actions.yaml` plus the contents of every
   prompt file the manifest references, concatenated in a
   canonical order*, and a `trusted_at` timestamp. (Review F5:
   prompt-file changes are equally dangerous as argv changes —
   prompt injection is real.)

   On every `POST /api/actions/<id>/runs`, the server re-hashes
   the manifest+prompt material and compares against the stored
   digest. Mismatch → 409 `manifest_changed` with a diff in the
   response, action UI disables, user must re-trust. The trust
   prompt shows the resolved `argv` (not just labels) so users
   review what will actually run, not a friendly description.

   Until trusted, the kanban shows actions as visible-but-disabled
   with a "this repo defines N actions; review and trust to
   enable" affordance.

2. **Web → server trust.** Same as today: per-process CSRF token,
   `Host` allowlist (loopback aliases only by default). Actions
   inherit this surface; the new `--enable-actions` flag is an
   *additional* opt-in on top of `--allow-remote-writes`, not a
   reuse of it.

3. **Server → runner trust.** When the runner is in-process, this
   is internal and trivial. When external (future), pairing
   requires a runner token (`issuectl runner-token create`) and the
   server gates `/api/runners/*` by it.

Other hardening that drops out cleanly. **Stating the actual
security boundary plainly** (review F6): argv-only and template
allowlists protect against *web-injected* command strings. They do
*not* sandbox the action manifest itself. A trusted manifest can
declare `command: ["sh", "-c", "rm -rf $HOME"]` or
`["python", "-c", "..."]` and it will run. The trust gate is the
only thing standing between a cloned repo and arbitrary code
execution. Argv-only buys you "a malicious browser tab on the
loopback can't inject metacharacters"; it does *not* buy you "a
checked-in `actions.yaml` is sandboxed".

- **No `kind: shell`.** Shell-string commands are not a v0.6.0
  feature. If we ever add them, they live behind
  `dangerous_shell: true` per-action plus a global
  `--enable-shell-actions` flag, with a startup warning.
- **Argv only.** Action commands are `["argv", "as", "list"]`,
  never a single string.
- **Template allowlist.** Closed set of variables; whole-argument
  substitution only.
- **Slug sanitisation for paths/branches.** `slug::is_valid`
  (`src/slug/mod.rs:99`) already rejects leading/trailing `-` and
  consecutive `--`, so option-injection via `--orphan` is blocked
  for issue slugs today. For values that flow into argv positions
  (paths, branch names), still use a `safe_slug()` helper and
  insert `--` separators in argv where the called program supports
  them. (Review F7.)
- **Environment policy** (review F17): children inherit a *minimal*
  environment by default. `PATH` is sanitised to remove repo-local
  directories; `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`,
  `GIT_CONFIG`, `GIT_CONFIG_GLOBAL` are unset before spawn.
  Explicit allowlist per action for things like
  `ANTHROPIC_API_KEY`, `SSH_AUTH_SOCK`. Resolved executable path
  (from `which`) is recorded in the run JSON for audit.
- **Bounded extra instructions.** `allow_extra_instructions: true`
  caps at 8 KiB. This is a manifest-size sanity bound, *not* a
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
- an editable "extra instructions" textarea (when allowed),
- a preview of the resolved argv (so the user can see what will
  run — for `kind: workmux` this is the resolved
  `workmux add …` invocation),
- a "Run" button.

After submit:

- transient toast "queued #abc123" with the run id,
- a per-card "runs" inset showing the most recent N runs with
  status pills. Pills are kind-aware: a `kind: workmux` run shows
  the live workmux agent status (`thinking`, `running`, `idle`,
  `done`) rather than a fake `running` for the duration. (Review
  F15 — the lifecycle distinction is concrete because workmux
  reports it.)
- click into a run for full status + log tail. For `kind: exec`
  this is `GET /api/runs/<id>/logs` against our log files; for
  `kind: workmux` this is the result of `workmux capture <run> -n 200`,
  ANSI-stripped.
- "Attach in tmux" link for `kind: workmux` runs renders a
  copy-pasteable `tmux attach -t <session> \; select-window -t <run_id>`
  derived from `workmux status --json`.
- run lifecycle events flow on the **existing `/events`** SSE
  channel as a new `RunUpserted` payload variant, not on a
  separate `/events/runs` stream. (Review F12 / DISCUSS resolution
  in `history/review-web-control-surface.md`.)

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
2. `kind: workmux` action. The runner shells out to `workmux add`,
   polls `workmux status --json`, reads logs via
   `workmux capture`. Pre-flight checks `workmux --version` and
   resolves the configured agent in `PATH`.
3. `kind: workmux-send` action. The runner shells out to
   `workmux send <target> --file <prompt>`. Pre-flight checks
   that the target worktree exists.
4. `kind: exec` generic action with argv-only commands, closed
   template-variable allowlist, process-group spawn (`setsid`),
   per-stream log size cap, environment policy.
5. Run queue under `git rev-parse --git-common-dir`/issuectl/runs/
   (with XDG fallback for non-git repos) and the lock + state-
   machine + `preparing/` reservation described in §4 / §5.
6. Embedded runner inside `issuectl serve --enable-actions`,
   detached via `setsid` so `serve` restart does not kill running
   children (review F9). No external `issuectl runner` binary
   yet.
7. HTTP API (loopback-only by default):
   - `GET /api/actions` — manifest, availability flags, resolved
     argv preview.
   - `POST /api/actions/<id>/runs` — enqueue. Requires
     `Idempotency-Key` (review F10).
   - `GET /api/runs?slug=<slug>` and `GET /api/runs/<id>` — status.
   - `GET /api/runs/<id>/logs/{stdout,stderr}` — log tails for
     `kind: exec`; for `kind: workmux` proxies to
     `workmux capture`.
   - `POST /api/runs/<id>/cancel` — cancel.
   - `GET /events` — `RunUpserted` payload variants flow on the
     existing `EventHub`/SSE channel (review F12 resolution),
     with the same `subscribe_since`/`instance_id` race-free
     handoff as issue events (review F11).
8. CLI parity:
   - `issuectl run <action> <slug> [--instructions ...]
     [--expected-version ...] [--json]`.
   - `issuectl runs list|show|cancel|logs`.
9. Trust gating: `issuectl trust` records repo + manifest+prompt
   digest in `$XDG_CONFIG_HOME/issuectl/trusted-repos.json`.
   Server re-hashes on every action invocation; mismatch → 409
   `manifest_changed`. Trust prompt shows resolved argv (review
   F4/F5/F23).
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
- Action manifests trusted automatically on first checkout.
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
  (pre-flight, status polling, log proxying, lifecycle); pushing
  these into YAML trades a one-time integration for ongoing
  manifest complexity in every repo. Honest trade-off named in
  the assessment as F28-DISCUSS, resolved here in favour of
  structured kinds.
- **Status mutation is a runner concern, not an enqueue concern.**
  This was the original design's worst structural bug (F1/F2);
  the revised §5.1 makes the run record durable before any side
  effect, which means failures always surface as visible run
  records rather than torn issue state.
- **No second SSE channel.** Run lifecycle multiplexes onto
  `/events`. (DISCUSS F12 → resolved towards multiplex.)
- **`actions.yaml` lives in the repo, not in
  `$XDG_CONFIG_HOME`.** This means trust must be content-hashed
  and re-validated, which we accept (F4). The alternative
  (user-config-only actions) was raised in DISCUSS F27 and
  rejected: shared actions are the whole point. Show resolved
  argv in the trust prompt; re-prompt on argv changes.

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
4. **Per-issue concurrency defaults by `kind`** — `workmux` →
   1, `workmux-send` → unbounded, `exec` → unbounded — or always
   require explicit declaration in the manifest? Review F18
   raised this; the design above picks per-kind defaults but the
   alternative (always explicit) is reasonable.
5. **How to surface action availability when a dependency
   disappears.** `workmux --version` at startup is fine, but
   each kanban request also needs to detect "workmux uninstalled
   while serve was running". Probe at action-list read with TTL
   cache (e.g. 30 s) plus on-demand recheck on enqueue.
6. **`workmux-send` target naming.** The current example uses
   `target: "{{slug}}"`, assuming the worktree was named after
   the slug. If `workmux add` was given an explicit `--name`
   that diverges, the send target is ambiguous. Either record
   the workmux name in the issue frontmatter on `kind: workmux`
   completion, or query `workmux list --json` and let the user
   pick from a dropdown. Defer to UX iteration.

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
4. **Trust gating: `issuectl trust` + manifest+prompt digest +
   per-request re-validation + UI argv preview** — depends on
   (1). Review F4/F5 makes this a v0.6.0 must-have, not a
   follow-up.
5. **Filesystem detection + `--unsafe-shared-fs`** — small,
   self-contained, depends on (1).
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
- `history/review-web-control-surface.md` — full review +
  assessment table that drove the revisions in this version of
  the note. F1, F2, F3, F4, F5, F6, F8, F9, F10, F11, F12, F14,
  F17, F18, F19, F22, F25, F29, F31, F32 are integrated above;
  F21 is the named SPIN-OFF; F26 and F28 are now resolved by the
  `workmux` assumption.
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
