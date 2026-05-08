# Web control surface — design spike

Status: **design note (spike output)**, not an implementation contract.
Supersedes the narrow precursor @excessively-beneficial-owner ("start
implementation" button) by widening the question: how does the kanban
become a *control surface* for issue-related work, not just an
issue-text editor.

The brainstorm input came from a multi-LLM `/llm-collab` session
(Gemini 3.1 Pro, GPT-5.5, DeepSeek v4 Pro). The synthesis here is
mine; the convergence across models was strong enough to skip a
build-on round.

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
- Reject on cohesion grounds. The runner-event channel in 2.3 should
  be a separate endpoint with explicit claim/ack.

## 3. Rubric

| Mechanism | Security | UX (latency / visibility / cancel) | OS port. | Reach | Impl cost | Blast radius |
| --- | --- | --- | --- | --- | --- | --- |
| 2.1 In-proc spawn | Medium — argv-only manifest helps, but server owns child lifecycle | Low latency, output buffering ad-hoc, cancel = kill PID | Good | Single machine, single runner | Low | High if `kind: shell` ever lands |
| 2.2 FS queue + runner | High — server only writes JSON; runner is the only exec surface | Medium latency (debounce or polling), durable status, cancel via state file | Excellent | Multi-runner local; **not** cross-machine | Medium | Low — exec in one place |
| 2.3 Agent-side runner over SSE | High when paired with token auth; loopback-only by default | Best UX — runner owns the terminal, status flows back live | Good (HTTP) | Multi-agent, cross-machine with explicit auth | Medium-High | Bounded by runner's own capabilities |
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

**Runner** — an *executor*. A process that claims runs and executes
them. Either embedded (`issuectl serve --enable-actions` runs an
in-process runner) or external (`issuectl runner --capability …`).
Execution pulls argv from the action declaration; the run never
ships argv from the wire.

```
.issuectl/actions.yaml          (declarations, committed)
.git/issuectl/runs/             (durable run state, NOT committed)
   queued/<run_id>.json
   running/<run_id>.json
   complete/<run_id>.json
   failed/<run_id>.json
   logs/<run_id>.{stdout,stderr}
   artifacts/<run_id>/
       context.md
       prompt.md
       instructions.txt
   queue.lock
```

`.git/issuectl/runs/` is the recommended location. `.git/` is
per-worktree, never committed, already excluded from `git status`.
Putting run logs/instructions in a *committed* path leaks
browser-supplied free text into git history; the spike's review
should confirm this. (Fallback: `$XDG_STATE_HOME/issuectl/<repo-hash>/`
if the `.git/` shape proves awkward across linked worktrees.)

### 4.1 Manifest shape (illustrative)

```yaml
version: 1
actions:
  implement:
    label: "Start implementation"
    description: "Create a worktree and start an agent on this issue"
    kind: worktree
    prompt: ".issuectl/prompts/implement.md"
    agent:
      command: ["claude"]      # argv array, never a shell string
    terminal:
      kind: tmux               # tmux | none
      session: "issuectl"
      window_template: "{{slug}}"
    worktree:
      base: "main"
      branch_template: "{{slug}}"
      path_template: "../worktrees/{{slug}}"
    allow_extra_instructions: true
    on_start:
      set_status: in-progress
      require_expected_version: true
    concurrency:
      per_issue: 1

  context-dump:
    label: "Print context"
    kind: exec
    command: ["issuectl", "context", "{{slug}}"]
    terminal:
      kind: none
```

Template variables resolve only against a closed allowlist:
`{{slug}}`, `{{run_id}}`, `{{repo_root}}`, `{{prompt_file}}`,
`{{context_file}}`, `{{instructions_file}}`. Whole-argument
substitution only; no interpolation into shell strings.

### 4.2 The `worktree` action kind is built-in, not generic exec

The brainstorm models converged here: making "spawn worktree + start
Claude" a structured action kind (rather than an `exec` shelling out
to a script) buys real things — pre-flight dependency check,
sensible window naming, per-issue concurrency, stable status events
for the UI, and a focused failure surface ("tmux not found" vs
"opaque exit 127"). The generic `exec` kind exists as the escape
hatch for anything else.

This is also the natural home for the existing `/worktree` skill's
mechanism. The skill becomes "the user-facing `/worktree` slash
command" *and* "the `worktree` action kind invoked from the kanban,"
sharing implementation in a `cli::worktree` module.

## 5. The contract: who holds which lock when

The single most important property of this design is **the issue
flock and the run-queue lock are different files**.

```
.issuectl/write.lock          ← existing issue mutation flock
.git/issuectl/runs/queue.lock ← new run-queue lock
```

