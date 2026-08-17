# pi.dev skill mirror: dual-home install, provenance, drift, prune

How the binary-shipped skills are mirrored into pi.dev's global skill corpus,
and how that corpus is kept observable. Implementation: `skill.rs`
(`install_skill_summary`, `record_pi_provenance`); origin issue
`pidev-pi-skill-lifecycle`.

## Dual-home install (`~/.pi/agent/skills/`)

Whenever the Claude layout is installed (`skill install`, `--force`, `--agent
all`, or `issuectl init`), each Claude `SKILL.md` is **also** written to
pi.dev's global skill corpus at `~/.pi/agent/skills/<name>/SKILL.md` (`issue`,
`issue-new`, `issue-intake`) so the skills are discoverable under the pi.dev
harness (invoked there as `/skill:<name>`; bare `/name` cross-references
resolve via pi's injected available-skills list, so **only the install target
differs — no body/link rewrite**). The mirror is byte-identical to the
repo-local Claude copy.

- **Vendored filter: only `SKILL.md` is mirrored** — the Codex prompts and the
  `issues/AGENTS.md` scaffold are not; a `--agent codex` install writes no pi
  copy.
- **Asymmetry:** the Claude/Codex targets are **repo-local** (rooted at the
  target repo), while the pi mirror is **home-global** (rooted at `$HOME`,
  resolved via `skill::pi_skills_root`; skipped when `$HOME` is unset).
- Each pi copy is written independently, so it never gates the repo-local
  install — a present pi copy still lets a plain install repair a deleted
  Claude skill (a non-force run leaves the pi copy in place; `--force`
  refreshes it).

## Corpus lifecycle (provenance · drift · prune)

The global pi copies are otherwise unmanaged, so `skill.rs` adds an
observability layer on top of the mirror:

- **Provenance manifest.** Every pi mirror pass also writes
  `~/.pi/agent/skills/.issuectl-manifest.json` (`PI_MANIFEST_FILE`) — a
  tool-namespaced JSON map of `skill name → { version }` recording which
  corpus entries issuectl wrote and at which version. This is out-of-band: the
  `SKILL.md` bodies stay byte-identical to the Claude copies, so provenance
  lives in the manifest, not a marker inside the skill. Other tools keep their
  **own** manifests (e.g. `.orchestratectl-manifest.json`); neither tool
  prunes the other's entries. **Provenance follows real write events, never
  on-disk existence:** `record_pi_provenance` records only the skills a run
  actually created/overwrote (threaded via a `written` set from the mirror
  loop), stamped with the running version. A managed-name file that already
  existed and was left in place — a non-force install, or a hand-authored
  copy — is **not** adopted, so `pi-prune` can never later delete a file
  issuectl did not write. The manifest is written atomically (temp + rename);
  manifest keys are validated to safe single path components on load
  (`is_valid_skill_name`) so a tampered key can't steer a delete outside the
  corpus, and the strict loader refuses a corrupt/foreign/unsupported manifest
  rather than acting on an empty misread of it.
- **`issuectl skill pi-status`** (read-only) classifies every corpus entry:
  `up-to-date` · `stale` (issuectl-owned, on-disk differs, recorded version ≠
  running — a different binary wrote it) · `modified` (differs but recorded
  version == running — hand-edited/corrupted) · `missing` (manifest row, file
  gone) · `orphan` (manifest row for a skill issuectl no longer ships) ·
  `unmanaged` (on disk, not in the manifest — hand-authored or another
  tool's). Supports `--json`.
- **`issuectl skill pi-prune`** removes `orphan` entries (deletes the mirrored
  `SKILL.md`, drops the dir only if now empty, clears the manifest row) and
  clears `missing` rows. **Dry-run by default; `--force` applies.** It never
  touches `unmanaged` entries and never deletes a *current* skill — a `stale`
  or `modified` copy is refreshed via `skill install --force`, not pruned.
  Deletion is guarded: only a regular-file `SKILL.md` in a dir holding nothing
  else is removed; a symlinked or sibling-laden orphan is reported under
  `skipped` and left for the user. Prune refuses to run at all on a
  corrupt/untrusted manifest.

## Reconciliation policy: always-on-force (a decision, not an accident)

The write path deliberately mirrors the repo-local targets — a non-force
install leaves an existing pi copy alone; `--force` overwrites it
unconditionally to the running binary's version (**not**
overwrite-only-if-newer). This matches the repo-local Claude/Codex targets
exactly (force means force) and avoids both a surprising "your `--force` did
nothing" outcome and brittle version-ordering at write time. The known cost —
an *older* binary's `--force` can rewrite the global copy to an older
version — is handled by making drift **visible** (`pi-status` flags a
recorded-version mismatch) and **reversible** (re-run `skill install --force`
from the newest binary, or `pi-prune` for orphans), not by guarding the write.
Chosen because the pi corpus is a derived convenience whose ground truth is
any repo's current binary; a write-time newer-only guard would add a second,
subtly-different overwrite rule for one of the several install targets and
still couldn't order dev builds reliably.

## Known gap

There is no `skill uninstall`, and it is an open question what one should do
with the shared global pi copy that can't be reference-counted across repos.
Documented follow-up, out of scope for this lifecycle layer.
