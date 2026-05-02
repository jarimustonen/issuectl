---
created: 2026-04-26
updated: 2026-04-26
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#2"]
labels: [ai, opex]
---

# 14. Opex-hallinta: rate limiting, token-budjetti ja kustannuskatto

_Source: email-service AI agent_

## Description

Rate limiting per lähettäjä, globaali token-budjetti ja kustannuskatto Anthropic API -kutsuille. Matkalaskupalvelussa per-lasku-hinnoittelu tekee tästä vähemmän kiireellisen, mutta tarvitaan kun järjestelmä kypsyy.

## Scope

- [ ] Per-sender rate limit (esim. 10 viestiä/tunti)
- [ ] Global concurrency semaphore Anthropic-kutsuille
- [ ] Daily cost ceiling (disablointi + fallback-viesti)
- [ ] Token/character budget per viesti ja historia
- [ ] Quoted email reply stripping (estää token-kasvun)
- [ ] Metriikat kustannusseurantaan
- [ ] Per-tenant token-budjetit (päivä/kuukausi)
- [ ] Tool-kutsujen metriikat (kutsumäärä, latenssi, virhetaso per työkalu)
- [ ] Kustannus-per-kuitti ja kustannus-per-matka -seuranta
- [ ] Vision-kuvien token-kustannusten seuranta
- [ ] Max liitteet/sähköposti ja max sivut/PDF -rajat
- [ ] Max tool-kutsut per sähköposti
- [ ] Alertit kun retry-määrät piikkaavat
