---
created: 2026-04-27
updated: 2026-04-30
closed: 2026-04-30
type: feature
reporter: jari
assignee: jari
status: done
priority: high
epic: 56
labels: [devex, infra]
---

# 33. Local development environment

_Source: monorepo root_

## Description

Currently every change to the API service, email service, or AI agent has to be deployed to production for testing. This is slow and risky. The goal is the simplest practical way to run the full stack (or meaningful subsets of it) locally on macOS Apple Silicon so that Jari can iterate quickly on:

- Registration / email verification flow
- AI agent and tool loop
- IMAP/SMTP plumbing
- Database schema changes

## Scope

Produce an analysis document that:

1. Inventories what each service needs to run locally
2. Compares options for local infra (Postgres, mail server, frontend)
3. Recommends a pragmatic approach with concrete next steps
4. Sketches the files needed (compose, env templates, scripts)

Constraints:
- macOS Apple Silicon, Podman (not Docker)
- Real `ANTHROPIC_API_KEY` is available locally
- Some prod deps (Stalwart cluster, Mailgun) won't be reproduced locally

## Resolution (2026-04-30, B2-local-dev-analysis worktree)

Alkuperäinen scope ("analysis + concrete next steps") on suoritettu:

- **Pre-A4 `analysis.md` (2026-04-27)** dokumentoi Hybrid-arkkitehtuurin (Postgres jaettuna kontissa, sovellukset host-prosesseina, Mailpit per-instance, opt-in GreenMail/Roundcube). Päätökset (§7) lukittu.
- **Phase 1 -refaktori (`#56`)** toteutti suurimman osan suosituksista A2/A3/A4a/A4b/A4c/B1-worktreissä:
  - Yhdistetty `grooveserve-server`-binääri (yksi HTTP+IMAP -prosessi)
  - Unified DB per-instance (`grooveserve_dev_<safe_id>`)
  - gsdev-CLI (`tools/dev/gsdev/`) ja `.workmux.yaml`-paneelit
  - Per-account SMTP-konfiguraatio (`SMTP_<NAME>_USER/_PASSWORD`)
  - gsadmin lokaalimoodi (`#50`)
- **Päivitetty `analysis.md` (2026-04-30, B2-worktree)** inventoi nykytilan, listaa post-A4-aukot, ja määrittelee jatkoissueet.

### Jatkotyöt (alaissueina)

- **#74** `gsdev mail send-eml` ja `mail history` post-A4 (rikki A4b:ssä, korjaus odottaa D-aaltoa)
- **#75** gsdev rebuild policy (mtime-pohjainen cache-rebuild)
- **#76** `crates/server/tests/claim_with_thread.rs` 11 testiä punaisina (pre-existing fixture-bug)
- **#77** Hetzner-host-rename `hostnamectl`-ajo (ops-tehtävä)

#13 healthcheck-monitori jatkaa pre-existing-issuena Phase 0:ssa (ei alaissue).

## Deliverables

- [x] `analysis.md` — pre-A4 (2026-04-27) ja post-A4 (2026-04-30) versiot, sis. inventaariot, vaihtoehtojen vertailu, suositukset
- [x] Follow-up issues (#74–#77) jatkotyölle

## Related

- `#56` Toimiva testattava perusta (epic, koordinointi)
- `crates/server/`, `crates/ops/`, `crates/dev-cli/` — Phase 1 -refaktorin tulokset
- `tools/dev/AGENTS.md` — gsdev-CLI:n käyttöohje
- `operations/dev/` — compose + env-templatet
