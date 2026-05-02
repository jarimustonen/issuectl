# Matkalaskun käsittelyputki — Design

Tämä dokumentti suunnittelee matkalaskujen käsittelyn ydinputken: työkalut, tietomalli ja agenttinen silmukka. Suunnitelman tavoite on muuttaa nykyinen "keskusteleva chatbotti" toimivaksi matkalaskuagentiksi, joka tallentaa, laskee ja koostaa.

> **Review:** Tämä design on käynyt läpi 4-LLM-review-prosessin (Gemini, GPT-5.5, Claude Opus, DeepSeek). Kriittiset havainnot on integroitu. Review-raportti: `history/review-matkalaskun-tyokalut.md`

## Sisällysluettelo

1. [Suunnitteluperiaatteet](#1-suunnitteluperiaatteet)
2. [Liitteiden käsittely](#2-liitteiden-käsittely)
3. [Tietomalli](#3-tietomalli)
4. [Raakadata → kirjanpitodata](#4-raakadata--kirjanpitodata)
5. [Matkan tunnistaminen](#5-matkan-tunnistaminen)
6. [Käyttäjäprofiili](#6-käyttäjäprofiili)
7. [Geolokalisaatio ja etäisyydet](#7-geolokalisaatio-ja-etäisyydet)
8. [Matkalaskuluonnos](#8-matkalaskuluonnos)
9. [Agentin työkalut](#9-agentin-työkalut)
10. [Agenttinen silmukka](#10-agenttinen-silmukka)
11. [Email-First UX -esimerkit](#11-email-first-ux--esimerkit)
12. [MVP-priorisointi](#12-mvp-priorisointi)

---

## 1. Suunnitteluperiaatteet

### Email-First UX

Sähköposti on ensisijainen käyttöliittymä. Jokainen toiminto, joka on käytettävissä web-UI:ssa, on käytettävissä myös sähköpostilla. Suunnittelu etenee aina järjestyksessä:

1. Suunnittele miten toimii sähköpostissa
2. Lisää web-UI saman backendin päälle

Käytännössä tämä tarkoittaa: agentti **tekee asioita** käyttäjän puolesta, ei pyydä käyttäjää täyttämään lomakkeita. Käyttäjä lähettää kuitin kuvan → agentti lukee sen, tallentaa tiedot, ja kertoo mitä ymmärsi.

### Unified Tool Surface

Agentin työkalut ja web-UI käyttävät samoja backend-operaatioita. Tämä toteutetaan komentokerroksena jossa on erilliset adapterit:

```
Sähköposti → AI-agentti → tool adapter → komentotaso → tietokanta
Web-selain → web-API → API adapter → komentotaso → tietokanta
```

AI-tool-adapter vastaanottaa probabilistista LLM-syötettä. Web-API-adapter vastaanottaa autentikoitua käyttäjäsyötettä. Molemmat kutsuvat samaa validoitua komentotasoa.

### LLM/backend-raja: "AI extracts, backend computes"

Selkeä vastuunjako LLM:n ja backendin välillä:

| LLM omistaa | Backend omistaa |
|---|---|
| Käyttäjän intentin tulkinta | Identiteetti ja auktorisointi |
| Liitteiden extraction (vision) | ALV-laskenta, päiväraha, km-korvaus |
| Luokitteluehdotus | Tilasiirtymät ja policy-validointi |
| Tarkentavat kysymykset | Idempotenssi ja transaktiot |
| Sähköpostin muotoilu | Rahamäärien viimeistely |

LLM **kerää faktat ja ehdottaa**. Backend **laskee, validoi ja tallentaa**. LLM ei koskaan päätä lopullisia rahamääriä. Ihminen tarkastaa ennen kirjanpitoa, ja kirjanpidossa vielä toinen tarkastus.

### Exact Decimal — ei float-tyyppejä rahalle

Rahamäärät ovat **aina** eksakteja desimaalilukuja (Rust: `rust_decimal::Decimal`, PostgreSQL: `NUMERIC`). Float-tyyppejä (`f32`, `f64`) ei saa käyttää rahalle missään kerroksessa — ei tallennuksessa, laskennassa, serialisoinnissa eikä API-rajapinnoissa. LLM:n JSON-syöte parsitaan `Decimal`-tyypiksi string-muunnoksen kautta (`Number.to_string() → Decimal::from_str`), jolloin pyöristysvirheitä ei synny.

### Kolmikerroksinen tietomalli

Jokaisesta kuitista on kolme versiota:

1. **Dokumentti** — alkuperäinen kuva/PDF (`attachments`)
2. **Raaka extraction** — mitä LLM näki kuvassa, ilman tulkintaa (`raw_text`, `raw_data`)
3. **Tyypitetty datadokumentti** — jäsennetty kuitti kenttineen, jossa arvot voivat olla `unknown`/`null`

### Progressiivinen tiedonkeruu

Agentti ei kysy kaikkea kerralla. Se tekee niin paljon kuin pystyy annetulla datalla, ja kysyy puuttuvat tiedot vasta kun ne estävät etenemisen. Esimerkiksi:

- Kuitti ilman matkatietoja → tallennetaan kuitti, kysytään matka myöhemmin
- Matka ilman ajoneuvoa → lasketaan julkisen liikenteen mukaan, kysytään tarvittaessa
- Käyttäjäprofiili täydentyy ajan myötä keskustelujen perusteella

### ToolContext — auktorisointi jokaisessa tool-kutsussa

Jokainen tool-handler saa `ToolContext`-rakenteen joka injektoidaan backendissä, ei koskaan LLM:n syötteestä:

```rust
pub struct ToolContext {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub inbound_email_id: InboundEmailId,
}
```

Jokainen SQL-kysely rajataan:

```sql
WHERE tenant_id = $ctx.tenant_id AND user_id = $ctx.user_id
```

LLM:n välittämät ID:t (esim. `expense_id`) ovat vain viittauksia — backend varmistaa aina omistajuuden.

> Sähköpostin autentikointi (DKIM/SPF/DMARC ennen tool-suoritusta) suunnitellaan erillisessä issuessa.

### Idempotenssi — Message-ID -deduplaatio

Sähköpostijärjestelmät uudelleenlähettävät viestejä (transientit virheet, SMTP retry). Jokainen mutatoiva operaatio on idempotenssisuojattu:

1. **Sähköpostitaso:** `message_id` tallennetaan unique-constraintilla. Sama viesti ei käsitellä uudelleen.
2. **Rivitaso:** Mutatoivat taulut sisältävät `message_id`-kentän (kommentoitu idempotenssi-käyttöön), jolla estetään duplikaattirivit.

Vastaussähköpostit lähetetään **outbox-patternilla** — tool-mutaatiot ja vastauksen lähetys ovat erillisiä, ja vastaus lähetetään workerilla joka kunnioittaa idempotenssiä.

---

## 2. Liitteiden käsittely

### Nykytila

`email.rs` parsii vain `body_plain`-kentän. MIME-liitteitä ei lueta eikä tallenneta.

### Suunnitelma

#### 2.1 Vastaanotto

`mail-parser`-kirjasto tukee jo MIME-parsintaa. `ParsedEmail`-rakennetta laajennetaan:

```rust
pub struct Attachment {
    pub filename: Option<String>,
    pub content_type: String,      // "image/jpeg", "application/pdf"
    pub content: Vec<u8>,          // raakadata
    pub size: usize,
}

pub struct ParsedEmail {
    // ...nykyiset kentät...
    pub attachments: Vec<Attachment>,
}
```

Tuetut kuvaformaatit asiakkailta — laaja tuki:
- `image/jpeg`, `image/png`, `image/heic`, `image/webp`, `image/tiff`, `image/bmp`
- `application/pdf` — PDF-kuitit ja -laskut

Kuvat konvertoidaan parhaaseen muotoon sen mukaan mitä kukin vision-malli tukee (ei sidottu yhteen malliin).

Hylätyt liitteet (loki + ilmoitus käyttäjälle):
- Liian suuri (> 10 MB)
- Tuntematon tyyppi (esim. .exe, .zip)

#### 2.2 Tallennus

**PostgreSQL + bytea** (MVP-valinta):

```sql
CREATE TABLE attachments (
    id          BIGSERIAL PRIMARY KEY,
    tenant_id   BIGINT NOT NULL REFERENCES tenants(id),
    message_id  TEXT,                   -- sähköpostin Message-ID, idempotenssi
    filename    TEXT,
    content_type TEXT NOT NULL,
    data        BYTEA NOT NULL,
    size_bytes  INTEGER NOT NULL,
    sha256      TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_attachments_tenant ON attachments(tenant_id);
```

**Miksi PostgreSQL eikä tiedostojärjestelmä/S3:**
- Yksinkertaisuus: yksi backup, yksi transaktio
- Kuitit ovat pieniä (tyypillisesti < 1 MB)
- Skaalaus S3:een myöhemmin on suoraviivainen migraatio
- TOAST-kompressio hoitaa ison osan tehokkaasti

#### 2.3 Liitteiden esikäsittely — loopin ulkopuolella

Liitteet käsitellään **ennen agenttista looppia** erillisillä LLM-kutsuilla. Agenttinen loop ei koskaan näe raakoja kuvia — se saa vain tyypitetyt johdannaisdokumentit.

```
Sähköposti saapuu + liitteet
  ↓
[1] Liitteiden tallennus (attachments-taulu)
  ↓
[2] Kuvan esikäsittely:
    - Pienennys (max ~2048px pitkä sivu)
    - EXIF-metadata poistetaan
    - Konversio mallin tukemaan formaattiin
  ↓
[3] Extraction per liite (erillinen LLM-kutsu, ei tool-kutsuja):
    - Sisältöluokittelu (vapaamuotoinen, esim. "receipt", "route_map", "invoice", "screenshot", "other")
    - Tyypitetty johdannaisdokumentti extraction-tuloksesta
  ↓
[4] Agenttinen loop saa:
    - Käyttäjän viestin teksti
    - Johdannaisdokumentit (teksti, ei kuvia)
```

**Extraction-kutsu on rajoitettu:** se vain lukee ja jäsentää dokumentin. Sillä ei ole pääsyä tool-kutsuihin. System promptissa kielletään ohjeiden tulkitseminen dokumentin sisällöstä.

**Jos agentti tarvitsee lisätietoa liitteestä**, se kutsuu työkalua:

- `re_extract_attachment(attachment_id, hint)` — backend tekee uuden extraction-kutsun apupromptilla, kuva ei tule loopin kontekstiin
- `get_attachment_image(attachment_id)` — äärimmäisissä erikoistapauksissa alkuperäinen kuva (lisää kustannuksia)

Tämä ratkaisee:
- **Kustannus:** kuvat eivät koskaan tule loopin kontekstiin eikä uudelleenlähety historiassa
- **Prompt injection:** extraction-kutsu on erillinen ja rajattu, ei tool-kutsuja
- **Tyypitys liitteen mukaan:** kuittikuva vs. Google Maps -screenshot vs. lasku saavat eri extraction-strategian

#### 2.4 Jäsennetty data kuitista

Extraction tuottaa tyypitetyn johdannaisdokumentin. Kentät voivat olla `null`/`unknown`:

```json
{
  "content_type": "receipt",
  "vendor": "Ravintola Kuu",
  "date": "2026-04-25",
  "total_amount": 24.50,
  "currency": "EUR",
  "items": [
    {"description": "Lounas", "amount": 24.50, "vat_rate": 14}
  ],
  "payment_method": "card",
  "raw_text": "RAVINTOLA KUU\n...\nYHT 24,50"
}
```

Reittikartta-extractionin johdannaisdokumentti:

```json
{
  "content_type": "route_map",
  "origin": "Helsinki",
  "destination": "Tampere",
  "distance_km": 178,
  "raw_text": null
}
```

---

## 3. Tietomalli

### Kokonaiskuva

```
tenants (yritykset)
  └── users (käyttäjät)
       ├── user_profiles (taustadata, osoitteet)
       ├── attachments (liitetiedostot)
       │    └── extractions (johdannaisdokumentit)
       ├── receipts (kuitit/tositteet)
       ├── trips (matkat)
       │    ├── expenses (kulurivit)
       │    │    └── receipt_id → receipts
       │    └── per_diems (päivärahat)
       └── expense_reports (matkalaskut)
            └── report_lines → expenses
```

### 3.1 Tenantit ja käyttäjät

```sql
CREATE TABLE tenants (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT NOT NULL,
    domain      TEXT,
    settings    JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE users (
    id          BIGSERIAL PRIMARY KEY,
    tenant_id   BIGINT NOT NULL REFERENCES tenants(id),
    email       TEXT NOT NULL,
    name        TEXT,
    role        TEXT NOT NULL DEFAULT 'user'
        CHECK (role IN ('user', 'admin', 'approver')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(tenant_id, email)
);

CREATE INDEX idx_users_tenant ON users(tenant_id);
```

> Huom: Käyttäjähallinta (#22) suunnitellaan erikseen. Tässä vain vähimmäisviittaus.

### 3.2 Liite-extractionit

```sql
CREATE TABLE extractions (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       BIGINT NOT NULL REFERENCES tenants(id),
    user_id         BIGINT NOT NULL REFERENCES users(id),
    attachment_id   BIGINT NOT NULL REFERENCES attachments(id),
    message_id      TEXT,               -- idempotenssi

    -- Extraction-tulos
    content_type    TEXT,               -- vapaamuotoinen: "receipt", "route_map", "invoice", "screenshot", ...
    raw_text        TEXT,               -- OCR:n tuottama raaka teksti
    extracted_data  JSONB NOT NULL,     -- tyypitetty johdannaisdokumentti
    model           TEXT NOT NULL,      -- mikä malli teki extractionin
    confidence      NUMERIC(4,3) CHECK (confidence >= 0 AND confidence <= 1),

    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_extractions_attachment ON extractions(attachment_id);
CREATE INDEX idx_extractions_tenant_user ON extractions(tenant_id, user_id);
```

### 3.3 Kuitit (receipts)

```sql
CREATE TABLE receipts (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       BIGINT NOT NULL REFERENCES tenants(id),
    user_id         BIGINT NOT NULL REFERENCES users(id),
    extraction_id   BIGINT REFERENCES extractions(id),
    message_id      TEXT,               -- idempotenssi

    -- Raakadata extractionista
    raw_text        TEXT,
    raw_data        JSONB,

    -- Jäsennetty data (voi olla null/unknown)
    vendor          TEXT,
    receipt_date    DATE,
    total_amount    NUMERIC(12,2),
    currency        TEXT NOT NULL DEFAULT 'EUR'
        CHECK (currency ~ '^[A-Z]{3}$'),
    items           JSONB,              -- [{description, amount, vat_rate, vat_amount}]
    payment_method  TEXT
        CHECK (payment_method IS NULL OR payment_method IN ('card', 'cash', 'invoice')),

    -- Luokittelu
    category        TEXT,               -- food, accommodation, transport, fuel, parking, other
    confidence      NUMERIC(4,3) CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),

    -- Metadata
    source          TEXT NOT NULL DEFAULT 'email'
        CHECK (source IN ('email', 'web', 'api')),
    status          TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'confirmed', 'rejected')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_receipts_tenant_user ON receipts(tenant_id, user_id);
CREATE INDEX idx_receipts_tenant_user_date ON receipts(tenant_id, user_id, receipt_date);
```

**Suunnittelupäätökset:**
- `raw_text` + `raw_data` säilytetään aina → audit trail (kolmikerroksisen mallin kerros 2)
- `items` on JSONB koska rivien määrä vaihtelee (yksi lounas vs. hotellilasku monen rivin kanssa)
- `category` on agentin ehdotus, `status` on käyttäjän vahvistus
- `confidence` auttaa päättämään pitääkö varmistaa käyttäjältä
- ALV-kenttä voi olla `null` (unknown) — ei oleteta kantaa jos kuitissa ei ole erittelyä
- Liite-kuitit -kardinaliteetti: 1:N (yksi extraction → yksi kuitti, yksi liite → monta extractionia). Agenttinen loop yhdistää metatietojen perusteella.

### 3.4 Matkat (trips)

```sql
CREATE TABLE trips (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       BIGINT NOT NULL REFERENCES tenants(id),
    user_id         BIGINT NOT NULL REFERENCES users(id),

    -- Matkan tiedot
    description     TEXT,
    purpose         TEXT
        CHECK (purpose IS NULL OR purpose IN ('business', 'training', 'conference', 'other')),
    destination     TEXT,
    origin          TEXT,

    -- Ajankohta (kellonajat päivärahalaskentaa varten)
    start_date      DATE NOT NULL,
    end_date        DATE NOT NULL,
    start_at        TIMESTAMPTZ,        -- tarkka lähtöaika (päiväraha)
    end_at          TIMESTAMPTZ,        -- tarkka paluuaika (päiväraha)
    timezone        TEXT NOT NULL DEFAULT 'Europe/Helsinki',

    -- Kuljetus
    transport_mode  TEXT
        CHECK (transport_mode IS NULL OR transport_mode IN ('car', 'public', 'flight', 'train', 'bus')),
    distance_km     NUMERIC(8,1),
    route_data      JSONB,

    -- Tila
    status          TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'complete', 'reported')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CHECK (start_date <= end_date)
);

CREATE INDEX idx_trips_tenant_user ON trips(tenant_id, user_id);
CREATE INDEX idx_trips_tenant_user_dates ON trips(tenant_id, user_id, start_date, end_date);
```

### 3.5 Kulurivit (expenses)

```sql
CREATE TABLE expenses (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       BIGINT NOT NULL REFERENCES tenants(id),
    user_id         BIGINT NOT NULL REFERENCES users(id),
    trip_id         BIGINT REFERENCES trips(id),
    receipt_id      BIGINT REFERENCES receipts(id),
    message_id      TEXT,               -- idempotenssi

    -- Kulurivin tiedot
    description     TEXT NOT NULL,
    expense_date    DATE NOT NULL,
    amount          NUMERIC(12,2) NOT NULL CHECK (amount >= 0),
    currency        TEXT NOT NULL DEFAULT 'EUR'
        CHECK (currency ~ '^[A-Z]{3}$'),
    category        TEXT NOT NULL,

    -- ALV (voi olla null = unknown)
    vat_rate        NUMERIC(5,2),
    vat_amount      NUMERIC(12,2),
    amount_excl_vat NUMERIC(12,2),

    -- Korvauslaji
    expense_type    TEXT NOT NULL DEFAULT 'receipt'
        CHECK (expense_type IN ('receipt', 'mileage', 'per_diem', 'meal_allowance')),

    -- Km-korvaus-spesifinen (backend laskee amount)
    mileage_km      NUMERIC(8,1),
    mileage_rate    NUMERIC(6,4),       -- €/km
    passengers      INTEGER,
    tax_rule_set_id BIGINT REFERENCES tax_rates(id),

    -- Tila
    status          TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'confirmed', 'rejected')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Km-korvauksen kenttävalidointi
    CHECK (
        expense_type <> 'mileage'
        OR (mileage_km IS NOT NULL AND mileage_rate IS NOT NULL)
    )
);

CREATE INDEX idx_expenses_tenant_user ON expenses(tenant_id, user_id);
CREATE INDEX idx_expenses_tenant_user_date ON expenses(tenant_id, user_id, expense_date);
CREATE INDEX idx_expenses_trip ON expenses(trip_id);
```

### 3.6 Päivärahat (per_diems)

Päiväraha on 1:1-laajennus `expenses`-tauluun (`expense_type = 'per_diem'`):

```sql
CREATE TABLE per_diem_details (
    expense_id      BIGINT PRIMARY KEY REFERENCES expenses(id),
    trip_id         BIGINT NOT NULL REFERENCES trips(id),

    -- Päivärahatiedot
    per_diem_date   DATE NOT NULL,
    destination     TEXT NOT NULL,
    country         TEXT NOT NULL DEFAULT 'FI',

    -- Laskenta (backend laskee)
    type            TEXT NOT NULL
        CHECK (type IN ('full_day', 'partial_day', 'meal_allowance')),
    base_rate       NUMERIC(8,2) NOT NULL,
    deductions      JSONB NOT NULL DEFAULT '[]',
    final_amount    NUMERIC(8,2) NOT NULL,
    tax_rule_set_id BIGINT REFERENCES tax_rates(id),

    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### 3.7 Matkalaskut (expense_reports)

```sql
CREATE TABLE expense_reports (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       BIGINT NOT NULL REFERENCES tenants(id),
    user_id         BIGINT NOT NULL REFERENCES users(id),

    -- Laskun tiedot
    title           TEXT NOT NULL,
    period_start    DATE,
    period_end      DATE,
    total_amount    NUMERIC(12,2),      -- lasketaan draft-tilassa, jäädytetään submitted-tilassa

    -- Tila (state machine)
    status          TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'review', 'submitted', 'approved', 'rejected', 'paid')),

    -- Hyväksyntä
    approver_id     BIGINT REFERENCES users(id),
    approved_at     TIMESTAMPTZ,
    rejection_reason TEXT,

    -- Integraatiot
    external_id     TEXT,
    exported_at     TIMESTAMPTZ,

    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE report_lines (
    id              BIGSERIAL PRIMARY KEY,
    report_id       BIGINT NOT NULL REFERENCES expense_reports(id),
    expense_id      BIGINT NOT NULL REFERENCES expenses(id),
    line_number     INTEGER NOT NULL,
    UNIQUE(report_id, expense_id)
);

CREATE INDEX idx_reports_tenant_user ON expense_reports(tenant_id, user_id);
CREATE INDEX idx_reports_tenant_user_status ON expense_reports(tenant_id, user_id, status);
```

### 3.8 Käyttäjäprofiili

```sql
CREATE TABLE user_profiles (
    id              BIGSERIAL PRIMARY KEY,
    user_id         BIGINT NOT NULL REFERENCES users(id) UNIQUE,

    -- Osoitteet
    home_address    TEXT,
    work_address    TEXT,
    home_lat        NUMERIC(9,6),
    home_lng        NUMERIC(9,6),
    work_lat        NUMERIC(9,6),
    work_lng        NUMERIC(9,6),

    -- Oletus-asetukset
    default_transport TEXT
        CHECK (default_transport IS NULL OR default_transport IN ('car', 'public', 'train', 'bus', 'flight')),
    default_vehicle   TEXT
        CHECK (default_vehicle IS NULL OR default_vehicle IN ('own_car', 'company_car')),

    -- Opitut preferenssit (agentti täydentää)
    preferences     JSONB NOT NULL DEFAULT '{}',

    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

> Pankkitili kerätään erillisessä turvallisessa käyttöliittymässä (web-UI), ei LLM:n kautta. Ei kuulu tähän tietomalliin.

### 3.9 Verohallinnon korvausmäärät

```sql
CREATE TABLE tax_rates (
    id              BIGSERIAL PRIMARY KEY,
    rate_type       TEXT NOT NULL
        CHECK (rate_type IN ('mileage', 'per_diem_domestic', 'per_diem_abroad', 'meal_allowance')),
    country         TEXT NOT NULL DEFAULT 'FI',
    params          JSONB NOT NULL,
    valid_from      DATE NOT NULL,
    valid_until     DATE,
    UNIQUE(rate_type, country, valid_from)
);
```

> `UNIQUE(rate_type, country, valid_from)` mahdollistaa vuoden sisäiset muutokset (esim. km-korvauksen muutos kesken vuoden).

### ER-kaavio (tekstimuodossa)

```
tenant ──1:N── user ──1:1── user_profile
                │
                ├──1:N── attachment ──1:N── extraction
                │                              │
                ├──1:N── receipt ──N:1── extraction (optional)
                │            │
                ├──1:N── trip
                │            │
                ├──1:N── expense ──N:1── trip (optional)
                │            │       └── receipt (optional)
                │            │       └── per_diem_details (1:1, optional)
                │            │
                └──1:N── expense_report
                             └──1:N── report_line ──N:1── expense
```

---

## 4. Raakadata → kirjanpitodata

### Pipeline

```
Liite (kuva/PDF)
  ↓
[1] Esikäsittely (kuvan pienennys, formaattikonversio)
  ↓
[2] Extraction (erillinen LLM-kutsu, loopin ulkopuolella)
  ↓ johdannaisdokumentti (content_type, vendor, date, amount, items, ...)
[3] Agenttinen loop: päättelee rakenne
  ↓ mihin matkaan kuuluu, luokittelu, ryhmittely
[4] Backend: validointi ja laskenta
  ↓ ALV-erittely, korvauslaji, tarkistukset
[5] Tallennus
  ↓ receipt + expense (status=draft, ihminen tarkistaa)
[6] Loppukäsittely: deterministinen laskenta kokonaisuudelle
  ↓ päivärahat, km-korvaukset, kokonaissummat
```

### ALV-käsittely

Suomen ALV-kannat (2026):
- 25,5 % — yleinen
- 14 % — ruoka, ravintola
- 10 % — majoitus, henkilöliikenne, kirjat
- 0 % — terveydenhoito, koulutus

Extraction poimii ALV:n kuitista jos se on eriteltynä. **Jos kuitissa ei ole ALV-erittelyä, arvo jää `null` (unknown)** — ei oleteta kantaa kategorian perusteella. Ihminen tai hyväksyjä vahvistaa.

### Verohallinnon korvausmäärät

Korvausmäärät tallennetaan `tax_rates`-konfiguraatiotauluun. Backend laskee deterministisesti. Jokaiseen laskentaan tallennetaan viittaus käytettyyn sääntöön (`tax_rule_set_id`).

---

## 5. Matkan tunnistaminen

### Tunnistuslogiikka

Agentti tunnistaa matkan seuraavilla signaaleilla:

1. **Eksplisiittinen:** Käyttäjä sanoo "olin työmatkalla Tampereella"
2. **Kuittianalyysi:** Hotelli + ravintola + kuljetus samasta kaupungista, samoilta päiviltä
3. **Kalenteri (myöhempi vaihe):** Tapahtuma toisella paikkakunnalla

### Tunnistusalgoritmi

```
JOKAISELLE uudelle kuitille:
  1. Onko sijainti eri kuin käyttäjän koti/työ?
  2. Onko samalta alueelta ja samoilta päiviltä jo kuitteja?
     → JOS kyllä: ehdota liittämistä olemassa olevaan matkaan
     → JOS ei: ehdota uuden matkan luomista
  3. Onko kuitti majoitus (hotelli)?
     → Vahva signaali työmatkasta
  4. Onko kuitti kuljetus (juna, lento, taksi)?
     → Yhdistä mahdolliseen matkaan päivämäärän perusteella
```

### Matkan automaattinen luominen

Agentti ei luo matkaa automaattisesti — se **ehdottaa** ja pyytää vahvistuksen:

> "Näyttää siltä, että olit työmatkalla Tampereella 24.–25.4. Luonko matkan ja liitän nämä kuitit siihen?"

Poikkeus: jos käyttäjä on eksplisiittisesti kertonut matkasta, agentti luo sen suoraan.

---

## 6. Käyttäjäprofiili

### Progressiivinen tiedonkeruu

Agentti kerää tietoa luontevasti keskustelujen lomassa. Ei "setup wizard" -kyselyä.

**Ensimmäisellä interaktiolla:**
- Nimi (sähköpostista tai kysymällä)
- Yritys/tenant (domain-matching tai kysymällä)

**Ensimmäisellä matkalaskulla:**
- Kotiosoite (km-laskentaa varten): "Mistä lähdit matkaan?"
- Kulkuneuvo: "Millä matkustit?" → tallennetaan oletukseksi

**Ajan myötä:**
- Tyypilliset reitit (Helsinki–Tampere toistuu → ehdotetaan suoraan)
- Majoitustottumukset
- Ruokailutottumukset (ateriakorvausvähennykset)

### Tallennusstrategia

Strukturoitu data (`user_profiles`-taulu) + vapaamuotoinen (`preferences` JSONB):

```json
{
  "frequent_routes": [
    {"from": "Helsinki", "to": "Tampere", "distance_km": 178, "count": 5}
  ],
  "typical_transport": "car",
  "typical_meal_budget": 15.00,
  "notes": "Käyttää usein junaa Tampereelle, autoa muualle"
}
```

---

## 7. Geolokalisaatio ja etäisyydet

### MVP-ratkaisu

**Käyttäjän ilmoittama etäisyys** + agentin järkevyystarkistus:

1. Käyttäjä kertoo matkan (esim. "ajoin Helsinki–Tampere")
2. Agentti kutsuu `calculate_distance`-työkalua
3. Työkalu geokoodaa osoitteet ja laskee reitin
4. Jos käyttäjä ilmoittaa poikkeavan kilometrimäärän, agentti tarkistaa

### Geokoodaus ja reititys

**Suositus: OpenRouteService (avoimen lähdekoodin)**
- Ilmainen self-hosted tai rajattu ilmaistaso API:lla
- Perustuu OpenStreetMap-dataan
- Tukee autoreititystä Suomessa hyvin

**Vaihtoehto: Google Maps API**
- Tarkkuus, erityisesti osoitteet
- Hinta: $5/1000 reititystä — hyväksyttävä MVP:ssä

### Multi-stop-matkat

Käyttäjä voi ilmoittaa välipysähdyksiä:

> "Ajoin Helsinki → Hämeenlinna (tapaaminen) → Tampere (hotelli)"

Agentti laskee kunkin osuuden erikseen ja summaa kokonaiskilometrit.

---

## 8. Matkalaskuluonnos

### Tilakone

```
         ┌──────────────────────┐
         │                      ▼
draft ──→ review ──→ submitted ──→ approved ──→ paid
                        │                        ▲
                        └──→ rejected ──→ draft ─┘
```

`total_amount` lasketaan `draft`-tilassa dynaamisesti. Se jäädytetään kun status muuttuu `submitted`:ksi.

Tilasiirtymät tapahtuvat backendissä, ei LLM:n päätöksellä.

### Luonnoksen muokkaus sähköpostilla

Käyttäjä voi muokata luonnosta luonnollisella kielellä:

- "Muuta lounaan summaksi 16,50€"
- "Poista parkkikuitti"
- "Lisää taksimatka lentokentältä, 35€"
- "Näytä luonnos"

Agentti tulkitsee pyynnön ja kutsuu oikeaa työkalua.

> Sähköpostin käsittelyssä agenttinen loop saa vain kyseisen viestin sisällön, ei quote-tekstiä. Tämä estää vanhojen viestien laukaisemasta toimintoja.

### Luonnoksen esitysmuoto sähköpostissa

Agentti muotoilee luonnoksen taulukoksi:

```
Matka: Tampere 24.–25.4.2026

| #  | Kulu                    | Summa   | ALV    |
|----|-------------------------|---------|--------|
| 1  | Juna Helsinki–Tampere   | 34,50 € | 10 %   |
| 2  | Hotelli Scandic         | 129,00 €| 10 %   |
| 3  | Lounas Ravintola Kuu    | 24,50 € | 14 %   |
| 4  | Päiväraha 24.4. (osa)   | 24,00 € | —      |
| 5  | Päiväraha 25.4. (koko)  | 53,00 € | —      |
|    | **Yhteensä**            |**265,00 €**|     |
```

---

## 9. Agentin työkalut

### Työkalu-arkkitehtuuri

Jokainen tool-handler saa `ToolContext`-rakenteen (§1). LLM:n välittämät ID:t validoidaan aina `tenant_id + user_id` -rajauksella.

Backend tarjoaa agentille myös **laskentavälineitä** — tool-kutsuja joilla agentti voi laskea yksittäisiä asioita keskustelussa (esim. km-korvauksen arvio). Nämä ovat erillisiä kokonaisuuden **loppukäsittelystä**, jossa deterministinen laskentaprosessi viimeistelee koko matkalaskun.

Työkalut ovat tavoitetasolla — tarkka tool-surface muotoutuu toteutuksessa. Erityisesti tool-kutsujen **transaktionaalisuus** (operaatioiden atominen suoritus) suunnitellaan erillisessä issuessa (#27).

### 9.1 Kuittien tallennus ja hallinta

- `save_receipt` — tallenna kuittidata extractionin perusteella
- `update_receipt` — muokkaa kuitin tietoja (käyttäjän korjaus)
- `list_receipts` — listaa ja suodata käyttäjän kuitteja

### 9.2 Kulurivit

- `add_expense` — lisää kulurivi (kuittipohjainen, manuaalinen)
- `update_expense` — muokkaa kuluriviä
- `set_expense_status` — muuta kulun tilaa (draft, confirmed, excluded)
- `list_expenses` — listaa käyttäjän kulurivit

### 9.3 Matkat

- `create_trip` — luo uusi matka
- `update_trip` — muokkaa matkan tietoja
- `link_expense_to_trip` — liitä kulu matkaan
- `list_trips` — listaa käyttäjän matkat

### 9.4 Laskenta (agentin laskentavälineet)

- `calculate_distance` — geokoodaa osoitteet ja laske reitti
- `calculate_mileage` — laske km-korvaus Verohallinnon säännöillä
- `calculate_per_diem` — laske päiväraha matkan perusteella

Nämä ovat **informatiivisia** — ne palauttavat laskentatuloksen agentille, joka voi kertoa sen käyttäjälle. Lopullinen tallennus tapahtuu loppukäsittelyssä.

**`calculate_per_diem` palauttaa `needs_more_info`** jos kellonajat puuttuvat — ei hyväksy Clauden keksimiä aikoja:

```json
{
  "ok": false,
  "code": "missing_trip_times",
  "missing": ["start_at", "end_at"],
  "user_message": "Tarvitsen lähtö- ja paluuajan päivärahan laskemiseen."
}
```

### 9.5 Liitteet

- `re_extract_attachment` — pyydä uusi extraction apupromptilla (kuva ei tule loopin kontekstiin)
- `get_attachment_image` — hae alkuperäinen kuva (erikoistapaukset, lisää kustannuksia)

### 9.6 Matkalaskut

- `get_draft_summary` — kevyt luonnosyhteenveto kuluista
- `create_report` — luo virallinen matkalaskuluonnos (myöhempi vaihe)
- `submit_report` — lähetä hyväksyttäväksi (myöhempi vaihe)
- `list_reports` — listaa matkalaskut

### 9.7 Käyttäjäprofiili

- `get_user_context` — hae käyttäjän profiili ja oletukset (ei palauta pankkitiliä)
- `update_user_preferences` — päivitä turvallisia kenttiä (osoitteet, oletuskulkuneuvo)

---

## 10. Agenttinen silmukka

### Nykyinen arkkitehtuuri (agent.rs)

```
email → load history → build request → API call → extract text → reply
```

Yksi API-kutsu, ei tool_use-käsittelyä.

### Uusi arkkitehtuuri

```
sähköposti saapuu
  ↓
[1] Liitteiden esikäsittely + extraction (loopin ulkopuolella)
  ↓ johdannaisdokumentit
[2] Message-ID -deduplaatio (onko jo käsitelty?)
  ↓
[3] Käyttäjän tunnistus → ToolContext { tenant_id, user_id }
  ↓
[4] Agenttinen loop:
    load history → build request (tools + johdannaisdokumentit)
      → API call
      → WHILE stop_reason == ToolUse:
          → suorita tool_use-kutsut (ToolContext injektoitu)
          → lähetä tool_result takaisin
          → API call (jatko)
      → extract text
  ↓
[5] Vastaus outbox-jonoon → worker lähettää SMTP:llä
```

### Keskusteluhistorian tallennus

`content_json JSONB` korvaa nykyisen `content TEXT` -kentän. Tukee kaikkia sisältötyyppejä (teksti, tool_use, tool_result).

**Historiaan ei tallenneta kuvia** — extractionin jälkeen kuva korvataan tekstiviittauksella:

```json
{"type": "text", "text": "[Liite käsitelty: extraction_id=42, Ravintola Kuu 24,50€]"}
```

**Historian katkaisu on paritustietoinen:** `tool_use`-blokkia ei saa jättää ilman vastaavaa `tool_result`-blokkia.

**Tuntemattomat ContentBlock-tyypit:** Jos API palauttaa tuntemattoman blokkityypin (esim. uusi Anthropic-tyyppi), se logitetaan warn-tasolla, tuottaa alertin ylläpidolle, ja säilytetään raakana JSON:na round-trippausta varten. Käsittely ei keskeydy.

### Turvarajat

- **Max iterations:** 10 tool_use-kierrosta per sähköposti
- **Max tokens per turn:** 4096
- **Timeout:** 120s per API-kutsu, 300s koko silmukalle
- **Kustannuskatto:** Seurataan per käyttäjä/päivä (ks. #14 opex-hallinta)

---

## 11. Email-First UX -esimerkit

### Esimerkki 1: Ensimmäinen kuitti

**Käyttäjä lähettää:** Sähköposti aiheella "Lounas" ja liitteenä kuittikuva.

**Järjestelmä:**

1. Vastaanottaa sähköpostin liitteineen
2. Tallentaa kuittikuvan (`attachments`)
3. **Esikäsittely (loopin ulkopuolella):** pienentää kuvan, extraction-kutsu → johdannaisdokumentti: `{content_type: "receipt", vendor: "Ravintola Kuu", date: "2026-04-25", total_amount: 24.50, ...}`
4. **Agenttinen loop alkaa** — saa johdannaisdokumentin tekstinä, ei kuvaa
5. Agentti kutsuu `save_receipt(...)` johdannaisdokumentin perusteella
6. Agentti vastaa:

> Kuitti tallennettu!
>
> **Lounas Ravintola Kuu** — 24,50 €
> Päivämäärä: 25.4.2026
>
> Kuuluuko tämä johonkin työmatkaan? Jos kyllä, kerro matkan kohde ja päivämäärät.

### Esimerkki 2: Kokonainen matka

**Käyttäjä:** "Olin Tampereella asiakaspalaverissa 24.–25.4. Ajoin omalla autolla. Tässä hotellikuitti ja lounas."

Liitteet: 2 kuittikuvaa.

**Järjestelmä:**

1. **Esikäsittely:** 2 extractionia → 2 johdannaisdokumenttia (hotelli + lounas)
2. **Agenttinen loop:** saa viestin + johdannaisdokumentit
3. Agentti päättelee rakenteen: tämä on matka, kuuluu yhteen
4. Tool-kutsut: `create_trip(...)`, `save_receipt(...)` × 2, `add_expense(...)` × 2
5. Agentti kutsuu `calculate_mileage(356)` → laskentaväline palauttaa 106,80€
6. Agentti kertoo tuloksen, kysyy kellonajat päivärahalaskentaan

> Matka Tampereelle tallennettu! Tässä yhteenveto:
>
> | #  | Kulu                         | Summa    |
> |----|------------------------------|----------|
> | 1  | Hotelli Scandic Tampere      | 129,00 € |
> | 2  | Lounas Ravintola Kuu         | 24,50 €  |
> | 3  | Km-korvaus 356 km            | 106,80 € |
> |    | **Yhteensä**                 | **260,30 €** |
>
> Mihin aikaan lähdit matkaan ja palasit? Tarvitsen tarkat kellonajat päivärahan laskemiseen.

### Esimerkki 3: Luonnoksen muokkaus

**Käyttäjä:** "Lounaan hinta oli oikeasti 16,50€. Ja poista hotellikuitti, se menee firman kortille."

**Agentti:**

1. `update_expense(expense_id=2, amount=16.50)`
2. `set_expense_status(expense_id=1, status="excluded")`

> Päivitetty:
> - Lounas: 24,50 € → 16,50 €
> - Hotelli Scandic merkitty pois laskulta
>
> **Uusi yhteenveto:**
>
> | #  | Kulu                         | Summa    |
> |----|------------------------------|----------|
> | 1  | Lounas Ravintola Kuu         | 16,50 €  |
> | 2  | Km-korvaus 356 km            | 106,80 € |
> |    | **Yhteensä**                 | **123,30 €** |

---

## 12. MVP-priorisointi

### Vaihe 1: Kuittiarkisto (agenttinen loop + kuittien tallennus + luonnosyhteenveto)

**Tavoite:** Kuittiarkisto jossa agentti vastaanottaa kuitteja, tekee extractionin, tallentaa tiedot, ja pystyy listaamaan sekä koostamaan yhteenvedon.

**Turvasubstraatti (toteutetaan osana vaihetta 1):**
- `ToolContext { tenant_id, user_id }` jokaiseen tool-handleriin
- Message-ID -deduplaatio
- Outbox-pattern vastaussähköposteille
- Liitteiden esikäsittely loopin ulkopuolella
- `content_json` -historian tallennus (ei kuvia historiaan)

**Tietokanta:**
- `tenants`, `users`
- `attachments`, `extractions`
- `receipts`, `expenses`

**Työkalut:**
- `save_receipt`, `update_receipt`, `list_receipts`
- `add_expense`, `update_expense`, `set_expense_status`, `list_expenses`
- `get_draft_summary`
- `get_user_context`, `update_user_preferences`
- `re_extract_attachment`

**Ei vielä:**
- Matkat, päivärahat, km-korvaukset
- Viralliset matkalaskut, hyväksyntäkierto
- Geolokalisaatio

**Liittyy issueisiin:** #2, #7, #8

### Vaihe 2: Matkat ja laskelmat

**Tavoite:** Agentti tunnistaa matkat, laskee korvaukset, ja tarjoaa laskentavälineitä.

Toteutetaan:
- `trips`, `per_diem_details`, `tax_rates`
- `create_trip`, `update_trip`, `link_expense_to_trip`, `list_trips`
- `calculate_per_diem`, `calculate_mileage`
- Loppukäsittely: deterministinen laskenta kokonaisuudelle

**Liittyy issueisiin:** #9, #20

### Vaihe 3: Matkalaskut ja hyväksyntä

**Tavoite:** Viralliset matkalaskut, hyväksyntäkierto.

Toteutetaan:
- `expense_reports`, `report_lines`
- `create_report`, `get_report_draft`, `submit_report`, `list_reports`
- Tilakoneen toteutus backendissä
- Hyväksyntäreititys (#21)

**Liittyy issueisiin:** #6, #21

### Vaihe 4: Geolokalisaatio ja integraatiot

**Tavoite:** Automatisoi km-laskenta ja vie kirjanpitoon.

Toteutetaan:
- `calculate_distance` (OpenRouteService/Google Maps)
- Google Calendar -integraatio (#16)
- Netvisor/Procountor-vienti (#18, #19)

### Yhteenveto

```
Vaihe 1 ──→ Vaihe 2 ──→ Vaihe 3 ──→ Vaihe 4
Kuittiarkisto Matkat      Laskut      Integraatiot
(#2,#7,#8)   (#9,#20)    (#6,#21)    (#16,#18,#19)
```

Jokainen vaihe tuottaa itsenäisesti käyttökelpoisen kokonaisuuden. Vaihe 1 on kuittiarkisto jossa agentti tallentaa, listaa ja koostaa. Vaihe 2 tuo matkojen tunnistamisen ja korvauslaskennan. Vaihe 3 virallistaa raportoinnin. Vaihe 4 yhdistää ulkoisiin järjestelmiin.
