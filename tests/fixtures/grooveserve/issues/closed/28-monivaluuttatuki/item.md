---
created: 2026-04-26
updated: 2026-04-30
closed: 2026-04-30
type: feature
reporter: jari
assignee: jari
status: done
priority: normal
epic: 56
related: ["#26", "#38"]
labels: [accounting]
commits:
  - hash: TBD
    summary: "feat(ops/migrations): add currency fields and exchange_rates_cache (#28)"
  - hash: TBD
    summary: "feat(ops): ExchangeRateFetcher trait + cache-first get_rate orchestrator (#28)"
  - hash: TBD
    summary: "feat(ops/receipts): currency fields on save/update + revision snapshot (#28)"
  - hash: TBD
    summary: "feat(server/tools): save_receipt accepts currency fields, ECB rate fallback (#28)"
  - hash: TBD
    summary: "test(ops/receipts): currency-related integration tests (#28)"
  - hash: TBD
    summary: "docs(ops,server): document exchange_rates module + currency block (#28)"
---

# 28. Monivaluuttatuki

_Source: matkalaskujen käsittely_

## Description

Lisää tuki useille valuutoille matkalaskuissa. Kansainväliset työmatkat tuottavat kuitteja eri valuutoissa (USD, SEK, GBP, jne.). Järjestelmän pitää tallentaa alkuperäinen valuutta ja summa sekä EUR-muunnos.

## Scope

- `expenses` ja `receipts` -tauluihin: `original_currency`, `original_amount`, `exchange_rate`, `exchange_rate_date`
- Vaihtokurssin lähde (ECB, manuaalinen, kortin kurssi)
- Miten agentti käsittelee ulkomaanvaluuttakuitteja
- Miten matkalaskun kokonaissumma lasketaan sekavalyytoilla

## Toteutus (C3, 2026-04-30)

**Schema** (`migration 020_currency_fields.sql`): nelikenttäinen lohko
(`original_currency` CHAR(3), `original_amount` NUMERIC(12,2),
`exchange_rate` NUMERIC(12,6), `exchange_rate_date` DATE) lisättiin
`receipts`-, `expenses`- ja `receipt_revisions`-tauluihin. Atomic
CHECK (kaikki neljä NULL tai kaikki NOT NULL) + EUR-koherenssi-CHECK
(EUR-tapauksessa `exchange_rate=1.0` ja `original_amount IS NOT
DISTINCT FROM total_amount`) + ISO 4217 -regex + `rate > 0`.
Identtiset CHECKit `receipt_revisions`-taulussa estävät historiarivien
vajaat tilat. Erillinen `exchange_rates_cache`-taulu (PK
`(rate_date, currency)`) cachettaa ECB-päiväkurssit.

**Ops-kerros** (`crates/ops/src/exchange_rates.rs`): `ExchangeRateFetcher`-
trait + `get_rate(db, fetcher, currency, date)` cache-first
orkestrointi. EUR returns `1.0`; muutoin cache-haku
"latest <= date for currency", miss → fetcher-kutsu → upsert kaikki
`RateEntry`-rivit ja toinen cache-haku. `crates/ops` pysyi
HTTP-vapaana — ECB-fetch (`EcbFetcher`) elää
`crates/server/src/exchange_rates.rs`:ssä reqwest+XML-parserilla.

**Validointi** (`crates/ops/src/receipts/currency.rs`):
`validate_currency_block` peilaa schema-CHECKejä rakenteellisena
`OpError::InvalidInput`-virheenä, jotta agentti saa luettavan virheen
`pg.check_violation`-bubblauksen sijaan. Non-EUR:lle myös cross-check
`original_amount * exchange_rate ≈ total_amount` (toleranssi 0.01 EUR
sub-cent-pyöristykselle).

**Server-tools** (`save_receipt`/`update_receipt`): JSON-skeemoihin
neljä uutta kenttää, `resolve_currency_block`-helper hoitaa
EUR-defaultin, manuaalisen kurssin ja ECB-fallbackin. Boot wiraa
`set_global_fetcher(Arc::new(EcbFetcher::new()))`-singletonin
(MVP-shortcut, dokumentoitu `crates/server/AGENTS.md` "Exchange rate
plumbing" -osiossa).

**Revision-snapshot**: `lock_and_record_revision_tx` kantaa myös uudet
neljä kenttää, joten valuuttamuutos kuitilla säilyttää aiemman
valuutta-tilan `receipt_revisions`-taulussa.

**Testit**: 10 uutta sqlx-integraatiotestiä `receipts/tests.rs`:ssä +
9 unit-testiä `currency.rs`:ssä + 7 sqlx-testiä `exchange_rates.rs`:ssä
mock-fetcherillä + 5 ECB XML -parserin yksikkötestiä server-puolella
(ei verkkokutsuja CI:ssä). 131 ops + 277 server lib-testiä vihreänä;
pre-existing `claim_with_thread.rs` 11-failure dokumentoitu C2:n
worktree-lokissa, ei C3:n aiheuttamaa.

**Matkalasku-kokonaissumma**: SUM(`receipts.total_amount`)
EUR-arvoina, jokainen rivi muunnettuna jo tallennusvaiheessa.
Erillinen aggregointinäkymä on Phase 2/3 -työtä.

## Out of scope

- Matkalasku-näkymä (`expense_reports`-aggregointi) — Phase 2/3
- Kalenteri-integraatio kurssipäivän tunnistamiseen — myöhempi vaihe
- Kortin kurssin automaattinen tunnistaminen kuitin OCR:stä — #15
- Useat kurssilähteet samaan kuittiin — yksi lähde riittää MVP:ssä
