---
created: 2026-04-29
updated: 2026-04-29
type: improvement
reporter: jari
assignee: jari
status: in-progress
priority: normal
labels: [email, attachments, resilience]
related: ["#43", "#45"]
---

# 46. Extract attachments before/independent of spam verdict

_Source: Roundcube round-trip jossa rescue ei nähnyt liitteitä_

## Description

`process_message_inner` ajaa vision-OCR:n liitteille **vain jos
spam-verdikti on Clean**. Jos viesti merkitään `suspicious`/`spam`/
`monitor_only`, prosessi siirtää viestin folderiin ja merkitsee
`email_processing.status`:in, mutta `attachments` ja `extractions`-
taulut jäävät tyhjiksi sen viestin osalta.

Tämä rikkoo `gsadmin email rescue`-flow:n: kun rivi flipataan
`retryable`-tilaan myöhemmin (esim. transient bug korjattu),
`process_retries` lukee `db::load_extraction_summaries`:in joka on
tyhjä → agent saa pelkän body_plain:n → ei voi prosessoida liitteitä
joita kuitenkin oli alkuperäisessä viestissä.

## Reproduction

1. Lähetä viesti agentille jossa on PDF-liitteitä, ennen kuin
   known-sender-bypass on aktiivinen → spam-verdikti suspicious
2. Korjaa bypass, rescue-flippaus → retry-polku
3. Agentti vastaa "en näe liitteitä", vaikka liitteet ovat IMAPissa

Toistui Jarin Roundcube-demoissa 2026-04-29 (3 + 22 + 1 PDF-liitettä,
0 prosessoitu).

## Korjaussuunnitelma

- [x] Aja `extraction::process_attachment` kaikille liitteille
      **ennen** spam-/handler-policy-päätöstä, tai ainakin kaikille
      verdikteille jotka voidaan myöhemmin rescate (Clean +
      Suspicious + MonitorOnly). Spam-pure (DMARC reject p=reject)
      voi edelleen skipata.
- [x] Säilytä lokituksessa selvä erottelu: extraktio onnistui mutta
      viesti silti hylättiin spam-syistä. Auditoinnin pitää näyttää
      molemmat tilat.
- [x] Jos extraction-tulokset luetaan myöhemmin rescue:n yhteydessä,
      ne ovat valmiina — ei IMAP re-fetchiä tarvita (ks. #47:n
      vaihtoehto B).

## Toteutus

Extractionin gating-päätös: `handler::should_extract_attachments`
(`recipient == "assistant"` && verdict ≠ Spam && liitteitä > 0 &&
Clean-polulla decision.reply == CanReply). `process_message_inner`
kutsuu uutta `extract_attachments`-helperia **spam-verdiktin ja
routing-päätöksen jälkeen, mutta ennen päätöksen suorittamista** —
samalla trace_id:llä syntyy ensin `extraction.complete`-rivit
liitteille, sitten `extraction.summary`, ja sitten `message.skipped`
(suspicious-polku) tai agentin vastaus (clean-polku).

Aiemmin `process_assistant_reply` ajoi extractionin itse ennen
agenttiloopia. Nyt se lukee samat summaryt
`db::load_extraction_summaries`:llä — sama polku jota retry-flow
käytti aiemmin. Yhden lähteen total: rescue ja first-attempt näkevät
saman datan.

### Round 2 -korjaukset (saman PR:n sisällä)

Reviewn (gemini + gpt-5.5 + claude-opus-4-7 + deepseek) jälkeen:

