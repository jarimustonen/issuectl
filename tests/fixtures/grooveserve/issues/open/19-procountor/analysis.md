# Procountor-integraatio — analyysi

## Yhteenveto

Procountor tarjoaa modernin REST/JSON API:n matkalaskujen (travel invoices) luontiin. API tukee OAuth 2.0 M2M -autentikointia, joka sopii backend-integraatioon. Matkalaskut luodaan `TRAVEL_INVOICE`-tyyppisinä laskuina standardin `/invoices`-endpointin kautta. Rust-toteutus on suoraviivainen — valmiita kirjastoja ei ole, mutta `reqwest` + `serde` riittävät.

## API-yleiskatsaus

### Autentikointi

Procountor tukee kahta OAuth 2.0 -flowta:

| Flow | Käyttötarkoitus | Grooveservelle |
|------|----------------|----------------|
| Authorization Code | Interaktiivinen kirjautuminen | Onboarding (asiakas yhdistää Procountor-tilinsä) |
| Client Credentials / M2M (API Key) | Backend-integraatiot | **Pääasiallinen käyttö** — matkalaskujen vienti |

**M2M-flow:**
1. Asiakas luo API-avaimen Procountorissa (sidottu käyttäjään + yritykseen + client-sovellukseen)
2. Grooveserve vaihtaa API-avaimen access tokeniin: `POST https://api.procountor.com/api/oauth/token`
3. Access token (JWT, HMAC SHA-256) voimassa 1 tunti
4. Refresh token voimassa 6 kk, max 100 per client/yritys/käyttäjä

**Onboarding-flow:** Asiakkaan Procountor-yhdistäminen vaatii kertaluonteisen Authorization Code -flownn, jossa asiakas kirjautuu Procountoriin ja valitsee yrityksen. Tämä voidaan toteuttaa web-käyttöliittymässä myöhemmin.

### Matkalaskuendpointit

Procountor käyttää yhteistä `/invoices`-endpointia kaikille laskutyypeille. Matkalaskut erotetaan `type: "TRAVEL_INVOICE"` -kentällä.

**Keskeiset endpointit:**

| Endpoint | Käyttö |
|----------|--------|
| `POST /invoices` | Matkalaskun luonti |
| `GET /invoices/{ids}` | Laskujen haku (max 200 kerralla) |
| `PUT /invoices/{id}` | Laskun päivitys |
| `POST /attachments` | Kuittiliitteet (multipart upload) |
| `PUT /invoices/{id}/approve` | Hyväksyntä |
| `GET /invoices/{id}/comments` | Kommentit |

**Webhookit:** Procountor tukee push-notifikaatioita laskutapahtumista — hyödyllinen hyväksyntäkierron seurantaan.

### Matkalaskun tietomalli

```
TRAVEL_INVOICE
├── counterParty          # Matkustaja (nimi, hetu/y-tunnus, pankkitili)
├── travelInformationItems[]
│   ├── departure         # Lähtöaika
│   ├── arrival           # Paluuaika
│   ├── places            # Kohteet
│   └── purpose           # Matkan tarkoitus
├── invoiceRows[]         # Rivit (päivärahat, km-korvaukset, kulut)
│   ├── product           # Tuotenimi
│   ├── productCode       # Tuotekoodi
│   ├── quantity           # Määrä
│   ├── unit              # KILOMETER, DAY, FULL_DAY, jne.
│   ├── unitPrice         # Yksikköhinta
│   ├── vatPercent        # ALV-%
│   ├── startDate/endDate # Jakso
│   └── comment           # Lisätieto
├── paymentInfo           # Maksutiedot (EUR, eräpäivä, pankkitili)
├── attachments[]         # Kuittiliitteet
└── invoiceApprovalInformation  # Hyväksyjät ja tarkastajat
```

**Statuskierto:** `UNFINISHED` → `VERIFIED` → `APPROVED` → `PAYMENT_QUEUED` → `PAID`

