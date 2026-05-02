# Hyväksyntäkierto — analyysi

Matkalaskun hyväksyntä osana AI-agentin keskustelua. Esimies hyväksyy tai hylkää vastaamalla agentin sähköpostiin vapaalla tekstillä.

## Arkkitehtuurin yleiskuva

```
1. AI-agentti koostaa matkalaskun käyttäjän kuiteista/kalenterista
       ↓
2. Agentti lähettää esimiehelle sähköpostin:
   "Matti kävi Tampereella 15.4., kulut 245,80 €. Hyväksytkö?"
       ↓
3. Esimies vastaa vapaalla tekstillä: "joo ok" / "ei, hotelli liian kallis"
       ↓
4. Stalwart vastaanottaa → IMAP IDLE → email-service
       ↓
5. Email-service persistoi viestin + luo käsittelyjob (ei blokkaa IMAP:ia)
       ↓
6. Worker: Rust-koodi parsii From, In-Reply-To, Authentication-Results
   → injektoi kontekstin (lähettäjä, auth-tulos, viestiketju) agenttiin
       ↓
7. Agentti tulkitsee vastauksen (LLM) — näkee vain uusimman vastauksen
       ↓
8. Agentti kutsuu approve/reject -toolia
       ↓
9. Tool: tarkistaa oikeuden (injektoitu konteksti), kirjaa päätöksen
       ↓
10. Agentti lähettää vahvistuksen esimiehelle ja tekijälle
```

### Agenttisen loopin konteksti-injektio

Rust-runtime hallitsee kontekstin, ei LLM:

- **Lähettäjän identiteetti**: From-osoite sähköpostista, ei LLM:n kertomana
- **Authentication-Results**: Stalwartin DKIM/SPF/DMARC-tulos headerista
- **Viestiketjun konteksti**: In-Reply-To → tiedetään mihin pyyntöön vastataan
- **Expense-konteksti**: agentti itse lähetti pyynnön, tietää mistä matkalaskusta on kyse

Jokainen tool-kutsu saa nämä runtime-kontekstina. LLM ei valitse expense_id:tä eikä lähettäjän identiteettiä — ne ovat tiedossa sähköpostista ja agentin omasta kontekstista.

## Miten tämä eroaa perinteisestä hyväksyntäkierrosta

Perinteinen malli: erillinen tilakone, HMAC-tokenit Reply-To:ssa, avainsanatunnistus, kryptografinen validointi.

Tämä malli: **hyväksyntä on agentin tool-kutsu**. Agentti ymmärtää luonnollista kieltä, ja tool tekee oikeustarkistuksen. Ei tarvita tokeneita, VERP-osoitteita eikä avainsanalistoja.

Etuja:
- Esimies voi vastata miten haluaa — "ok", "hyväksyn", "joo menee", "looks good"
- Hylkäyksen syy tulee luonnollisesti mukana — "ei, hotellikulut yli budjetin"
- Agentti voi kysyä tarkennuksia: "Hylkäätkö kokonaan vai vain hotellin osalta?"
- Ei tarvetta opettaa käyttäjälle erityisiä avainsanoja

## Agentin approve/reject -tool

### Tool: `approve_expense`

```
LLM:n antamat parametrit:
  decision:    string    — "approved" | "rejected"
  comment:     string?   — vapaamuotoinen kommentti (erityisesti hylkäyksessä)

Runtime-konteksti (injektoitu Rust-koodista, ei LLM:ltä):
  sender_email:          string    — From-osoite sähköpostista
  authentication_result: string    — Stalwartin DKIM/SPF/DMARC-tulos
  expense_id:            integer   — agentin kontekstista (agentti lähetti pyynnön)
  expense_version:       integer   — matkalaskun versio pyyntöhetkellä
```

### Tooliin sisäänrakennettu logiikka

Tool ei luota agenttiin sokeasti. Ennen hyväksynnän kirjaamista tool:

1. **Tarkistaa autentikoinnin** — Authentication-Results: DMARC=pass vaaditaan
2. **Tunnistaa lähettäjän** — injektoitu From-osoite → käyttäjähaku tietokannasta
3. **Tarkistaa hyväksyntäoikeuden** — onko tämä henkilö matkalaskun tekijän esimies / hyväksyjä?
4. **Tarkistaa matkalaskun tilan** — onko se tilassa jossa hyväksyntä on mahdollista? (`SELECT ... FOR UPDATE`)
5. **Tarkistaa version** — onko matkalasku muuttunut pyyntöhetken jälkeen?
6. **Kirjaa päätöksen** — tallentaa tietokantaan kuka hyväksyi, milloin, millä kommentilla

Jos mikä tahansa tarkistus epäonnistuu, tool palauttaa virheen agentille, joka viestii tilanteen esimiehelle.

### Turvallisuus

