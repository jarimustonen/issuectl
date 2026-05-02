---
created: 2026-04-26
updated: 2026-05-01
closed: 2026-05-01
type: task
reporter: jari
assignee: jari
status: done
priority: normal
labels: [infra, email]
---

# 14. info@grooveserve.com -sähköpostiosoite

_Source: kotisivut_

## Description

Kotisivujen footerissa (sekä `terms.tsx` ja `privacy.tsx` -sivuilla) on linkki `info@grooveserve.com` -osoitteeseen, mutta kyseistä tiliä ei ollut luotu Stalwart-palvelimelle. Viestit tähän osoitteeseen eivät menneet perille.

## Päätös

**Erillinen `info`-tili Stalwartiin** (ei alias jari@:lle, ei ulkoinen forward).

Perustelut:
- Pidetään yleinen yhteydenotto-osoite erillään henkilökohtaisesta inboxista — myöhemmin voidaan jakaa pääsy useammalle ihmiselle muuttamatta osoiterakennetta.
- Ulkoinen forward (esim. jari@itsellesi.fi) hylättiin: relay-loop-riskit, reverse-reply ei toimi out-of-the-box, ja vaatisi joka tapauksessa SMTP-lähetyskonfiguroinnin jos haluttaisiin vastata `info@`-osoitteena.
- Aluksi luetaan Roundcubella; ei ping-pong/AI-käsittelyä eikä `grooveserve-server`-binäärin IMAP-monitorointia.

**Reverse-reply ei vielä konfiguroitu.** Jos `info@`-osoitteena halutaan vastata, lisätään myöhemmin SMTP-tili per-address-SASL-mallin mukaisesti (`SMTP_INFO_USER` / `SMTP_INFO_PASSWORD` -namespace, ks. #56 decision log).

## Resolution

- Luotu Stalwart-tili `info` (`gsinfra mail create info`, accountId 7).
- Salasana synkattu SOPSiin (`operations/secrets/stalwart.enc.yaml` → `info_password`).
- Vastaanotto verifioitu lähettämällä SMTP-testiviesti palvelimen sisältä porttiin 25 ja seuraamalla Stalwart-loki:
  - `Message ingested ... accountId = 7, to = ["info@grooveserve.com"]`
  - `Delivery completed ... queueName = "local", to = ["info@grooveserve.com"]`
- `operations/AGENTS.md`: lisätty rivi "Osoitteet"-taulukkoon ja täsmennetty että vain botti-tilit (`healthcheck`, `assistant`) ovat IMAP-worker-monitoroinnin piirissä.

Ei pysyviä Ansible-konfiguraatiomuutoksia — Stalwart-tilien luonti tapahtuu API:n kautta (`gsinfra mail create`), ei roolin kautta.
