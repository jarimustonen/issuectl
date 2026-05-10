---
created: 2026-05-09
updated: 2026-05-10
type: improvement
reporter: jari
status: in-progress
priority: normal
epic: hugely-exciting-spiders
related: ['@deeply-wistful-beam']
labels: [release-v0.6.0, backlog]
---

# Replace thread-local schema/transitions cache activation with explicit injection

## Description

Spin-off from /llm-review of @deeply-wistful-beam (server config cache).

The server-mode schema + transitions cache is currently activated via a
thread-local guard installed inside `tokio::task::spawn_blocking` closures
in `src/server/api.rs`. `schema::load` / `transitions::load` consult the
thread-local and route through the cache when active. CLI never installs
one, so behaviour is unchanged there.

This works correctly today — `Drop`-based guard handles panic, no async
boundary is crossed — but the design has known fragility:

- Behaviour of `mutate::*` depends on hidden ambient state, not the type
  signature. Future server routes can forget the `enter()` call and
  silently get the slow path.
- `ActiveGuard` is accidentally `Send`; future async refactors that hold
  it across `.await` would corrupt the slot via tokio worker reuse.
- `repo_config` lives at crate root (`src/main.rs`) so the CLI binary
  carries a server-only type. Moving it under `src/server/` requires
  removing the thread-local first.
- Tests share thread-local state across the cargo test pool; a forgotten
  drop or `mem::forget` would pollute the next test on the same thread.

## Goal

Replace thread-local activation with explicit dependency injection. The
working shape proposed in the review is something like:

```rust
pub trait ConfigSource {
    fn schema(&self, root: &Path) -> Result<Arc<Schema>>;
    fn transition_rules(&self, root: &Path) -> Result<Arc<TransitionRules>>;
}
```

with `UncachedConfig` for the CLI and `RepoConfigCache` impl-ing the
trait for the server. Mutate APIs grow a `&dyn ConfigSource` parameter
and the `repo_config` thread-local + `enter()` / `current()` machinery
goes away entirely.

## Out of scope

- The mtime-vs-content-hash invalidation question. That stays an
  independent decision; this issue is about the activation mechanism,
  not the cache key.

## Open question for prioritisation

How many new mutate entry points or new frontends (TUI, LSP, alt server)
will land in the next 3-6 months? If one or more, this refactor pays
itself back. If none, the current thread-local + `!Send` guard is fine
and this issue can sleep until it actually bites.

## Cost

Touches the four `mutate::*` public functions
(`update_issue`, `new_issue`, `update_body`, `close_issue`), plus their
~50 test callsites in `src/mutate/mod.rs`. CLI callers in `src/main.rs`,
`src/doctor.rs`, `src/context.rs` need an `UncachedConfig` instance threaded
through (or a default impl). Likely conflicts with sister-worktree
changes that touch `mutate.rs`, so worth coordinating with whatever
lands in v0.5.0 first.