Päivärahat ja km-korvaukset mallinnetaan tavallisina laskuriveinä oikeilla tuotekoodeilla, määrillä (päivät/kilometrit) ja yksikköhinnoilla (Verohallinnon viralliset korvausmäärät).

## Vertailu: Procountor vs Netvisor

| Ominaisuus | Procountor | Netvisor |
|-----------|-----------|----------|
| **API-formaatti** | REST + JSON | REST + XML |
| **Autentikointi** | OAuth 2.0 (M2M / Auth Code) | Partner ID + asiakasavain + allekirjoitus |
| **Matkalaskutyyppi** | Natiivi `TRAVEL_INVOICE` | Ostolaskut matkakenttien kanssa |
| **API-spesifikaatio** | OpenAPI/Swagger saatavilla | XML-skeemadokumentaatio |
| **Testiympäristö** | Ilmainen PTS-ympäristö | Sandbox saatavilla |
| **Rate limit** | 60 req/s (prod), 90 req/min (test) | Ei hyvin dokumentoitu |
| **Sertifiointi** | Vaaditaan (12 tarkistuspistettä) | Kumppanisopimus vaaditaan |
| **Rust-kirjastot** | Ei ole | Ei ole |
| **Dokumentaation laatu** | Hyvä, OpenAPI-spec | Kohtalainen, XML-esimerkit |

**Johtopäätös:** Procountorin API on modernimpi ja helpompi integroida (JSON, OAuth2, OpenAPI). Netvisorin XML-pohjainen API vaatii enemmän työtä. Molemmat vaativat kumppanisopimuksen tuotantokäyttöön.

## Rust-toteutuksen suunnitelma

### Kirjastovalinnat

| Tarve | Kirjasto | Perustelu |
|-------|---------|-----------|
| HTTP-client | `reqwest` | Async, TLS, jo käytössä ekosysteemissä |
| JSON-serialisointi | `serde` + `serde_json` | De facto standardi |
| OAuth 2.0 | `oauth2` crate tai oma | M2M-flow on yksinkertainen (token endpoint + refresh) |
| Multipart upload | `reqwest::multipart` | Kuittiliitteet |
| Virhekäsittely | `thiserror` | Tyypitetyt virheet |

### Arkkitehtuuri

```
services/
  procountor/          # Uusi service tai kirjasto
    src/
      lib.rs           # Public API
      client.rs        # HTTP-client, autentikointi, token-hallinta
      models.rs        # Procountor-tietomallit (serde-struktit)
      invoices.rs      # Matkalaskujen luonti ja haku
      attachments.rs   # Liitetiedostot
      error.rs         # Virhetyypit
```

**Vaihtoehto:** Jos Netvisor-integraatio tulee myös, yhteinen trait-rajapinta:

```rust
#[async_trait]
trait AccountingExport {
    async fn create_travel_invoice(&self, invoice: &TravelInvoice) -> Result<ExportResult>;
    async fn attach_receipt(&self, invoice_id: &str, receipt: &Receipt) -> Result<()>;
    async fn get_invoice_status(&self, invoice_id: &str) -> Result<InvoiceStatus>;
}
```

Tämä mahdollistaa asiakaskohtaisen kirjanpitojärjestelmän valinnan ilman bisneslogiikan muutoksia.

### Tietomallit (esimerkkejä)

```rust
#[derive(Serialize)]
pub struct CreateTravelInvoice {
    #[serde(rename = "type")]
    pub invoice_type: String,  // "TRAVEL_INVOICE"
    pub counter_party: CounterParty,
    pub travel_information_items: Vec<TravelInformationItem>,
    pub invoice_rows: Vec<InvoiceRow>,
    pub payment_info: PaymentInfo,
}

#[derive(Serialize)]
pub struct TravelInformationItem {
    pub departure: String,   // ISO 8601
    pub arrival: String,
    pub places: String,
    pub purpose: String,
}

#[derive(Serialize)]
pub struct InvoiceRow {
    pub product: String,
    pub product_code: Option<String>,
    pub quantity: f64,
    pub unit: String,        // "KILOMETER", "DAY", "FULL_DAY"
    pub unit_price: f64,
    pub vat_percent: f64,
    pub comment: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}
```

