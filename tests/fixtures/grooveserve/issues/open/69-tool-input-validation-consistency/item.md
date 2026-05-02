---
created: 2026-04-30
updated: 2026-04-30
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
labels: [tools, validation, dx]
---

# 69. Tool-wrapperien syöte-validointi yhtenäiseksi

_Source: `crates/server/src/tools/`-puun wrapper-kerros_

## Description

LLM-toolien Rust-wrapperit eivät validoi syötteitä yhtenäisesti ennen ops-kutsua. Tämä tuottaa kahta ongelmaa:

1. **Huonoja virheviestejä LLM:lle.** Validoimaton arvo putoaa DB CHECK -virheeksi tai sqlx-tyyppivirheeksi. LLM saa siistin "Database error" -merkkijonon mutta ei diagnostiikkaa siitä mikä syöte oli väärin.
2. **Epäjohdonmukaisuus toolien välillä.** Esim. `save_receipt`-wrapper validoi `currency`/`payment_method`/`category`, mutta `update_receipt`-wrapper hyväksyy samat kentät ilman muotoa- tai enum-tarkistusta.

Esimerkkejä konkreettisista aukoista (löydetty C2:n LLM-arviossa, eivät kattava lista):

- `crates/server/src/tools/receipts/update_receipt.rs`:
  - `currency` ei pre-validoida 3-kirjaimiseksi ISO 4217 -muotoon (CHECK rajoittaa `'^[A-Z]{3}$'`).
  - `payment_method` ei pre-validoida enumiksi `(card|cash|invoice)` vaikka schema-deklaraatio mainitsee enumin.
- Yleisempi kysymys: kuinka montaa muuta toolia tämä koskee?

## Goals

- Käy läpi **kaikki** `crates/server/src/tools/`-wrapperit ja tunnista missä syöte-validoinnin tulisi tapahtua wrapper-kerroksessa ennen ops-kutsua.
- Ratkaisu: yhteinen util-moduuli (`crates/server/src/tools/util.rs` on jo osittain käytössä) johon yhtenäiset tarkastimet (currency-shape, ISO-päiväys, enum-tarkistus, decimal-range, jne.) keskitetään.
- Kirjaa konventio AGENTS.md:ään: "wrapper validoi muodon/enumin; ops validoi DB-sidotut invariantit (FK-omistajuus, idempotency-key)".
- Kun virhe paljastuu DB-tasolla, käännä se ops-puolella ihmisluettavaksi ennen palautusta `OpError::InvalidInput`-muodossa, jotta LLM saa hyödyllisen viestin.

## Non-goals

- **Ei** siirretä validointia ops-kerrokseen. ops-puoli on tarkoituksellisesti DB-only; muoto-/enum-tarkistus kuuluu kanavakerrokseen (samoin kuin CSRF, rate-limit). HTTP-puoli ja agent-tool-puoli voivat jakaa util-funktioita mutta eivät ops:ia.
- **Ei** poisteta DB CHECK -rajoituksia. Niiden tarkoitus on belt-and-braces — wrapper validoi siisteyden vuoksi, CHECK suojaa silti suorat SQL-INSERTit.

## Background

Spin-off LLM-arviosta (`history/review-c2-receipt-revision-history.md`) C2-worktreen yhteydessä. Pre-existing aukko, ei C2:n aiheuttama, mutta löytyi C2:n review-kierroksen sivutuotteena. GPT-5.5 ja Claude raportoivat löydökset.

## Related

- #38 — Receipt revision-history (C2). Spin-offin lähde.
- Yleisempi tool-konventio: `crates/server/AGENTS.md`, AGENTS-AI-FIRST-CLI.md.
