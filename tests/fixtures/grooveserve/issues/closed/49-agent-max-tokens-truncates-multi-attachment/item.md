---
created: 2026-04-29
updated: 2026-04-29
closed: 2026-04-29
type: bug
reporter: jari
assignee: jari
status: fixed
priority: normal
labels: [email, agent, llm, attachments]
related: ["#46", "#48"]
commits:
  - hash: 64e2e40
    summary: "fix(email): raise max_tokens — agent 4096→32000, extraction 2048→8192 (#49)"
---

# 49. Agent reply truncates (MaxTokens) on messages with many attachments

_Source: 2026-04-29 round-trip demo, message `fb8d90eca…` 21 liitettä_

## Description

Kun käyttäjä lähettää sähköpostin jossa on iso määrä liitteitä (tässä
21 PDF/PNG-tiedostoa, "Maaliskuun kuitit"), `process_message`-polku
suorittaa vision-OCR:n kaikille onnistuneesti (kaikki 21 ekstraktiota
tallennettu DB:hen), mutta sen jälkeen agentin **ensimmäinen LLM-
iteraatio palaa `stop_reason=MaxTokens`** ennen yhdenkään tool_use-
blokin valmistumista.

Lokin todiste:

```
"Agent iteration","iteration":1,"stop_reason":"MaxTokens",
"input_tokens":10642,"output_tokens":4096,
"message_id":"<fb8d90eca2439c624e76cb3c1d37aac9@grooveserve.local>"
```

Käyttäjälle lähetetty vastaus (`thread_messages.id=18`):

> Kiitos kuitteista. Tallennan kaikki laskut. Yhteenveto:
>
> [Reply truncated — model hit max_tokens before finishing this turn.]

Lopputulos: 21 ekstraktiota DB:ssä, **0 receipts/expenses tallennettu**
tästä viestistä. Käyttäjä saa puutteellisen vastauksen.

## Reproduction

1. Lähetä `assistant@grooveserve.local`-osoitteeseen sähköposti jossa
   ≥10 PDF-liitettä (kuittikokoelma)
2. Tarkista `email-service.log` — etsi `stop_reason":"MaxTokens"`
3. Tarkista DB:
   - `attachments` ja `extractions` täyttyvät normaalisti
   - `receipts`/`expenses` jäävät tyhjiksi tämän viestin osalta
   - `thread_messages.body_plain` (tai vastaava) sisältää
     "[Reply truncated]"-merkinnän

## Korjausvaihtoehtoja

### A. Nosta `max_tokens` rajaa
Yksinkertaisin: nykyinen 4096 → esim. 16000–32000. Claude Haiku/
Sonnet 4.5+ tukee jopa 64000 output-tokenia. **Plus**: ei muuta
arkkitehtuuria. **Miinus**: yksittäinen kallis kutsu, ei skaalaa
50+ liitteen tapauksiin.

### B. Loopita MaxTokens-tilan ohi
Jos `stop_reason == MaxTokens`, jatka samaa keskustelua uudella
iteraatiolla (vastaus jatkuu siitä mihin jäi). Vaatii että agent-
loopissa `MaxTokens` on legit "continue"-signaali, ei terminaali.
**Plus**: skaalautuu pitkille listoille. **Miinus**: monimutkaisempi
state-management; hieman duplikointia jos malli tuottaa "Yhteenveto:"-
prefiksi joka uudessa kutsussa.

### C. Käsittele liitteet erissä
Pilko viestin liitteet 5–10 kpl eriin ja aja agent-loop kerran per
erä. Joka erän jälkeen sama agent-konteksti, mutta extraction-
blokit ja toolit suoritetaan pienemmissä ryhmissä. **Plus**: skaalaa
hyvin, jokainen erä mahtuu max_tokens-budjettiin. **Miinus**: vaatii
agent-loopin refaktoroinnin ja erittäin tarkkaa kontekstin
hallintaa (käyttäjälle yksi vastaussähköposti, ei N kpl).

## Suositus

**A ensimmäisenä korjauksena** (nosta max_tokens 16000:een), MVP-
vaiheessa riittävä. **B** myöhemmin jos osoittautuu että jopa 16000
loppuu kesken (esim. 50+ liitettä). **C** vasta jos batch-luonteinen
tarve on toistuva.

## Korjauksen jälkikäteinen tarkennus (2026-04-29)

Vakio oli kovakoodattu kahdessa paikassa:

- `services/email/src/agent/mod.rs:23` — `MAX_TOKENS = 4096` (agent-loop)
- `services/email/src/extraction.rs:159` — `max_tokens: 2048` (vision-OCR per liite)