**Huom:** Tarkemmat tyypit (enum Unit, Decimal-tyypit hinnoille) kannattaa määritellä toteutusvaiheessa OpenAPI-specin perusteella.

### Token-hallinta

```
1. Käynnistyksen yhteydessä: hae access token API-avaimella
2. Tallenna token + expiry muistiin (ei tietokantaan)
3. Joka pyynnön yhteydessä: tarkista onko token voimassa
4. Jos vanhentunut: refresh tai hae uusi
5. Jos refresh epäonnistuu: hälytys (API-avain vanhentunut?)
```

## Hinnoittelu ja kustannukset

### Kehittäjälle (Grooveserve)

- **Ei kehittäjämaksua** — Procountor ei peri integraatiokumppanilta maksuja
- **PTS-testiympäristö on ilmainen** — kaikki endpointit ja UI-toiminnot käytettävissä veloituksetta
- Kustannukset kohdistuvat loppuasiakkaalle, ei integraation kehittäjälle

### Loppuasiakkaalle (Procountorin käyttäjä)

| Kustannus | Hinta (alv 0 %) |
|-----------|----------------|
| Kuukausimaksu (API-käyttö) | 12,90 €/kk |
| Per integraatio | + 2,49 €/kk |
| Avausmaksu | 0 € |
| Dokumenttikohtaiset | Procountorin normaali palveluhinnasto |
| ALV | 25,5 % |

Asiakkaan kokonaiskustannus Grooveserve-integraatiolle: **~15,39 €/kk + alv** + dokumenttikohtaiset kulut.

## Kumppaniksi rekisteröityminen

### Procountor

1. Täytä yhteydenottolomake: `dev.procountor.com/contact/request-testing-environment/`
2. Procountor provisioi PTS-tunnukset manuaalisesti (ei itsepalvelua)
3. Kehitä ja testaa PTS-ympäristössä
4. Ota yhteyttä Procountoriin tuotantokäyttöön siirtymiseksi
5. Sertifiointiprosessi (12 tarkistuspistettä, ks. alla)
6. **Vaatimus:** Vähintään 2 tuotantoasiakasta ennen virallista sertifiointia

**Aikataulua ei ole julkisesti dokumentoitu.** Prosessi on lomakepohjainen, ei läpinäkyvä.

### Vertailu: Netvisor-kumppanuus

| Ominaisuus | Procountor | Netvisor |
|-----------|-----------|----------|
| **Hakemus** | Yhteydenottolomake | Strukturoitu hakemus |
| **Käsittelyaika** | Ei ilmoitettu | 1-3 arkipäivää |
| **Tasot** | Ei tasoja | 3 tasoa (Integrator, Partner, Store) |
| **Prosessi** | Epäselvä, tapauskohtainen | Selkeä 6-vaiheinen |
| **Kehittäjämaksut** | Ei | Ei |
| **Kumppaniverkosto** | Ei julkista tietoa | 850+ kumppania, 40 000+ yritystä |
| **Tyytyväisyys** | Ei julkista tietoa | 95 %+ |

**Johtopäätös:** Netvisorin kumppaniprosessi on läpinäkyvämpi ja nopeampi. Procountorin prosessi on epämääräisempi mutta API itsessään on modernimpi.

## Sertifiointi ja tuotantoon pääsy

Procountor vaatii **sertifiointiprosessin** ennen tuotanto-API:n käyttöä. 12 tarkistuspistettä (`dev.procountor.com/api-certification/`):