- **Migraatio 013** lisää `UNIQUE (attachment_id)` `extractions`-tauluun,
  ja `process_attachment`-INSERT käyttää `ON CONFLICT DO UPDATE`. Estää
  duplikaattirivien syntymisen Reclaim-polulla (kts. #1 review-listassa).
- **Compute decide() ennen extractionia**, jolloin Clean+NoReply-viestit
  (Auto-Submitted, List-Id, DSN-bouncet, noreply@) eivät enää aja
  vision-OCR:ia turhaan. Predikaatti `should_extract_attachments` ottaa
  nyt `&Decision`-parametrin (kts. #2 review).
- **Liitteiden caps**: `MAX_ATTACHMENTS_PER_MESSAGE = 15`,
  `MAX_TOTAL_ATTACHMENT_BYTES = 25 MB`. Suoja denial-of-wallet -
  hyökkäyksiä vastaan jos hyökkääjä spoofaa known-senderiä Suspicious-
  polulle (kts. #3 review).
- **Kaikki-tai-ei-mitään-extraction**: `extract_attachments` palauttaa
  `ExtractionStats`. Jos jokin liite epäonnistuu Clean+CanReply-polulla,
  `process_message_inner` palauttaa `Err` → IMAP jättää viestin INBOXiin
  → seuraava IDLE-cycle reclaims + retry. Migraatio 013 tekee
  uudelleenyrityksestä idempotentin (kts. #8 review, Gemini'n näkemys).
- **`load_extraction_summaries`-virheet eivät enää nielty hiljaa**:
  sekä `process_assistant_reply` että `retry_assistant_message` reitittävät
  DB-virheen retry-jonoon `handle_first_ai_error`/`schedule_or_fail`:lla
  sen sijaan että lähettäisivät agentin tyhjällä summary-listalla
  (kts. #4 review).
- **LLM-mockit**: lisätty `wiremock` dev-dep ja `AnthropicClient::with_url`
  jonka avulla integraatiotestit ajavat oikean `process_attachment`-koodin
  fake-Anthropic-endpointtia vasten — ei enää SQL-fake-tuottaja-rivejä
  (kts. #6 review).
- **Cosmetic**: `extract_attachments` ottaa `&str` `&Configin` sijaan;
  no-AI-client-loki on `warn` `debug`:n sijaan; `assistant_state` on
  `.expect()`:llä invariantti.

### Round 3 -korjaukset (saman PR:n sisällä)

Toinen LLM-review focused-deltalle löysi 3 oikeaa bugia round-2:n
korjauksista:

- **Migraatio 013 dedup**: pre-#46 Reclaim saattoi minted duplikaatti-
  rivit `extractions`-tauluun. `CREATE UNIQUE INDEX` aborttaisi
  prod-deployssa. Lisätty defensiivinen DELETE + `receipts.extraction_id`
  FK:n re-pointing ennen indeksin luontia.
- **Permanent vs Transient -virhejaottelu**: Anthropic 4xx (esim.
  korruptoitunut PDF, content-policy-hylkäys) ei ole transient, mutta
  `extract_attachments` käsitteli sen `failed`:nä → bail → IMAP reclaim
  → loop forever. `process_attachment` luokittelee nyt:
  - **Permanent** (size, MIME, Anthropic 4xx) → kirjoittaa stub-
    `extractions`-rivin (`content_type: "extraction_skipped"` +
    `skip_reason` + agentille suomeksi käyttäjähinttiä) → palauttaa
    `Ok(summary)` → agent ajaa ja kertoo käyttäjälle.
  - **Transient** (5xx after retries, network, DB) → palauttaa `Err`
    → bail → IMAP reclaim retryaa idempotency:n turvin.
  - Agentin `AGENTS.md`-promptiin lisätty ohje: jos liite on
    `extraction_skipped`, mainitse käyttäjälle ja pyydä uudelleen
    tuetussa formaatissa.
- **Policy-skip Clean+CanReply lähettää torjuntaviestin**: kun 16+
  liitettä tai 26+ MB ylittää käsittelyrajan, ei ajeta agenttia
  tyhjillä summaryilla (joka loi #46-bugia uudestaan). Sen sijaan
  lähetetään templated "liitteitä liikaa, lähetä pienemmissä erissä"-
  vastaus suoraan käyttäjälle.

### Spin-off-issuet

- **#55** — Multi-worker IMAP processing per account (extraktio blokkaa
  IDLE-loopin nyt myös Suspicious-polulla — pre-existing Clean-polulla,
  laajeni #46:ssa).
- **`load_extraction_summaries` not tenant-scoped** — pre-existing,
  cosmic-ray nykyisellä yhdellä tenantilla, mutta block multi-tenant
  rolloutia (filed inside this issue's notes for now).

## Testit

Yksikkötestit (`handler::should_extract_attachments`, 11 kpl):
- Clean+CanReply / Suspicious / MonitorOnly + assistant + liitteitä → true
- Spam (DMARC reject) → false
- Tyhjät liitteet → false
- healthcheck / postmaster / käyttäjäpostilaatikot → false
- Clean + Auto-Submitted / List-Id / DSN bounce / noreply@ → false
- Suspicious vaikka NoReply-päätös → true (rescue tarvitsee rivit)

Yksikkötestit (`extraction::check_extraction_policy`, 5 kpl):
- Normaalibatch / tyhjä → ok
- Liian monta liitettä → TooManyAttachments
- Yli koko-cap → TotalSizeExceeded
- Count-rajan tarkistus ennen size-rajaa

Integraatiotestit (`tests/extraction_rescue.rs`, sqlx + wiremock, 5 kpl):
- Real `process_attachment` → DB → `load_extraction_summaries` round-trip
- Reclaim ei tuplaa rivejä `attachments`/`extractions`-tauluissa
- Liitteiden järjestys säilyy
- Tyhjä message_id palauttaa tyhjän Vecin (ei virhettä)
- Reclaim päivittää extracted_data:n uusimmasta tuloksesta

Manuaalinen end-to-end (Roundcube round-trip): toistettava
reproduktion jälkeen uudelleen. Lähetä viesti suspicious-polulle (esim.
ennen kuin known-sender-bypass aktivoituu) → flippaa rescue → tarkista
`gs-email-cli history` että agentti näkee tool_use-blokit liitteille.

## Out of scope

- Liitteiden re-fetch IMAPista (#47, vaihtoehto B).
- Spam-verdiktin muuttaminen.

## Trade-off

- **+** Rescue toimii — agentti näkee liitteet jälkikäteenkin.
- **+** Jos viesti merkattiin väärin spam:ksi, sisältö on jo
  jäsennetty ja saatavilla.
- **−** Hieman LLM-kustannusta spam-viesteistä (vision-OCR).
- **−** Pieni risk: spam-viestien sisältö menee LLM:lle. (Tämä on
  ongelma vain jos LLM-providerin sopimusehdot vaikuttavat.)