Anthropic-API:n todelliset rajat ovat huomattavasti suuremmat (Sonnet 4.6
ja Haiku 4.5: 64 000, Opus 4.7: 32 000), joten sovellus pyysi vain
~7 % siitä mitä malli olisi voinut tuottaa.

**Tehty (vaihtoehto A):**

- `MAX_TOKENS` 4096 → **32000** (toimii kaikilla nykyisillä malleilla,
  21–70 kuitin viestit mahtuvat budjettiin)
- `extraction.rs` `max_tokens` 2048 → **8192** (~20× marginaali yhden
  kuitin JSON-blobille; kattaa monisivuiset ALV-laskut ilman
  truncationia)

**Yhä auki (vaihtoehto C, erillinen architectural improvement):**

`save_receipt` ottaa nykyisin `raw_text`/`items`/`vendor`/`total_amount`
parametreina — agentti joutuu kopioimaan ne extraction-blokista
toolikutsuun. Tämä tuplaa output-token-kulutuksen. Cleaner: `save_receipt`
ottaisi pelkän `extraction_id`:n ja kopioisi kentät palvelinpuolella
DB:stä (extraction-rivi sisältää jo `extracted_data`-JSON:in). Agentin
rooli typistyy "valitse mitkä ekstraktiot tallennetaan + mahd. korjaa
yksittäisiä kenttiä", ei "regurgitoi koko OCR-data". Päätettäneen
erikseen — jos token-säästö tarpeen, file separate issue.

**Verification pending:** uusi 21-liitteen Roundcube-testi
korjattua binääriä vasten — odotetaan että agentti saa kaikki 21
save_receipt-kutsua mahdutettua yhden iteraation budjettiin (output
arvio: 21 × ~400 = ~8400 tokenia, hyvin alle 32 000).

## Verifiointi 2026-04-29 (worktree `c1-verify-49-maxtokens`)

Lokaali round-trip-testi `gsdev mail send`-CLI:n kautta:

- **Lähetys**: `jari@grooveserve.local` → `assistant@grooveserve.local`,
  subject "Huhtikuun kuitit (#49 v3)", **25 liitettä** (PDF + PNG)
  hakemistosta `~/Downloads/2026/Huhtikuu/`
- **Loki todistaa korjauksen**:
  - 4 agent-iteraatiota: stop_reason `ToolUse`, `ToolUse`, `ToolUse`, `EndTurn`
  - **Ei MaxTokens-stop_reasonia** missään iteraatiossa
  - Iteraatio 1: input=13 473, output=**5 860** tokenia (huom: yli vanhaa
    4 096-rajaa, joka oli vika)
  - Total: 91 587 input + 10 737 output tokens
- **DB todistaa täydellisen tallennuksen**:
  - `attachments` = 25
  - `extractions` = 25
  - **`receipts` = 25** (kaikki vendor + total_amount oikein, esim.
    "Mailgun 32.00 EUR", "Hetzner 59.61 EUR", "Airbnb 75.00 EUR", VR-liput
    23.80–84.70 EUR)
  - **`expenses` = 25**
- **Käyttäjälle lähtevä vastaus** ei sisällä truncation-merkintää, vaan
  oikean yhteenvedon: "Tallensin kaikki 25 kuittia. … Yhteensä 1 793,34 €".

**Sivuhuomio (verifiointiprosessista):** ensimmäinen yritys epäonnistui
koska `gsdev mail send` käyttää välimuistissa olevaa
`~/.cache/gsdev/cli/release/gs-email-cli`-binääriä, ei worktreen omaa
`target/debug/gs-email-cli`. Cache oli buildattu 2026-04-27 (ennen
korjausta). Manuaalinen `rm` + uudelleen-trigger ratkaisi tilanteen,
mutta tämä on oma sudenkuoppansa joka ansaitsee oman issuen jos toistuu
muillakin verifioinneilla — ks. `analysis-cache-staleness.md`.

Status: **fixed**.

## Suhde #46:een

#46 ("extract aina") koskee retry-polkua ja spam-haaraa. Tämä bug
osuu `process_message`-polun normaaliin Clean-haaraan kun
extraction onnistui mutta vastausgenerointi katkesi. Eri vika,
sama oire (käyttäjä ei saa vastausta jossa olisi tallennetut
kuitit).

## Quick fix for stuck demo state

Tällä hetkellä DB:ssä on 24 ekstraktiota mutta vain 4 tallennettua
receiptia. Voi pyytää agenttia uudelleen: muotoile sähköposti
"käsittele myös aiemmin lähettämäni Maaliskuun kuitit" tai poista
`thread_messages`-rivit ko. viestin osalta ja tee `gsadmin email
rescue` (kun korjaus käytössä).