1. **Test Environment** — validointi PTS:ssä ensin
2. **Integration Communication** — request/response-formaatit, sivutus, rajat
3. **Authentication** — OAuth 2.0 (M2M suositeltu)
4. **API Users & Permissions** — käyttöoikeushallinta
5. **Product Type** — kohdejärjestelmä Procountor Financials
6. **Webhooks** — pankkisiirrot, palkat, laskutapahtumat
7. **Performance** — 60 req/s rate limit, sivutus, exponential backoff
8. **Error Handling** — virheilmoitukset käyttäjälle, lokitus
9. **Release Notes & API Versions** — rikkovat muutokset, versioseuranta
10. **Web Browser Support** — selaintuki (jos web-UI)
11. **Client Name** — nimen vastaavuus markkinointinimeen
12. **Customer Guidance** — integraatio-ohjeet loppukäyttäjälle

**Aikataulu:** Ei julkisesti dokumentoitu. Vaatii vähintään 2 tuotantoasiakasta → catch-22: tarvitaan asiakkaita sertifiointiin, mutta sertifiointia tuotantokäyttöön. Käytännössä tämä tarkoittanee, että alkuvaiheen asiakkaat käyttävät integraatiota ennen virallista sertifiointia.

## Riskit ja avoimet kysymykset

| Riski | Vaikutus | Mitigaatio |
|-------|---------|-----------|
| Sertifiointiprosessi kestää | Viivästyttää tuotantokäyttöä | Aloita sertifiointi aikaisin, kehitä PTS-ympäristössä |
| 2 asiakkaan vaatimus sertifiointiin | Catch-22 tuotantokäytölle | Neuvottele Procountorin kanssa poikkeus alkuvaiheeseen |
| Kumppaniprosessin läpinäkymättömyys | Ei tiedetä aikatauluja | Ota yhteyttä ajoissa, varaa puskuria |
| API-versiointi (YY.MM) | Rikkovat muutokset | Seuraa Procountorin release-kalenteria, tue 1-2 versiota |
| Asiakkaan onboarding | OAuth-flow vaatii web-UI:n | MVP: manuaalinen API-avaimen syöttö |
| Päiväraha/km-korvausten tuotekoodit | Procountorin odottamat koodit voivat vaihdella | Selvitä testiympäristössä |

**Avoimet kysymykset:**
1. Mitä tuotekoodeja Procountor odottaa päivärahoille ja km-korvauksille?
2. Tukeeko Procountor suoraan Verohallinnon päiväraha- ja km-korvausmääriä vai pitääkö ne syöttää manuaalisesti?
3. Miten Procountorin hyväksyntäkierto integroituu Grooveserven omaan hyväksyntäkiertoon?
4. Miten "2 tuotantoasiakasta ennen sertifiointia" -vaatimus käytännössä toimii uudelle kumppanille?

## Suositus MVP:lle

### Vaihe 1: Perusta (arvio: 1-2 viikkoa)
- Rekisteröidy Procountor-kehittäjäksi, hanki PTS-testiympäristön tunnukset
- Toteuta M2M OAuth 2.0 -autentikointi (token-hallinta)
- Toteuta matkalaskun luonti (`POST /invoices` + `TRAVEL_INVOICE`)
- Testaa PTS-ympäristössä

### Vaihe 2: Liitteet ja haku (arvio: 1 viikko)
- Kuittiliitteiden upload (`POST /attachments`)
- Laskujen statusten seuranta
- Virhekäsittely ja retry-logiikka

### Vaihe 3: Sertifiointi (arvio: 2-4 viikkoa)
- Sertifiointiprosessin läpikäynti
- Tuotanto-API:n käyttöönotto

### MVP-rajaus
- **Sisältyy:** Matkalaskun luonti, kuittiliitteet, M2M-autentikointi
- **Ei sisälly MVP:hen:** Hyväksyntäkierto (käytetään Procountorin omaa), webhookit, Netvisor-integraatio
- **Onboarding:** MVP:ssä asiakas toimittaa API-avaimen manuaalisesti (ei OAuth-flow:ta)

### Trait-rajapinta alusta alkaen

Vaikka Netvisor-integraatio ei kuulu MVP:hen, kannattaa `AccountingExport`-trait määritellä alusta alkaen. Se ei lisää merkittävästi työmäärää mutta helpottaa myöhempää Netvisor-tukea.