Turvallisuus ei nojaa sähköpostin kryptografiaan vaan tooliin:

| Uhka | Esto |
|------|------|
| Väärä henkilö yrittää hyväksyä | Tool tarkistaa lähettäjän oikeuden tietokannasta |
| Agentti kutsuu toolia ilman esimiehen vastausta | Agentti saa toolin käyttöoikeuden vain kun keskustelussa on esimiehen viesti |
| Matkalasku hyväksytään kahdesti | Tool tarkistaa tilan — jo hyväksyttyä ei voi hyväksyä uudelleen |
| Spoofattu From-osoite | Stalwartin DMARC-validointi (Authentication-Results header) |
| Matkalasku muuttunut pyyntöhetken jälkeen | Tool vertaa expense_version — pyytää uuden hyväksynnän jos muuttunut |
| Tuplahyväksyntä (race condition) | `SELECT ... FOR UPDATE` + transaktio |

Stalwart tarkistaa DKIM, SPF ja DMARC automaattisesti saapuville viesteille. Tool vaatii **DMARC=pass** (ei pelkkä DKIM tai SPF erikseen) — DMARC varmistaa, että From-domain ja allekirjoitus ovat linjassa. Pelkkä DKIM=pass todistaa vain allekirjoittavan domainin, ei lähettäjää.

### Mitä EI tarvita

- HMAC-tokenit Reply-To -osoitteessa (agentti tietää kontekstin, ei tarvita kryptografista sidontaa)
- VERP/subaddressing
- Avainsanalistat (HYVÄKSYTTY/HYLÄTTY)
- Erillinen hyväksyntätilakone
- `mail-auth` -crate (Stalwart hoitaa)

## Tietokanta

### Hyväksyntätaulu

```sql
CREATE TABLE expense_approvals (
    id                  SERIAL PRIMARY KEY,
    expense_id          INTEGER NOT NULL REFERENCES expenses(id),
    expense_version     INTEGER NOT NULL,        -- matkalaskun versio pyyntöhetkellä
    approver_user_id    INTEGER NOT NULL REFERENCES users(id),
    approver_email      TEXT NOT NULL,            -- email-osoite päätöshetkellä (snapshot)
    decision            TEXT NOT NULL CHECK (decision IN ('approved', 'rejected')),
    comment             TEXT,                     -- vapaamuotoinen kommentti
    request_message_id  TEXT NOT NULL,            -- agentin lähettämän pyynnön Message-ID
    response_message_id TEXT NOT NULL UNIQUE,     -- esimiehen vastauksen Message-ID
    auth_results        TEXT,                     -- Authentication-Results header
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_expense_approvals_expense ON expense_approvals(expense_id);
```

Hyväksyntätaulun olennaiset lisäykset verrattuna alkuperäiseen:
- `expense_version` — estää muuttuneen matkalaskun hyväksymisen
- `approver_user_id` — FK käyttäjään (email voi muuttua)
- `CHECK`-rajoitus decisionille
- `NOT NULL` + `UNIQUE` message_id:ille — idempotenssi
- `auth_results` — audit trail

### Matkalaskun tilasiirtymä

```
draft → pending_approval → approved → exported
                         → rejected → draft (korjattu versio)
```

Tilasiirtymä tapahtuu `expenses`-taulussa, ei erillisessä approval-taulussa.

## Sähköpostin kulku

### Agentin viesti esimiehelle

Agentti muotoilee viestin luonnollisella kielellä osana keskustelua:

```
From: Grooveserve <assistant@grooveserve.com>
To: esimies@yritys.fi
Subject: Matti Meikäläisen matkalasku: Helsinki–Tampere 15.4.2026

Hei,

Matti Meikäläinen on lähettänyt matkalaskun hyväksyttäväksi:

  Matka: Helsinki → Tampere, 15.4.2026
  Junaliput: 45,80 €
  Päiväraha (kokopäivä): 51,00 €
  Hotelli: 149,00 €
  Yhteensä: 245,80 €

Hyväksytkö matkalaskun? Voit vastata tähän viestiin.

– Grooveserve
```

### Esimiehen vastaus

Vapaamuotoinen — mitä tahansa:
- "Joo hyväksyn"
- "Ok"
- "Ei mene läpi, hotellibudjetti on 120€/yö"
- "Junaliput ok mutta hotelli pitää selvittää"

### Agentin tulkinta

LLM tulkitsee vastauksen ja päättää toiminnon. Jos vastaus on epäselvä, agentti kysyy lisää sen sijaan että tekee oletuksen.

**Quoted text -käsittely:** Esimiehen vastaus sisältää yleensä alkuperäisen viestin lainattuna (`> Hyväksytkö matkalaskun?`). LLM:lle syötetään vain uusin vastaus, ei koko viestiketjua. Tämä vähentää tokenien kulutusta, hallusinaatioriskiä ja prompt injection -mahdollisuutta. Käytännössä tarvitaan `email_reply_parser` -crate tai vastaava logiikka quoted text -poistoon.

