---
created: 2026-08-12
updated: 2026-08-12
type: bug
status: open
priority: high
related: ['@pi-manifest-locking']
---

# pi-corpus: prune/install follow directory symlinks — delete/overwrite can escape the corpus

_Source: crates/issuectl-core/src/skill.rs_

## Description

Spin-off from /llm-review of pi-manifest-locking (see history/review-pi-manifest-locking.md; OpenAI #1/#2).

`is_valid_skill_name` validates only the manifest KEY string (rejects `..`, separators, dotfiles). It does NOT establish that `<pi_root>/<name>` is a real directory beneath the corpus. If an issuectl-owned entry dir is replaced by a symlink (e.g. `~/.pi/agent/skills/triage-bugs -> /external/dir`):

- `pi_prune`: `pi_root.join(name).join("SKILL.md").symlink_metadata()` and `read_dir(dir)` follow the intermediate dir symlink; `remove_file` can then delete `/external/dir/SKILL.md`.
- `install --force` via `install_rendered_file`: `path.exists()` + `std::fs::write` follow a symlinked `issue/SKILL.md -> ~/.ssh/config` and overwrite the target.

The "never delete/write outside the corpus" doc guarantee is therefore false against symlink traversal in intermediate components. The new advisory lock does not help (it does not stop a user/sibling tool replacing the dir).

Fix direction: reject any `<pi_root>/<name>` whose own `symlink_metadata()` is not a real directory before touching `SKILL.md`; prefer descriptor-relative (openat + O_NOFOLLOW / AT_SYMLINK_NOFOLLOW) inspect+delete to close the TOCTOU window. Pre-existing; out of scope for the locking change.
