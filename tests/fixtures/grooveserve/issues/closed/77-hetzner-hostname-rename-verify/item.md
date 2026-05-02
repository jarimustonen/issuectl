---
created: 2026-04-30
updated: 2026-04-30
closed: 2026-04-30
type: chore
reporter: jari
assignee: jari
status: done
priority: normal
related: ["#33", "#56"]
labels: [ops, infra]
---

## Resolution

Tehty 2026-04-30 B2-worktreessä:

- Hetzner Cloud Console server #127982431 nimetty `grooveserve-email` → `grooveserve-server` (`hcloud server update`).
- OS-hostname asetettu palvelimella (`hostnamectl set-hostname grooveserve-server`).
- Verifioitu: `hcloud server list` ja `ssh root@204.168.196.71 hostname` palauttavat `grooveserve-server`.

Komennot ajettiin Hetzner-API-tokenilla (`operations/secrets/hetzner.enc.yaml`) ja Ansible-SSH-avaimella (`~/.ssh/grooveserve-hetzner`). PTR `mail.grooveserve.com` on muuttumaton (Phase 11 -päätös).

# 77. Hetzner-host-rename `grooveserve-email` → `grooveserve-server` viimeistely

_Source: B2-local-dev-analysis worktree, 2026-04-30. Lähde: A4b Worktree-loki (`#56`)._

## Description

A4b:n cutover (`#56` Phase 11) yhdisti `services/api`:n ja `services/email`:n yhdeksi `crates/server` -binääriksi ja nimesi Ansible-puolen Hetzner-hostin uudelleen `grooveserve-email` → `grooveserve-server` (host_vars, mattermost.yml host-ryhmä, dokumentaatio). PTR `mail.grooveserve.com` jäi ennalleen.

A4b:n lokirivi:
> Hetzner-rename: ohjeet annettu Phase 10+11 -committin viestissä; käyttäjä ajaa `hostnamectl`-komennot palvelimella.

Tämä jäi avoimeksi tehtäväksi käyttäjän osalle.

## Scope

- Tarkista palvelimen nykyinen hostname (SSH `grooveserve-server` tai vanha `grooveserve-email`):
  ```bash
  ssh grooveserve-server hostnamectl
  ```
- Jos hostname on vielä `grooveserve-email`, aja:
  ```bash
  ssh grooveserve-server "hostnamectl set-hostname grooveserve-server"
  ```
- Tarkista Hetzner Cloud Console: VPS:n nimi vastaa `grooveserve-server`. Päivitä manuaalisesti consolesta jos eroaa.
- Varmista että SSH-konfiguraation alias toimii (`~/.ssh/config`).
- Varmista että `gsadmin status` ja `gsadmin logs` toimivat uudella hostnamella.

## Acceptance

- `ssh grooveserve-server hostnamectl` palauttaa hostname=`grooveserve-server`
- Hetzner Cloud Console näyttää VPS:n nimellä `grooveserve-server`
- `gsadmin` -komennot toimivat ennallaan (alias-läpinäkyvyys)
- Päivitetty `operations/servers/grooveserve-server.md` jos drift edelleen on jäljellä

## Out of scope

- DNS-muutokset (`mail.grooveserve.com` PTR pysyy ennallaan, Phase 11:n päätös)
- Mailgun/Stalwart -konfiguraatiomuutokset

## Why this is filed under #33

Tämä **ei ole local-dev-asia** suoranaisesti, mutta löytyi B2-worktreen aukko-inventaariossa. Jätetty avoimena ettei unohdu — file:nä erillisenä jotta tuotanto-ops-vuoro voi napata.
