---
created: 2026-08-12
updated: 2026-08-12
type: improvement
status: open
priority: normal
related: ['@pi-corpus-symlink-traversal']
---

# pi-corpus: harden mutating paths with descriptor-relative no-follow ops (close TOCTOU + hard-link overwrite)

## Description

Spin-off from the /llm-review of `pi-corpus-symlink-traversal` (all 4 models —
gemini/openai/anthropic/deepseek — reached consensus; see
`history/review-pi-corpus-symlink-traversal.md`). That fix closes the
**persistent-symlink** threat (a symlink planted before issuectl runs, plus the
cooperative advisory flock) via `symlink_metadata`-then-act gates in
`classify_pi_corpus`, `orphan_is_safely_removable`, and
`ensure_pi_mirror_target_within_corpus`. It deliberately does NOT close the
harder residuals, which need an architectural change (a new dependency:
`rustix` or `cap-std`).

Residuals to close here:

1. **TOCTOU (check-then-use race).** `symlink_metadata` (check) and
   `remove_file`/`create_dir_all`/`std::fs::write` (use) are separate syscalls.
   A hostile *same-UID* process that does not respect the flock can swap a real
   `<pi_root>/<name>` dir for a symlink in the window between check and
   destructive op and still escape the corpus. Fix: open `pi_root` once as a
   dirfd and operate relative to it with `openat2(RESOLVE_BENEATH |
   RESOLVE_NO_SYMLINKS)` (Linux 5.6+), falling back to
   `openat`/`mkdirat`/`unlinkat`/`renameat` with `O_NOFOLLOW`.

2. **Hard-link overwrite via `--force`** (OpenAI). A regular-file `SKILL.md`
   that is a hard link to an external file is truncated in place by
   `std::fs::write` — `is_symlink()` is false, so the current guard allows it.
   Fix: never overwrite an inode in place — write a fresh temp file in the
   verified entry dir and atomically `renameat` it over `SKILL.md`.

3. **Root/ancestor symlink** (DeepSeek). `symlink_metadata` follows symlinks in
   `pi_root`'s ancestry; if `~/.pi/agent/skills` itself is a symlink, all ops
   resolve elsewhere. Consider canonicalizing + fencing the root once, or
   anchoring on a `pi_root` dirfd opened with nofollow. (Debatable — `pi_root`
   is trusted input resolved from `HOME`.)

4. **`classify_pi_corpus`: `present = meta.is_ok()`** collapses *any* stat error
   (permission/IO) to "absent" → `Missing` → clears the provenance row. Distinguish
   a genuine `NotFound` from an inaccessible entry (e.g. a `Presence` enum) so a
   transient error never drops a row.

Threat-model boundary and the pointers above are documented inline at the
pi.dev-corpus-lifecycle section header in `crates/issuectl-core/src/skill.rs`.
Not a blocker for the shipped fix; scope this as its own change because it adds a
dependency and rewrites the write/delete primitives.