If the issue flock is ever held *across an agent's lifetime*, the
issue lock becomes a denial-of-service primitive against the kanban,
the CLI, and any other agent. That is not acceptable.

### 5.1 Enqueue path (web → run created)

```
client                              server
  |  POST /api/actions/implement/runs                |
  |  X-Issuectl-CSRF: <tok>                          |
  |  { slug, expected_version, instructions }        |
  |  ------------------------------------------>     |
  |                                                  |
  |                 1. validate CSRF + Host          |
  |                 2. validate action_id ∈ manifest |
  |                 3. flock(write.lock)             |
  |                    a. locate_issue(slug)         |
  |                    b. read item.md, hash         |
  |                    c. compare expected_version   |
  |                    d. (optional) on_start:       |
  |                       set_status: in-progress    |
  |                       (full mutate, V → V')      |
  |                    e. RENDER context bundle      |
  |                       to artifacts/<run>/        |
  |                 4. release(write.lock)           |
  |                 5. flock(queue.lock)             |
  |                    write run.json to .tmp        |
  |                    rename → queued/<id>.json     |
  |                 6. release(queue.lock)           |
  |                 7. publish ActionRunCreated      |
  |                    on a separate /events/runs    |
  |  202 Accepted { run_id, status: "queued" }       |
  |  <------------------------------------------     |
```

Time the issue flock is held: long enough to read item.md, optionally
mutate status, and render the context bundle. **Not** long enough to
fork a process or wait on tmux. Steps 5–6 happen under the run-queue
lock, which is uncontended with the issue flock.

### 5.2 Claim + execute path (runner → child)

```
runner (in-proc or external)
  loop:
    1. flock(queue.lock)
    2. pick first queued/<id>.json matching capabilities
    3. rename → running/<id>.json; write runner_id, pid, started_at
    4. release(queue.lock)
    5. spawn argv from action manifest, redirect stdout/stderr to
       logs/<id>.{stdout,stderr}
    6. wait; on exit, flock(queue.lock); rename → complete/ or failed/
    7. release(queue.lock)
```

The runner never holds the issue flock during step 5. If the action
mutates the issue (e.g. records commits at the end), it does so by
shelling out to `issuectl --json update --expected-version …`, the
same path the CLI uses. Its `flock` call is short and bounded.

### 5.3 Cancellation

`POST /api/runs/<id>/cancel` writes a `cancel_requested: true` field
into the run JSON, and (if `running` and pid is local) sends `SIGTERM`
to the recorded pid. The runner converts to `cancelled` on observed
exit. SIGKILL escalation is manual; we don't promise to terminate
agents that ignore SIGTERM.

### 5.4 Stale-run reaper

On `serve` startup (and every N minutes during running), iterate
`running/`. If `host == this_host` and `pid` no longer exists, mark
`lost`. Cross-host runs are left alone — that's the future
multi-machine story and we don't speculatively reap.

## 6. Trust, security, blast radius

Three distinct trust boundaries, each enforced separately:

1. **Repo trust.** `.issuectl/actions.yaml` is in-repo and therefore
   carried by `git checkout`. A drive-by clone of a malicious repo
   followed by `issuectl serve --enable-actions` is an RCE if we
   auto-trust. **Do not auto-trust.** Require an explicit
   `issuectl trust` recorded *outside* the repo
   (`$XDG_CONFIG_HOME/issuectl/trusted-repos.json`, keyed by
   canonical repo path or content hash). Until trusted, the kanban
   shows actions as visible-but-disabled with a "this repo defines
   N actions; review and trust to enable" affordance.

2. **Web → server trust.** Same as today: per-process CSRF token,
   `Host` allowlist (loopback aliases only by default). Actions
   inherit this surface; the new `--enable-actions` flag is an
   *additional* opt-in on top of `--allow-remote-writes`, not a
   reuse of it.

3. **Server → runner trust.** When the runner is in-process, this
   is internal and trivial. When external (future), pairing
   requires a runner token (`issuectl runner-token create`) and the
   server gates `/api/runners/*` by it.

Other hardening that drops out cleanly:

- **No shell.** `kind: shell` is not a v0.6.0 feature. If we ever
  add it, it lives behind `dangerous_shell: true` per-action plus a
  global `--enable-shell-actions` flag, with a startup warning.
- **Argv only.** Action commands are `["argv", "as", "list"]`,
  never a single string.
- **Template allowlist.** Closed set of variables; whole-argument
  substitution only.
- **Slug sanitisation for paths/branches.** Even though
  `slug::is_valid` is strict today, paths and branch names should
  go through a `safe_slug()` helper that rejects anything outside
  `[A-Za-z0-9_-]+`.
