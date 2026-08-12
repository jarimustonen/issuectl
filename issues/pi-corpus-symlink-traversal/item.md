---
created: 2026-08-12
updated: 2026-08-12
type: bug
status: fixed
priority: high
related: ['@pi-manifest-locking']
closed: 2026-08-12
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

## Resolution

### 2026-08-12T17:03:11Z · @issuectl

Fixed in 62bd733 + e2fa3b9. Every pi-corpus path that walks, deletes, or overwrites under pi_root now refuses to follow a directory/SKILL.md symlink out of the corpus, checked via symlink_metadata (which never follows the final component):

- WALK (classify_pi_corpus / pi-status): a symlinked <pi_root>/<name> is reported Unmanaged without reading through it — no out-of-corpus read/leak, never prune-eligible.
- DELETE (orphan_is_safely_removable → pi_prune): refuses a symlinked/non-dir entry (checked first), fail-closed on stat errors and unreadable siblings, and handles remove_file/remove_dir failures without clearing the row.
- WRITE (ensure_pi_mirror_target_within_corpus → install_pi_mirror): refuses a symlinked entry dir or non-regular SKILL.md, validates the skill name at the fs boundary; save_pi_manifest temp write uses O_EXCL so a pre-planted symlink is refused.

7 hermetic regression tests (unix tempdirs, never real ~/.pi). Reviewed via /llm-review (4 models, consensus) — history/review-pi-corpus-symlink-traversal.md.

Scoped OUT (spin-off pi-corpus-fd-relative-hardening): the fix closes the persistent-symlink threat + cooperative advisory flock, but NOT a hostile same-UID process racing TOCTOU between check and syscall, nor --force hard-link overwrite. Full closure needs descriptor-relative no-follow ops (openat2 RESOLVE_BENEATH/RESOLVE_NO_SYMLINKS, unlinkat, atomic renameat) — a dependency-adding architectural change. Boundary documented inline.