**Selektiivinen vahvistus:** Selkeissä tapauksissa ("ok", "hyväksyn") agentti voi kutsua toolia suoraan. Epäselvissä tai ehdollisissa vastauksissa ("junaliput ok mutta hotelli pitää selvittää") agentti kysyy tarkennusta ennen tool-kutsua.

## Miten tämä istuu nykyiseen email-serviceen

### Nykyinen arkkitehtuuri

Email-service monitoroi Stalwart-tilejä IMAP IDLE:llä ja reititää viestit `handler.rs`:ssä vastaanottajan perusteella. Tällä hetkellä `assistant@` palauttaa ping-pong-vastauksen.

### Muutos

`assistant@`-tilin handler vaihtuu ping-pongista AI-agenttiin. Agentti:
- Ylläpitää keskusteluhistoriaa per viestiketju (In-Reply-To/References), ei per lähettäjä
- Käyttää LLM:ää vastausten tulkintaan
- Kutsuu tooleja (approve_expense, reject_expense, create_expense, ...)

### Asynkroninen käsittely

LLM-kutsut kestävät 3-30 sekuntia. Ne **eivät saa blokata IMAP IDLE -looppia**, muuten yhteydet katkeavat ja viestejä jää huomaamatta.

Ratkaisu:
1. IMAP-handler parsii viestin ja tallentaa sen tietokantaan → luo käsittelyjob
2. Erillinen worker poimii jobin ja ajaa agentin (LLM + tool-kutsut)
3. Worker lähettää vastaukset SMTP:llä

Tämä erottaa viestin vastaanoton (nopea) käsittelystä (hidas).

Hyväksyntäkierto on vain yksi agentin tooleista — ei erillinen järjestelmä.

### Uusi tili?

Ei välttämättä tarvita erillistä `approvals@`-tiliä. `assistant@` hoitaa kaiken — myös hyväksyntäkeskustelut esimiesten kanssa. Yksi agentti, yksi osoite.

## MVP-suositus

### Vaihe 1: approve/reject -tool
1. `expense_approvals` -tietokantataulu ja migraatio
2. Tool-toteutus: oikeustarkistus, tilasiirtymä, tallennus
3. Yksikkötestit toolille

### Vaihe 2: Agentin hyväksyntäviesti
1. Agentin prompt: milloin ja miten pyytää hyväksyntää
2. Sähköpostin muotoilu esimiehelle
3. Vastauksen reititys oikeaan keskusteluun

### Vaihe 3: Vahvistukset ja edge caset
1. Vahvistusviesti tekijälle (hyväksytty/hylätty)
2. Epäselvä vastaus → agentti kysyy tarkennuksen
3. Osittainen hylkäys → agentti neuvottelee

### Rajaus MVP:n ulkopuolelle
- Monen hyväksyjän ketju
- Eskalointisäännöt (ei vastausta N päivässä)
- Web-pohjainen hyväksyntä
- Hyväksynnän delegointi (sijainen)

### Toteutettavuus

Teknisesti yksinkertainen. Uutta Rust-koodia tarvitaan:
- Tool-funktio (oikeustarkistus + DB-kirjoitus) — suoraviivaista
- Tietokantamigraatio — yksi taulu
- Agentin prompt-muutokset — ei Rust-koodia

Suurin avoin kysymys on AI-agentin toteutus itsessään (issue #2), johon hyväksyntä on yksi tool muiden joukossa.

## Nykyisen email-servicen korjaustarpeet

Hyväksyntäkierron toteutus vaatii näitä muutoksia olemassa olevaan koodiin:

### email.rs — puuttuvat headerit

`ParsedEmail` tarvitsee uudet kentät:
- `in_reply_to: Option<String>` — viestiketjun sidonta
- `references: Vec<String>` — laajempi ketjutieto
- `authentication_results: Option<String>` — Stalwartin DMARC-tulos
- HTML-body fallback: jos `body_text(0)` on tyhjä, käytä `body_html(0)` + HTML→text muunnos (monet Outlook-käyttäjät lähettävät vain HTML:ää)

### handler.rs — `should_skip_sender`

`from_lower.contains("@grooveserve.com")` estää kaiken sisäisen liikenteen, mukaan lukien testauksen ja mahdolliset sisäiset hyväksyjät. Pitää muuttaa tarkemmaksi: ohita vain tunnetut palvelutilit (assistant@, healthcheck@), ei koko domainia.

### handler.rs — asynkroninen käsittely

`route()` on nykyisin synkroninen. LLM-käsittelyä varten tarvitaan erillinen worker-malli (ks. "Asynkroninen käsittely" yllä).
