---
created: 2026-04-30
updated: 2026-04-30
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#28"]
labels: [accounting, schema]
---

# 71. Kuitti vs. laskutustapahtuma ja FX-kulujen käsittely

_Source: C3 (#28) post-implementation /llm-review, käyttäjän selvennys 2026-04-30_

## Description

C3:n monivaluuttatuki (#28) jätti kaksi avointa kysymystä jotka muodostavat saman päätösperheen ja siirretään tähän erilliseen issueen myöhempää suunnittelua varten.

### Kysymys 1 — `total_amount`:n semantiikka

C3:n nykyinen toteutus käsittelee `receipts.total_amount`:in **EUR-konvertoituna kokonaissummana** (skill-prompti `save_receipt.skill.md` sanoo "Total amount in EUR"; `validate_currency_block` cross-checkkaa `original_amount × exchange_rate ≈ total_amount`). Käyttäjän selvennys 2026-04-30: oikea malli on:

- **Kuitti (`receipts`-rivi)** — `total_amount` + `currency` ovat kuitin omassa valuutassa (USD 100 USD-kuitilta, EUR 50 euroopassa lounaalta).
- **Laskutustapahtuma** — käyttäjän kortilla / pankkitilillä veloitettu summa omassa valuutassa (meidän tapauksessa EUR). Tämä on erillinen tapahtuma kuitista: sama USD-100-kuitti voi tuottaa eri EUR-veloituksen riippuen kortin FX-kulusta ja vaihtokurssista.

Eli `total_amount` ei pitäisi olla EUR-canonical vaan kuitin oman valuutan summa. EUR-veloitus on erillinen käsite jota ei vielä mallinneta.

### Kysymys 2 — FX-kulut ja cross-check

C3:n `validate_currency_block` enforce:aa
`|total_amount − original_amount × exchange_rate| ≤ 0.01 EUR` non-EUR-kuiteilla. Tämä toimii ECB-kurssilla syntetisoidulla EUR:lla mutta hylkää reaalimaailman kortti-veloituksia: kortti veloitti EUR 93.73 USD-100-kuitilta (3 % FX-kulu) vaikka ECB-kurssi 0.91 → EUR 91. Toleranssi on ratkaisu sub-cent-pyöristykselle eikä FX-kuluille.

## Päätökset jotka tehdään tässä issuessa

1. **`total_amount`:n semantiikka**: pidetäänkö nykyinen "EUR-canonical" -malli vai siirrytäänkö "receipt-currency" -malliin? Jälkimmäinen vaatii:
   - `validate_currency_block`:n cross-checkin uudelleenarvioinnin (mitä `total_amount` cross-checkaa?)
   - `SaveReceiptInput`/`UpdateReceiptInput` -semantiikan muutoksen
   - skill-promptin uudistuksen
   - aggregointi-näkymä Phase 2/3:lle (matkalasku-summat eri valuutoissa: muunnos query-tasolla?)

2. **Laskutustapahtumien tallennus**: oma `billing_events`-taulu, vai laajennetaanko `receipts`-rivin nelikenttäistä lohkoa kanavoimaan laskutustapahtumaa? Käyttäjän päätös 2026-04-30: **ei tehdä billing_events-taulua vielä**, selviämme ilman sellaista MVP:ssä.

3. **FX-kulujen käsittely**: kun `total_amount` on receipt-currency, FX-kulut ovat osa erillistä laskutustapahtumaa. Cross-check non-EUR-kuiteilla muuttuu sisäiseksi (kuitin oman valuutan summan ja `original_amount`:n ekvivalenssi → tarpeeton) tai poistuu kokonaan.

## Reuna-ehdot

- Migraatio 020 on jo mainissa kun tämä toteutetaan; muutokset vaativat oman migraation tai backfillin.
- Phase 2/3 (web-näkymä, matkalasku-aggregointi) tarvitsee ratkaisun ennen kuin se voi näyttää sekavalyytta-kuluja oikein.
- Tämä koskee myös `expenses`-tauluja samalla logiikalla — koordinoidaan #73:n kanssa.

## Out of scope (tässä issuessa)

- Kortti-integraatio (oikea kortin EUR-summan automaattinen tuonti)
- Pankkitili-integraatio
- Useat veloitukset samaan kuittiin (yhteistilaukset, splitatut)

## Liittyvät

- `#28` — alkuperäinen monivaluuttatuki-issue (C3-implementaatio)
- `#73` — expenses-currency-block-kytkentä (kärsii samasta semantiikkakysymyksestä)
- `crates/server/src/tools/receipts/save_receipt.skill.md` — nykyinen skill-prompti joka kuvaa total_amount EUR:na
- `crates/ops/src/receipts/currency.rs::validate_currency_block` — nykyinen cross-check-logiikka
- `crates/ops/migrations/020_currency_fields.sql` — nykyinen schema (CHECK-rajoitteet)