- **Bounded extra instructions.** `allow_extra_instructions: true`
  caps at e.g. 8 KiB and the UI shows the user exactly what will be
  passed.
- **Log rendering.** stdout/stderr surfaced in the UI is rendered
  as text, not HTML. Agent output is untrusted.
- **No remote actions by default.** `--allow-remote-writes`
  *plus* `--enable-actions` does not imply remote actions; need
  an explicit `--allow-remote-actions` (which we do not ship in
  v0.6.0).

## 7. UX shape

A single click on a card surfaces an action menu derived from
`/api/actions`. Selecting an action opens a small modal with:

- the action's label/description,
- an editable "extra instructions" textarea (when allowed),
- a preview of the resolved argv (so the user can see what will run),
- a "Run" button.

After submit:

- transient toast "queued" with the run id,
- a per-card "runs" inset showing the most recent N runs with
  status pills (`queued`, `running`, `complete`, `failed`,
  `cancelled`, `lost`),
- click into a run for full status + log tail (`/api/runs/<id>/logs`),
- "Open terminal" link for `terminal: tmux` runs renders a
  copy-pasteable `tmux attach -t issuectl \; select-window -t <slug>`,
- a `/events/runs` SSE stream pushes status transitions live.

For v0.6.0, "open terminal" is a copy-pasteable hint, not a deep
link. The OS-launcher problem is real but not on the critical path.

## 8. Reach: single-user, multi-agent, cross-machine

| Concern | v0.6.0 | Later |
| --- | --- | --- |
| Single user, single runner | ✓ in-proc runner inside `serve` | — |
| Single user, multiple runners on one box | ✓ external `issuectl runner` claims by capability | ✓ |
| Cross-machine via shared filesystem | **not promised** — filesystem-queue semantics are local-loopback-correct only | reject as out of scope |
| Cross-machine via auth'd HTTP | not in v0.6.0 | `issuectl runner connect <url> --token …`, gated by `--allow-remote-runners` |
| "Send to my already-running Claude" | not in v0.6.0 | runner registers as `interactive-terminal`-capable and decides how to deliver (pty-pipe, `--resume`, etc.) |

Filesystem-queue cross-machine looks tempting (shared NFS, a
checked-in symlink, etc.) and we should explicitly *not* try. NFS
`rename(2)` is not atomic the way local POSIX is; a queue built on
that breaks at exactly the wrong time.

## 9. Recommended v0.6.0 slice

**In:**

1. `.issuectl/actions.yaml` parser with strict schema.
2. `kind: worktree` built-in action that reuses the `/worktree`
   skill's mechanism (`git worktree`, optional tmux session, agent
   argv). Pre-flight dependency check; surfaces clear errors when
   git/tmux/agent are missing.
3. `kind: exec` generic action with argv-only commands and the
   closed template-variable set.
4. Run queue under `.git/issuectl/runs/` (with XDG fallback) and
   the lock + state-machine described in §4 / §5.
5. Embedded runner in `issuectl serve --enable-actions`. No
   external runner yet.
6. HTTP API:
   - `GET /api/actions` — manifest, with availability flags
     (`available`, `unavailable_reason`).
   - `POST /api/actions/<id>/runs` — enqueue.
   - `GET /api/runs?slug=<slug>` and `GET /api/runs/<id>` — status.
   - `GET /api/runs/<id>/logs/{stdout,stderr}` — log tails.
   - `POST /api/runs/<id>/cancel` — cancel.
   - `GET /events/runs` — SSE for run lifecycle.
7. CLI parity:
   - `issuectl run <action> <slug> [--instructions ...]
     [--expected-version ...] [--json]` so the verb is reachable
     without the web.
   - `issuectl runs list|show|cancel|logs` for inspection.
8. Trust gating: `issuectl trust` records a repo as trusted in
   `$XDG_CONFIG_HOME/issuectl/trusted-repos.json`. Without it,
   actions are visible-but-disabled.
9. Stale-run reaper on startup.
10. `.issuectl/AGENTS.md` (when it lands) is included in rendered
    context bundles. Treated as informational policy text, not
    enforcement.

**Deferred (own follow-up issues):**

- External `issuectl runner` binary + capability advertisement +
  pairing token. (Multi-runner story.)
- `--allow-remote-actions` + remote runner registration. (Cross-
  machine story.)
- `kind: shell` (gated, dangerous). (Almost certainly never.)
- `kind: webhook` for tools that natively expose a local listener.
- Three-way "open existing run / cancel and start new / queue
  anyway" UX when per-issue concurrency is hit. v0.6.0 just
  rejects with `409 already_running` and the UI surfaces a link
  to the running run.
