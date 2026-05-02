---
created: 2026-04-26
updated: 2026-04-26
type: feature
reporter: jari
assignee: jari
status: in-progress
priority: normal
epic: 5
labels: [integraatio, kirjanpito]
---

# 19. Procountor-integraatio

_Source: matkalaskujen vienti_

## Description

Matkalaskujen vienti Procountor-kirjanpitojärjestelmään API:n kautta. Procountor on yksi Suomen suurimmista taloushallinto-ohjelmistoista ja tärkeä integraatiokohde asiakkaille.

Integraatio mahdollistaa matkalaskujen automaattisen siirron Grooveservesta Procountoriin, jolloin asiakkaan ei tarvitse syöttää tietoja manuaalisesti kirjanpitoon.

## Scope

- OAuth 2.0 M2M -autentikointi
- Matkalaskujen (TRAVEL_INVOICE) luonti Procountor API:n kautta
- Kuittiliitteiden siirto
- Hyväksyntäkierron tuki

## Progress

- **2026-04-26:** PTS-testiympäristöhakemus jätetty dev.procountor.com:ssa. Odotetaan Procountorin vastausta (tunnusten provisiointi).

## Next Steps

- Procountorilta PTS-tunnukset → aloita M2M OAuth -toteutus
- Katso `analysis.md` MVP-vaiheistus

## Related

- Netvisor-integraatio (toinen kirjanpitojärjestelmä, ei vielä issueta)
