---
created: 2026-04-29
updated: 2026-04-29
closed: 2026-04-29
type: feature
reporter: jari
assignee: jari
status: done
priority: normal
labels: [tooling, gsadmin, debug]
related: ["#48", "#49", "#50"]
---

# 51. gsadmin email trace — yhden viestin koko kaaren näkymä

_Source: 2026-04-29 demo, agentic loopin debugaus tarvitsi monta erillistä SQL-kyselyä_

## Description

Yhden viestin debugaaminen vaatii nyt 4–5 erillistä psql-kyselyä +
log-grepin. Komento `gsadmin email trace <message-id>` aggregoi
kaiken yhteen näkymään.

## Goal

```bash
gsadmin email trace "<fb8d90eca2439c624e76cb3c1d37aac9@grooveserve.local>"
```

näyttää (yhdessä):

1. **Email** — `email_processing`-rivi: status, spam_verdict, retry_count,
   created_at, reply_message_id
2. **Attachments** — kaikki `attachments`-rivit (filename, koko,
   content_type)
3. **Extractions** — `extractions.extracted_data` per liite
   (yhteenveto: vendor, total_amount, date — ei koko JSON)
4. **Receipts** — viestiin liittyvät `receipts` (vendor, total, status)
5. **Expenses** — `expenses`-rivit linkitettyjen kuittien kautta
6. **Logs** (vain `--log-file`-yliajolla) — log-rivit jotka
   matchaavat message_id:n

Detaljit (täydet content_block-blokit, koko keskustelu) ovat edelleen
`gs-email-cli history --email <addr>` -komennossa — trace antaa
"binäärihaun lähtökohdan", history zoomaa loppuun.

## Scope

- Komento: `gsadmin email trace <message-id>`
- Tukee `GSADMIN_EMAIL_DB_URL`-yliajoa (lokaali dev) — sama malli
  kuin `GSADMIN_DIRECT_DB_URL` `cmd_password_reset.py`:ssä.
  Prod-tilanne menee SSH-tunnelin kautta.
- `--json` flagillä koneellista tulostetta.
- `--log-file PATH` (valinnainen) lukee paikallisen log-tiedoston ja
  poimii matchavat rivit. Ilman tätä lokit jätetään väliin
  (prodissa logit menisivät journalctl:n kautta — myöhempi laajennus).

## Out of scope

- Conversations-taulun blokkien näyttäminen — `gs-email-cli history`
  tekee tämän paremmin.
- journalctl-haku prod-puolella — lisätään myöhemmin tai osana #50:tä.
- Trace useamman message_id:n yhdellä komennolla — vasta kun
  yksittäinen toimii.

## Quick Test

```bash
GSADMIN_EMAIL_DB_URL=postgresql://grooveserve:devpassword@127.0.0.1:55432/grooveserve_email_main_main \
  uv run gsadmin email trace "<fb8d90eca2439c624e76cb3c1d37aac9@grooveserve.local>" \
  --log-file /tmp/email-service.log
```

Odotettu output: 21 attachments, 21 extractions, 0 receipts (max_tokens-bug),
plus log-rivit jotka osoittavat `Agent iteration / stop_reason: MaxTokens`.

## Korjaussuunnitelma

- [x] Lisää `_email_db()` context manager `cmd_email.py`:hen
      (env-yliajo)
- [x] Lisää `trace` subkomento — yksi SQL-haku per relaatio
- [x] Tukee `--json`, `--log-file`
- [x] Test: ajettu 2026-04-29 demon datalla, fb8d90eca-viestillä
      löytyi 20 attachmenttia + 20 ekstraktiota + 0 receiptia (=
      #49 oire heti näkyvissä); c9c702f7-viestillä 3/3/3/3 toimii
- [x] Päivitetty `tools/admin/AGENTS.md` (= `CLAUDE.md`) komennolla

## Verification (2026-04-29)

```bash
GSADMIN_EMAIL_DB_URL='postgresql://grooveserve:devpassword@127.0.0.1:55432/grooveserve_email_main_main' \
  uv run gsadmin email trace 'fb8d90eca2439c624e76cb3c1d37aac9@grooveserve.local' \
  --log-file /tmp/email-service.log
```

Output kattaa email_processing-rivin, 20 attachmenttia (taulukko),
20 ekstraktiota (taulukko), 0 receiptia, 0 expensea, ja log-rivit
joista näkyy `stop_reason: MaxTokens` (= #49). Bare-ID
ilman angle bracketteja normalisoidaan automaattisesti.