- Real "send to existing Claude session" via runner-side
  `--resume <session-id>` plumbing.
- `/loop` / scheduled actions. (Out of scope for control surface.)

**Explicitly not building, ever:**

- Parent-terminal injection.
- Any in-process `sh -c` of strings derived from web requests.
- Action manifests trusted automatically on first checkout.

## 10. Trade-offs the design accepts

- **No live "send to my already-running Claude" in v0.6.0.** The
  user wanting that workflow runs `issuectl runner` later, in the
  tmux pane where they want it; v0.6.0 doesn't have that runner.
  The interim experience is "click → a new tmux window appears
  with Claude on the issue", which is what the existing
  `/worktree` skill produces today, and is still a real
  improvement over "switch to terminal, run `/worktree #NN`".
- **Run state lives in `.git/issuectl/runs/`, not `.issuectl/runs/`.**
  We pay a small "where's my logs?" discoverability cost to avoid
  the much bigger "browser-supplied instructions land in git
  history" cost. `issuectl runs show` makes it ergonomic.
- **The schema-defined-actions approach (option 3) ships *first*,
  not as a fallback.** Free-form prompt-from-web-into-server-into-
  agent (option 1) is not on the path. The user's underlying ask
  is "click and start work"; named actions deliver that without
  taking on a remote-shell-shaped surface.
- **`worktree` is a built-in action kind, not just an `exec`
  recipe.** This adds Rust code we'd otherwise avoid, but the
  payoff is that "tmux not found", "agent not in PATH", "branch
  already exists" become structured errors the UI can act on,
  not exit-code-127 mysteries.

## 11. Open questions for follow-up review

1. **`.git/` vs XDG state dir.** `.git/` is per-worktree; XDG is
   per-machine. Linked worktrees (the user's whole `/worktree`
   workflow) might *want* shared run history. Worth a focused
   consult before implementation.
2. **Snapshot timing of the context bundle.** Render at enqueue
   (preserves the state the user clicked on) vs at claim
   (preserves freshness). §5.1 picks enqueue; this is reversible
   per-action via a manifest field if a real use case appears.
3. **`AGENTS.md` enforcement vs documentation.** Today we treat it
   as text appended to the context bundle. If it ever needs to
   *gate* actions ("agents may not run `kind: shell`"), that's a
   structured-policy file, not markdown. Defer the question.
4. **Per-issue concurrency: "1" or configurable?** Defaulting to 1
   is right for `worktree`. Some `exec` actions are fine to run
   concurrently. Make it a per-action knob.
5. **How to surface action availability when a dependency is
   missing.** `which tmux` at startup is fine for `serve --enable-
   actions`, but each kanban request also needs to say "tmux
   disappeared". Fast re-check on demand or at most every N
   seconds.

## 12. Spin-off issues to file (recommendations)

After user checkpoint, the following issues are worth filing as the
v0.6.0 implementation slice — none should be opened by the spike
itself.

1. **Action manifest + run queue + embedded runner** — the §9 "in"
   list, minus the worktree-kind specifics. (The atomic primitive.)
2. **`worktree` action kind** — wraps the `/worktree` skill
   mechanism into a callable action; depends on (1).
3. **CLI parity: `issuectl run` + `issuectl runs …`** — depends on (1).
4. **Trust gating: `issuectl trust` + UI affordance** — depends on (1).
5. **Run-queue location: `.git/issuectl/runs/` vs XDG.** Smaller
   focused spike to settle question §11.1 before (1) lands.
6. **External runner + capability + pairing** — the v0.7.0+ story,
   filed as a follow-up to (1).

The narrow precursor @excessively-beneficial-owner stays open and
gets superseded by (2). Whoever picks up (2) is responsible for
closing the precursor with a `Source: <issue>` cross-reference.

---

## References

- `docs/design/web-edit-sync.md` — mutation protocol the run queue
  inherits the threat model from.
- `docs/design/body-sections.md` — `## Comments` is the canonical
  body section for free-form notes; the cross-reference on
  @excessively-beneficial-owner uses it.
- `.issuectl/prompts/implement.md` — already-shipped prompt template
  the `worktree` action kind renders against the
  `issuectl context <slug>` bundle.
- @excessively-beneficial-owner — narrow precursor; superseded in
  scope by this note, kept open for the follow-up implementation
  ticket.
- @profoundly-domineering-wound (closed `2026-05-08`) — landed
  the agent context bundle this design depends on.
- @markedly-terrific-angle — planned `.issuectl/AGENTS.md`; cited
  but not depended on by v0.6.0.
