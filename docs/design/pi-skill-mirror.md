# pi skill installation and legacy global-corpus lifecycle

Implementation: `crates/issuectl-core/src/skill.rs`.

## First-class native pi target

`issuectl skill install --agent pi` writes each selected skill to
`.pi/agent/skills/<name>/SKILL.md` beneath the resolved install base. Omitting
`--agent`, or passing `--agent all`, installs Claude, pi, and Codex together.
`--target <dir>` replaces the install base without changing those layouts.

Claude and pi receive byte-identical native Agent Skills. Codex receives the
self-contained prompt variant at `.codex/prompts/<name>.md`. Existing files are
preserved unless `--force` is explicit, and `--dry-run` performs collision reads
and reports the plan without creating directories or files.

This target-local behavior replaced the old implicit dual-home side effect:
`skill install` and `init` no longer write into `$HOME/.pi` merely because a
Claude skill was selected. That makes runtime selection inspectable, gives pi
symmetry with Claude and Codex, and keeps sandboxed installs hermetic.

## Legacy global corpus lifecycle

Older issuectl releases mirrored Claude skills into the home-global corpus at
`~/.pi/agent/skills/` and recorded ownership in
`.issuectl-manifest.json`. Existing installations remain observable and safely
removable:

- `issuectl skill pi-status` classifies issuectl-owned entries as up-to-date,
  stale, modified, missing, or orphaned, and reports unrelated entries as
  unmanaged.
- `issuectl skill pi-prune` is dry-run by default; `--force` removes safe
  orphaned entries and stale manifest rows. It never touches unmanaged entries.
- Manifest writes are atomic and lock-serialized. Names and filesystem targets
  are validated so cleanup cannot escape the corpus.

These lifecycle commands are retained for migration and cleanup. New installs
use the explicit native pi target and do not add global-manifest entries.
