---
created: 2026-05-01
updated: 2026-05-01
type: task
reporter: jari
assignee: jari
status: open
priority: high
epic: 56
related: ["#99"]
labels: [testing, e2e, local-dev]
---

# 116. E2E-perustestit lokaalisti

_Source: koko järjestelmä (sähköpostiputki, web-UI, agentti)_

## Goal

Varmistaa että Grooveserven peruskäyttötapaukset toimivat päästä päähän
lokaalissa dev-ympäristössä. Testit ajetaan AI-agentin toimesta
(`gsdev` + `gsadmin` + Playwright/`curl`), ei manuaalisesti.

Tämä on **perustason toiminnallisuustestaus** — varmistetaan että putket
ovat ehjät, ei validoida kuittien sisällön tarkkuutta. Sisällöllinen
validointi tulee dog food -testauksessa.

## Testiympäristö

- **Instanssi**: `_main` (päärepo, `gsdev instance ensure` ajettu)
- **DB**: `GSADMIN_EMAIL_DB_URL` + `GSADMIN_API_DB_URL` lokaaliin Postgresiin
- **Sähköposti**: `gsdev mail send` / `gsdev mail send-eml` (Mailpit), opt-in GreenMail + Roundcube
- **Web**: `curl` API-endpointteihin + Playwright `sites/www/` + `crates/server/` konffeilla
- **Palvelin**: `cargo watch` (workmux-taustalla) tai manuaalinen `grooveserve-server`

## Mitä testataan

### T1 — Sähköpostiputki (email ingestion)

Näissä ei validoida OCR-sisällön tarkkuutta, ainoastaan että putki
toimii: viesti vastaanotetaan, liite käsitellään, receipt syntyy,
agentti vastaa.

- **T1.1** Kuitti PDF-liitteellä → receipt + extraction syntyy
- **T1.2** Kuitti PNG-liitteellä → receipt + extraction syntyy
- **T1.3** Kaksi kuittia peräkkäin samalta käyttäjältä → molemmat receiptit syntyvät
- **T1.4** Tuntematon lähettäjä → `unknown_sender`, ei receiptiä
- **T1.5** Viesti ilman liitettä → graceful handling (ei kaatumista)

**Kuittilähde**: `~/Downloads/2026/` — käytetään oikeita kuitteja.
Valitaan 3–5 edustavaa kuittia eri kuukausilta ja eri tyypeistä
(VR, Airbnb, Hetzner, Mailgun, Anthropic).

**Verifiointi**: `gsadmin email trace <msg-id>` + `gsadmin email list`

### T2 — Web-näkymä (API-taso)

- **T2.1** `/health` — palvelin vastaa
- **T2.2** Login → sessio syntyy
- **T2.3** `GET /api/receipts` → lista käyttäjän tositteista
- **T2.4** `GET /api/receipts/:id` → yksittäisen tositteen tiedot
- **T2.5** `GET /api/attachments/:id` — liitteen lataus

**Verifiointi**: HTTP-statuskoodit + response body -tarkistukset `curl`illa

### T3 — Web-näkymä (Playwright-selain)

- **T3.1** Rekisteröityminen (onboarding-flow)
- **T3.2** Kirjautuminen sisään
- **T3.3** Tositelistaus avautuu ja näyttää todelliset receiptit
- **T3.4** Yksittäisen tositteen sivu (tiedot, liite)
- **T3.5** Navigaatio ja perus UI-elementit

**Konffit**: `sites/www/playwright.config.ts` + `crates/server/playwright.config.ts`

### T4 — Roundcube end-to-end (opt-in, IMAP + SMTP)

- **T4.1** `gsdev imap up` + `gsdev roundcube up` — ympäristö käynnistyy
- **T4.2** Viestin lähetys Roundcubesta → agentti prosessoi
- **T4.3** Agentin vastaus näkyy Roundcuben inboxissa

### T5 — Tietokannan eheys

- **T5.1** `email_processing`-riveillä on oikeat statukset
- **T5.2** `extractions` ↔ `receipts` ↔ `expenses` — viite-eheys
- **T5.3** `agent_runs` — jokaisella käsitellyllä viestillä on vähintään yksi ajo
- **T5.4** Ei orpoja rivejä (tarkistus `gsadmin email trace` -komennolla)

## Mitä EI testata

| Asia | Miksi ei |
|------|----------|
| OCR-sisällön tarkkuus (vendor, summa, päivämäärä) | Dog food -testaus |
| Monimutkaiset kuittityypit (hyvitykset, ALV-erittelyt, kausikortit) | Ei perustoiminnallisuutta |
| Virhetilanteet ja edge caset kattavasti | MVP-vaihe, peruspolku ensin |
| Hyväksyntäkierto (#21) | Ei vielä toteutettu |
| Kalenteri-integraatiot (#16, #17) | Ei vielä toteutettu |
| Netvisor/Procountor (#18, #19) | Ei vielä toteutettu |
| Suorituskyky ja kuormitustestaus | Ei relevantti MVP:lle |
| Tuotantoympäristö | Tämä on lokaali testaus; tuotantoa observoidaan erikseen |
| Spam-käsittely (#12) | Toimii riittävästi, ei kriittinen perustesteille |

## Testien suorittaminen

AI-agentti (tämä Claude Code -istunto) ajaa testit. Jokaisesta
testistä kirjataan:

1. Mitä ajettiin (komento)
2. Saatu tulos (status, output)
3. Pass/fail + havainnot

Testitulokset kirjataan tähän issueen `results.md`-tiedostona.

### Esiehdot

```bash
# 1. Varmista että lokaali ympäristö on kunnossa
gsdev instance ensure
gsdev status

# 2. Varmista että palvelin on päällä (workmux tai manuaalinen)
# API:sta vastaus: curl http://localhost:53004/health

# 3. Asenna tarvittavat riippuvuudet (vain kerran)
cd sites/www && pnpm install && pnpm exec playwright install chromium
cd crates/server && pnpm install && pnpm exec playwright install chromium

# 4. Aseta admin-työkalujen env-muuttujat lokaalille DB:lle
export GSADMIN_EMAIL_DB_URL=postgresql://grooveserve:devpassword@127.0.0.1:55432/grooveserve_email_main_main
export GSADMIN_API_DB_URL=postgresql://grooveserve:devpassword@127.0.0.1:55432/grooveserve_main_main
```

### Ajojärjestys

1. **T1** (sähköpostiputki) — ajetaan ensin, tuottaa dataa T2:lle ja T3:lle
2. **T5** (DB-eheys) — ajetaan T1:n jälkeen, tarkistaa syntyneen datan
3. **T2** (API) — `curl`-testit API-endpointteihin
4. **T3** (Playwright) — selainpohjaiset testit
5. **T4** (Roundcube) — opt-in, ajetaan jos halutaan demo

## Notes

- Playwright-konfiguraatiot ovat valmiina `sites/www/` ja
  `crates/server/` -hakemistoissa (`AGENTS-LOCAL-DEV.md` §E2E testing)
- `gsdev mail send-eml` ohittaa SMTP/IMAP-kerroksen — nopein tapa
  testata ingest-putkea
- `gsadmin email trace` antaa kattavan kuvan yhden viestin koko
  käsittelypolusta
- Testikuittien joukossa on sekä PDF- että PNG-muotoisia kuitteja —
  tämä kattaa molemmat päätapaukset
- Jos `_main`-instanssin `.env.local` puuttuu, `gsdev instance ensure`
  generoi sen
