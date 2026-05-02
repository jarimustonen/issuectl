---
created: 2026-04-30
updated: 2026-04-30
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#38"]
labels: [ops, receipts, data-integrity, policy]
---

# 68. `save_receipt`-upsertin kenttäkohtainen yli-kirjoituspolitiikka

_Source: `crates/ops/src/receipts/save.rs`, `ON CONFLICT DO UPDATE`-haara_

## Description

`save_receipt`:n upsert-haarassa eri kentillä on tällä hetkellä **eri politiikat** sille, mitä tapahtuu kun retry tuo `None`/tyhjän arvon:

```sql
DO UPDATE SET
    extraction_id  = COALESCE(EXCLUDED.extraction_id, receipts.extraction_id),  -- COALESCE
    vendor         = EXCLUDED.vendor,                                            -- replace
    receipt_date   = EXCLUDED.receipt_date,                                      -- replace
    total_amount   = EXCLUDED.total_amount,                                      -- replace
    currency       = EXCLUDED.currency,                                          -- replace
    category       = EXCLUDED.category,                                          -- replace
    payment_method = EXCLUDED.payment_method,                                    -- replace
    raw_text       = COALESCE(EXCLUDED.raw_text, receipts.raw_text),             -- COALESCE
    items          = COALESCE(EXCLUDED.items, receipts.items),                   -- COALESCE
    confidence     = COALESCE(EXCLUDED.confidence, receipts.confidence),         -- COALESCE
```

Ongelma: jos käyttäjä on käsin asettanut `payment_method = 'card'`, ja agentin OCR-retry palauttaa `payment_method = None`, manuaalinen korjaus pyyhkiytyy NULLiksi. Sama pätee `receipt_date`:lle. Sääntö on sekoitus "korvaa aina"- ja "säilytä jos uusi on tyhjä" -käyttäytymistä ilman dokumentoitua syytä.

C2 (#38) toi receipt_revisions-historian, joten data on nyt palautettavissa — mutta varsinainen yli-kirjoitusongelma on edelleen olemassa ja epäkonsistentti.

## Goals

Ratkaisu vaatii oikean policy-keskustelun, ei pelkkää yksittäistä korjausta. Tämän issuen tarkoitus on **avata se keskustelu**, ei lyödä lukkoon yhtä lähestymistapaa.

Avoimia kysymyksiä:

1. **Onko sääntö per-kenttä-tasolla vai globaali?** Esim. "kaikki retryssä `None`-arvot säilyttävät vanhan" vs. "vendor/total/currency/category korvataan aina, muut COALESCE-suojataan".
2. **Pitäisikö käyttäjälähtöiset kentät erottaa agenttilähtöisistä?** Esim. flagi `manually_corrected_fields TEXT[]` tai metadatan keskittäminen, joka kertoo mitkä kentät on suojattu agentilta. Tämä vaatii receipts-skeemamuutoksen.
3. **Mitä tehdään ristiriidassa?** Skenaario: käyttäjä on asettanut `vendor='Acme Corp'`, agentti retryssä lukee `vendor='Acme'`. Pitääkö agentin (a) säilyttää käyttäjän arvo, (b) yli-kirjoittaa, (c) lipuksi ratkaisuun käyttäjälle? Tämä kytkeytyy tulevaan korjaus-UI:hin (#11).
4. **Versio-historian rooli ratkaisussa.** Nyt kun #38 säilyttää aiemmat tilat, voi olla että "yli-kirjoita reilusti, palauta tarvittaessa revisionhistoriasta" on hyvä strategia — pelkän sääntö-tason sijaan.
5. **Tri-state-syöte (`Patch<T>`):** monessa REST-API:ssa erotetaan "absent" / "explicitly null" / "set to value". Rust-puolella tämä on kolmiarvoinen enum. Onko tarpeen?

## Non-goals

- Pelkkä mekaaninen "lisää COALESCE puuttuvilla kentillä"-korjaus ilman policy-päätöstä. Inkonsistenssin lisääminen lisää-vain-yhdellä-kerralla-kerrallaan-tavalla on huonompi kuin nykytila.

## Background

Spin-off LLM-arviosta (`history/review-c2-receipt-revision-history.md`) C2-worktreen yhteydessä. Useat reviewerit (Gemini, Claude, DeepSeek) huomasivat epäjohdonmukaisuuden; GPT-5.5 huomautti että fix riippuu siitä, onko `save_receipt` "täysi-replace" vai "patch from retry". Nykyinen koodi on sekoitus — ei kumpaakaan puhtaasti.

Pre-existing käyttäytyminen (ei C2:n aiheuttama), mutta C2 teki ongelman näkyväksi ja palautettavaksi.

## Related

- #38 — receipt-revision-history (C2). Revisio-historia mahdollistaa palautuksen, mutta ei ratkaise yli-kirjoitusongelmaa itsessään.
- #11 — Korjaus-UI tositteille. Päätös tästä vaikuttaa siihen, mitä agenttilooppi saa yli-kirjoittaa.
- #28 — Monivaluutta. Lisää enemmän kenttiä joiden conflict-politiikka pitää ratkaista.
