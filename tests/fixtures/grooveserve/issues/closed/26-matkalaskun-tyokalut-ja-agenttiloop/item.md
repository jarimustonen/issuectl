---
created: 2026-04-26
updated: 2026-04-26
type: feature
reporter: jari
assignee: jari
status: open
priority: high
epic: 5
related: ["#7", "#8", "#9", "#10", "#15", "#20"]
labels: [ai, design, core]
---

# 26. Matkalaskun käsittelyputki — työkalut, tietomalli ja agenttiloop

_Source: AI-agentti, matkalaskujen käsittely_

## Description

Suunnittele matkalaskujen käsittelyn ydinputki: työkalut, tietomalli ja agenttinen silmukka, joilla AI-agentti muuttaa kuitit, matkatiedot ja käyttäjäkontekstin valmiiksi matkalaskuluonnokseksi.

Tämä on suunnitteludokumentti (design.md), ei toteutus. Kattaa:

1. **Liitteiden käsittely** — sähköpostiliitteiden vastaanotto, tallennus, OCR
2. **Raakadata → kirjanpitodata** — kuittien tulkinta, ALV, kulukategoriat
3. **Matkan tunnistaminen** — useiden kuittien yhdistäminen matkaksi
4. **Käyttäjäprofiili** — taustadata, progressiivinen tiedonkeruu
5. **Geolokalisaatio ja etäisyydet** — osoitteiden geokoodaus, km-laskenta
6. **Matkalaskuluonnos** — tietomalli, tilat, muokkaus
7. **Agentin työkalumäärittelyt** — Claude tool_use -formaatti, agenttinen silmukka

## Design Principles

- **Email-First UX**: Sähköposti on ensisijainen käyttöliittymä. Kaikki mitä voi tehdä webissä, pitää voida tehdä myös sähköpostilla.
- **Unified Tool Surface**: Agentin työkalut ja web-UI käyttävät samoja backend-operaatioita.
- **MVP-pragmatismi**: Suunnitelma priorisoi mitä rakentaa ensin vs. myöhemmin.

## Deliverables

- [x] `design.md` — kattava suunnitteludokumentti
- [ ] Toteutus (erillinen issue)
