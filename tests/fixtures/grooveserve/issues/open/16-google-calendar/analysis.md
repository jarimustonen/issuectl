# Google Calendar -integraatio: analyysi

## Yhteenveto

Google Calendar API v3 tarjoaa kattavan REST-rajapinnan kalenteritapahtumien lukemiseen. Suora HTTP (`reqwest` + `serde` + `oauth2`-crate) on paras toteutustapa Rustilla — valmiit kirjastot ovat joko ylläpitämättömiä tai epävakaita. Kalenteridata on **kontekstia ja evidenssiä** matkatunnistukselle, ei absoluuttinen totuus — Verohallinnon päivärahalaskenta vaatii minuuttitarkkuutta, jota kalenteritapahtumat eivät tarjoa.

## 1. Google Calendar API v3

### Autentikointi (OAuth 2.0)

**Tarvittava scope:** `https://www.googleapis.com/auth/calendar.events.readonly` (sensitive, ei restricted)

**Web server -flow:**
1. Ohjaa käyttäjä Googlen consent-sivulle (`accounts.google.com/o/oauth2/v2/auth`)
2. Parametrit:
   - `client_id`, `redirect_uri`, `response_type=code`
   - `scope=https://www.googleapis.com/auth/calendar.events.readonly`
   - `access_type=offline` — kriittinen, ilman ei saa refresh_tokenia
   - `prompt=consent` — pakollinen, muuten reconnect ei palauta refresh_tokenia
   - `include_granted_scopes=true` — mahdollistaa inkrementaalisen scopejen lisäyksen
   - `state` — CSRF-suoja (ks. state-hallinta alla)
   - `code_challenge` + `code_challenge_method=S256` — PKCE
3. Käyttäjä hyväksyy, Google redirectaa takaisin `?code=...`
4. Palvelin vaihtaa koodin tokeneihin: `POST oauth2.googleapis.com/token` (sisältäen `code_verifier` PKCE:lle)
5. Vastaus: `access_token` (~1h), `refresh_token` (pitkäikäinen), `expires_in`

**PKCE (Proof Key for Code Exchange):** Käytetään vaikka kyseessä on confidential client. `oauth2`-crate tukee natiivisti (~4 riviä koodia). Suojaa authorization code -väärinkäytöltä vaikka koodi vuotaisi (browser extension, logi, proxy).

**State-parametrin hallinta:**
- Generoi kryptografisesti vahva `state`-nonce
- Tallenna tiivisteenä PostgreSQL:iin yhdessä `user_id`:n, `redirect_uri`:n, `code_verifier`:n ja vanhenemisajan kanssa
- Callbackissa: tarkista state löytyy, ei vanhentunut, ei käytetty, kuluta atomisesti
- Callbackin jälkeen: tarkista user session vastaa odotettua

**Token refresh:** POST samaan token-endpointiin `grant_type=refresh_token` + tallennettu `refresh_token`. Ei vaadi käyttäjän toimia.

**Refresh tokenin vanheneminen — KRIITTINEN:**
Refresh token voi lakata toimimasta monesta syystä:
- **Testing-tilassa (ennen verifiointia): vanhenee 7 päivässä.** Tämä dominoi MVP-kehitystä.
- Käyttäjä peruuttaa pääsyn
- Workspace-admin estää sovelluksen
- Käyttäjä vaihtaa salasanan
- Yli 50 refresh tokenia per käyttäjä/client (vanhin revokoidaan)
- Client secret nollataan
- Googlen riskijärjestelmä invalidoi

**Token-elinkaaritilat:**
```
connected → needs_reauth → revoked
              ↑                ↑
    refresh_failed      user_disconnected
    admin_blocked       token_expired (testing)
```

Järjestelmän on käsiteltävä `invalid_grant` normaalina tilana (ei poikkeuksena) ja pyydettävä käyttäjää yhdistämään uudelleen.

**Service account vs. user consent:** Service account toimii vain Google Workspace domain-wide delegationilla (admin myöntää). Ulkoisille käyttäjille (meidän case) tarvitaan standard OAuth user consent flow — jokainen käyttäjä myöntää erikseen.

### Events API

**Endpoint:** `GET /calendar/v3/calendars/{calendarId}/events`

MVP:ssä `calendarId=primary` käyttäjän pääkalenterille. Rajoitus dokumentoitava käyttäjälle — osa matkatapahtumista voi olla jaetuissa/tuoduissa kalentereissa.

**Oleelliset parametrit:**

| Parametri | Käyttö |
|-----------|--------|
| `timeMin` / `timeMax` | RFC3339 datetime, aikavälirajaus |
| `singleEvents=true` | Purkaa toistuvat tapahtumat yksittäisiksi |
| `orderBy=startTime` | Kronologinen järjestys (vaatii `singleEvents=true`) |
| `maxResults` | Max 2500/sivu (oletus 250) |
| `syncToken` | Inkrementaalinen synkronointi — palauttaa vain muuttuneet |
| `fields` | **Pakollinen** — datan minimointi (ks. alla) |

**Huom:** `q` (tekstihaku) ei sovellu sync-strategiaan — yhteensopivuusongelmat syncTokenin kanssa, monikielisyysongelmat, kiintiöiden tuhlaus. Käytä aikavälirajausta + paikallista luokittelua.

**Data minimointi `fields`-parametrilla:**

Oletuskyselyssä haetaan vain MVP:n vaatimat kentät (GDPR datan minimointi):

```
fields=items(id,etag,status,updated,summary,location,start,end,eventType,transparency,recurringEventId,originalStartTime,htmlLink),nextPageToken,nextSyncToken
```

**EI haeta oletuksena:**
- `description` — sisältää arkaluonteista dataa (terveyskäynnit, yksityiset muistiinpanot). Haetaan vain yksittäiselle kandidaattitapahtumalle tarvittaessa.
- `attendees` — PII (sähköpostiosoitteet, nimet)
- `conferenceData`, `attachments`, `creator`, `organizer`, `source`

**Tapahtuman kentät matkaevidenssin keräämiseen:**

| Kenttä | Merkitys |
|--------|----------|
| `summary` | Tapahtuman otsikko — "Lento Helsinkiin", "Hotel Marriott" |
| `location` | Vapaamuotoinen teksti — osoite, kaupunki, venue |
| `start` / `end` | Tapahtuman aika. **Huom:** koko päivän tapahtumissa `end.date` on exclusive (2 päivän tapahtuma: start=5.1., end=7.1.) |
| `eventType` | `default`, `outOfOffice`, `workingLocation`, `fromGmail` |
| `transparency` | `opaque` (varattu) vs `transparent` (vapaa) |
| `status` | `confirmed` / `tentative` / `cancelled` |

**Aikavyöhykkeet:** Events API palauttaa joko `dateTime` (RFC3339 + offset) tai `date` (koko päivän tapahtuma, NaiveDate). Suomen päivärahalaskennassa matkan kesto lasketaan todellisen kuluneen ajan mukaan, ja ulkomaan päiväraha määräytyy sen mukaan missä maassa ollaan vuorokauden vaihtuessa Suomen aikaa. Aikavyöhykekäsittely on kriittinen.

**Pagination:** `nextPageToken` iteroitava loppuun. `nextSyncToken` tallennetaan vasta viimeisen sivun jälkeen. Osittainen sync-token-päivitys sivuvirheessä korruptoi synkronointitilan.

### SyncToken — kriittiset reunaehdot

SyncToken ei ole yksinkertainen "anna muutokset" -kursori:

- **Sidottu kyselyparametreihin:** Token on validi vain täsmälleen samalla parametrisetillä (mukaan lukien `fields`) kuin alkuperäinen kysely. Parametrien muutos → token hylätään, uusi full sync.
- **HTTP 410 Gone:** Token vanhentunut → tyhjennä paikallinen cache, suorita full sync.
- **Per-kalenteri:** Jokaisella kalenterilla on oma sync token.
- **Cancelled/deleted -tapahtumat:** Inkrementaalinen sync palauttaa tombstone-merkinnät, jotka pitää käsitellä.
- **Toistuvat tapahtumat:** Yksittäisten instanssien peruutukset/muutokset vaativat huolellista käsittelyä.

**Sync-tilan tallennus:**

```sql
CREATE TABLE calendar_sync_states (
    id uuid PRIMARY KEY,
    connection_id uuid NOT NULL REFERENCES calendar_connections(id),
    calendar_id text NOT NULL,
    sync_token text,
    query_version int NOT NULL,  -- parametrien muutos invalidoi tokenin
    last_full_sync_at timestamptz,
    last_incremental_sync_at timestamptz,
    last_error text,
    UNIQUE (connection_id, calendar_id)
);
```

**Sync-algoritmi:**
```
1. Refresh access token jos vanhentunut
2. Jos sync_token olemassa → inkrementaalinen sync
3. Jos ei sync_tokenia → rajattu full sync (timeMin/timeMax)
4. Iteroi kaikki sivut (nextPageToken)
5. Tallenna nextSyncToken vasta viimeisen sivun jälkeen
6. HTTP 410 Gone → tyhjennä cache, full resync
7. HTTP 401 → refresh token; jos invalid_grant → merkitse needs_reauth
8. HTTP 429/403 → exponential backoff + jitter
```

### Kiintiöt

- Ei per-query-kustannusta (ilmainen API)
- Kiintiöt per-project ja per-user (sliding window/minuutti)
- Tarkat arvot vaihtelevat ja näkyvät Google Cloud Consolessa — älä kovakoodaa
- Ylitys: HTTP 403/429 + `Retry-After` -header
- **Rate limit -strategia:** Exponential backoff + jitter, per-käyttäjä ja globaali rajoitus, vältä synkronointia kaikille käyttäjille samalla hetkellä

### Push-notifikaatiot (Watch)

**Tuettu**, mutta rajoituksin. **Siirretään post-MVP:hen:**

- Notifikaatio kertoo vain *että jotain muuttui*, ei mitä
- **Ei 100% luotettava** — pieni osa viesteistä katoaa
- Kanavat vanhenevat, ei auto-renewia
- Vaatii: HTTPS-endpoint, valid SSL, domain verification Google Search Consolessa
- Webhook-endpointin on vastattava <10s (enqueue-and-ack, ei synkronista prosessointia)
- `X-Goog-Channel-Token` -verifikointi pakollinen (spoofed webhook -suojaus)
- Lead time: domain verification Search Consolessa ennen kuin watch toimii

MVP:ssä periodic polling riittää; push-notifikaatiot ovat optimointi.

### Google-verifiointi — kriittinen polku

`calendar.events.readonly` on **sensitive scope** → vaatii Googlen verifioinnin tuotantoon.

**Verifioinnin vaatimukset:**
- Julkinen kotisivu + sovelluksen kuvaus (meillä on)
- Privacy policy, joka kuvaa Google API -datan käytön (meillä on, päivitettävä)
- **Google API Services User Data Policy: "Limited Use" -kielioppi** pakollinen privacy policyssä
- Terms of service (meillä on)
- Domain-verifiointi
- Demo-video ja scope-perustelut mahdollisesti
- **Arvioitu kesto: 2-6 viikkoa** vastailu mukaan lukien

**Testing-tila (ennen verifiointia):**
- Max 100 testkäyttäjää, jokainen lisättävä **manuaalisesti** Google Cloud Consoleen
- Consent-ruudussa näkyy "This app isn't verified" -varoitus → ei kelpaa kaupalliseen käyttöön
- **Refresh tokenit vanhenevat 7 päivässä** Testing-tilassa

**Suositus:** Aloita verifiointi heti kun OAuth-flow on toimiva. Verification on kriittisen polun elementti, joka voi estää pilotin.

## 2. Rust-kirjastovertailu ja suositus

### Vertailu

| Kriteeri | google-calendar3 (Byron) | google-calendar (Oxide) | Suora HTTP (reqwest + serde + oauth2) |
|----------|:-:|:-:|:-:|
| API-kattavuus | Koko API | Koko API | Vain mitä tarvitaan |
| OAuth | yup-oauth2 (CLI-oriented) | Oma (0.x) | oauth2-crate (vakaa) |
| Ylläpito | **Etsii ylläpitäjää** | Oxide (ei core-tuote) | Oma vastuu |
| Turvallisuus | RUSTSEC-2025-0134 auki | OK | Oma vastuu |
| SaaS-yhteensopivuus | **Heikko** (tiedostopohjainen token-cache) | Keskitaso | **Täysi kontrolli** |
| Riippuvuudet | Raskas (hyper, yup-oauth2, hyper-rustls) | Keskikokoinen | Kevyt (reqwest + serde jo käytössä) |
| O365-valmius | Ei | Ei | **Kyllä** (CalendarProvider-trait) |

### Suositus: Suora HTTP (`reqwest` + `serde` + `oauth2`)

**Perustelut:**

1. **`google-calendar3` on hylätty** — maintenance mode, avoin RUSTSEC, etsii uutta ylläpitäjää. `InstalledFlowAuthenticator` + `persist_tokens_to_disk` on CLI-pattern joka **ei toimi** monen käyttäjän web-backendissä tai Podman-konteissa (tokenit katoavat restartissa, ei per-user erottelua, ei salausta). Custom `TokenStorage`-trait-toteutus yup-oauth2:lle kumoaa "helppouden" edun.

2. **Oxide `google-calendar` 0.10.0 on epävakaa** — semver-unstable, single-vendor (ei Oxiden core-tuote), generoitu kolmannen osapuolen OpenAPI-speksistä (ei Googlen virallinen Discovery), oma auditoimaton OAuth-toteutus.

3. **Tarvittava API-pinta on pieni:** Events.list, OAuth token exchange/refresh, mahdollisesti watch + channels.stop. ~300-500 riviä tyypitettyä koodia.

4. **`CalendarProvider`-trait mahdollistaa O365-tuen** (Microsoft Graph) myöhemmin ilman uudelleenkirjoitusta. AGENTS.md listaa "kalenteri (Google/O365)".

**Toteutusrunko:**

```rust
#[async_trait::async_trait]
pub trait CalendarProvider {
    async fn exchange_code(&self, code: &str, code_verifier: &str, redirect_uri: &str) -> Result<TokenSet>;
    async fn refresh_token(&self, refresh_token: &str) -> Result<TokenSet>;
    async fn list_events(&self, access_token: &str, req: &ListEventsRequest) -> Result<ListEventsResponse>;
}

pub struct GoogleCalendarClient {
    http: reqwest::Client,
    client_id: String,
    client_secret: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct GoogleEvent {
    pub id: String,
    pub etag: Option<String>,
    pub status: Option<String>,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub start: GoogleEventDateTime,
    pub end: GoogleEventDateTime,
    #[serde(rename = "eventType")]
    pub event_type: Option<String>,
    pub transparency: Option<String>,
    #[serde(rename = "recurringEventId")]
    pub recurring_event_id: Option<String>,
    #[serde(rename = "htmlLink")]
    pub html_link: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GoogleEventDateTime {
    /// Tarkka aika + aikavyöhyke (normaalit tapahtumat)
    #[serde(rename = "dateTime")]
    pub date_time: Option<chrono::DateTime<chrono::FixedOffset>>,
    /// Koko päivän tapahtuma (end.date on exclusive!)
    pub date: Option<chrono::NaiveDate>,
    #[serde(rename = "timeZone")]
    pub time_zone: Option<String>,
}
```

## 3. Matkatietojen tunnistaminen

### Perusperiaate: kalenteri = evidenssi, ei totuus

**Verohallinnon päivärahavaatimus:** Osapäiväraha (10h) ja kokopäiväraha (24h) lasketaan minuuttitarkkuudella matkan alkamisesta (lähtö kotoa/työpaikalta) ja päättymisestä (paluu). Kalenteritapahtuma "Berlin meeting 09:00" ei kerro milloin käyttäjä lähti kotoa.

Kalenterin rooli on tuottaa **matkakandidaatteja** ja **kontekstia** AI-agentille, joka kysyy käyttäjältä tarkat ajat. Kalenteridata ei koskaan tuota suoraan lopullisia matkalaskun aikoja.

```
Kalenteritapahtuma → Evidenssi → TripCandidate (luottamus + todisteet)
  → AI-agentti kysyy käyttäjältä tarkat ajat
  → Käyttäjän vahvistus → Trip (matkalaskukelpoinen)
```

### Matkaevidenssin signaalit

**Vahvat signaalit:**
- `location` sisältää kaupungin eri kuin käyttäjän kotikaupunki/toimisto
- `location` sisältää hotellin/lentokentän/venue-osoitteen
- `summary` sisältää avainsanoja: "lento", "flight", "hotel", "majoitus" + vieras kaupunki
- Monipäiväinen tapahtuma + location eri kuin kotikaupunki
- `outOfOfficeProperties` samalle ajanjaksolle

**Keskivahvat/opportunistiset signaalit:**
- `eventType == "fromGmail"` — hyödyllinen kun saatavilla, mutta: Smart Features usein pois päältä EU:ssa/Workspacessa, puuttuminen ei tarkoita mitään, ei todista työmatkaa eikä korvattavuutta
- Ulkoiset osallistujat (eri email-domain) + location
- `source`-URL osoittaa varauspalveluun
- Koko päivän tapahtuma kaupungin nimellä

**Heikot negatiiviset signaalit:**
- `conferenceData` ilman fyysistä locationia — usein etäkokous, mutta hybriditapaamisissakin on Meet-linkki
- `transparency: "transparent"` — informatiivinen, mutta matkustajat merkitsevät usein transparent
- `status: "cancelled"` — peruttu matka

**EI matkasignaali:**
- "kokous" / "meeting" (yleisin ei-matka-tapahtuma)

### Sijainnin normalisointi

Vapaamuotoiset `location`-kentät ovat sotkuisia: "HKI", "Helsinki", "Helsingfors", "Vantaan lentokenttä", "Customer office", "Kampintie 3". Vertailu kotikaupunkiin vaatii normalisointia.

**MVP-lähestymistapa:** LLM-pohjainen tulkinta osana AI-agentin päättelyä. Agentti saa kalenteritapahtumat kontekstina ja tulkitsee sijainnit. Erillinen geokoodauspalvelu (Google Maps API) on ylimitoitettu MVP:lle ja tuo GDPR-prosessointikysymyksiä.

### Tunnistusstrategia (heuristiikka-ensin, LLM tueksi)

1. **Deterministinen esisuodatus:** Hae tapahtumat aikavälille minimaalisilla kentillä. Suodata pois cancelled, transparent-without-location, toistuvat ilman locationia.
2. **Heuristinen luokittelu:** Location-vertailu, avainsanat, monipäiväisyys → confidence score + evidence trail.
3. **LLM ambiguiteettitilanteissa:** Confidence 0.3-0.7 → LLM tulkitsee kontekstissa (kalenteritapahtuma + käyttäjäprofiili). LLM ei saa raakakuvauskenttiä oletuksena.
4. **Käyttäjän vahvistus:** TripCandidate esitetään käyttäjälle todisteineen. AI-agentti kysyy tarkemmat tiedot (lähtöaika, paluuaika).

**TripCandidate-malli:**

```rust
pub struct TripCandidate {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub destination_text: Option<String>,
    pub confidence: f32,
    pub evidence: Vec<Evidence>,
    pub requires_confirmation: bool,  // aina true MVP:ssä
}

pub struct Evidence {
    pub source: EvidenceSource,
    pub event_id: Option<String>,
    pub summary: Option<String>,  // sanitoitu
    pub location: Option<String>,
    pub event_start: Option<DateTime<Utc>>,
    pub event_end: Option<DateTime<Utc>>,
    pub signal_type: String,
    pub signal_strength: f32,
}

pub enum EvidenceSource {
    CalendarEvent,
    Receipt,
    Email,
    UserConfirmation,
}
```

## 4. Token-hallinta ja tietoturva (GDPR)

### Token-tallennus — salattu, pakolliset kentät

OAuth refresh token antaa pitkäaikaisen pääsyn käyttäjän kalenteriin. **Salaus at rest on lakisääteinen vaatimus** (GDPR Art. 32), ei avoin kysymys.

```sql
CREATE TABLE calendar_connections (
    id uuid PRIMARY KEY,
    tenant_id uuid NOT NULL,
    user_id uuid NOT NULL,
    provider text NOT NULL CHECK (provider IN ('google')),  -- laajennettavissa: 'microsoft'
    provider_account_email text,
    scopes text[] NOT NULL,
    -- Salatut tokenit (envelope encryption, avain SOPS/age:sta)
    refresh_token_ciphertext bytea NOT NULL,
    refresh_token_key_id text NOT NULL,
    access_token_ciphertext bytea,
    access_token_expires_at timestamptz,
    -- Elinkaaritila
    status text NOT NULL CHECK (status IN (
        'connected', 'needs_reauth', 'revoked',
        'refresh_failed_transient', 'admin_blocked'
    )),
    last_sync_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    UNIQUE (tenant_id, user_id, provider)
);
```

**Salaus:** AES-256-GCM envelope encryption. Avain ei tietokannassa — SOPS + age (jo käytössä projektissa). Avainten rotaatiosuunnitelma (`refresh_token_key_id` mahdollistaa).

**Mitä EI saa tehdä:**
- Plaintext-tokenia lokeissa, traceissa tai panic-konteksteissa
- Tokenien näyttämistä API-vastauksissa
- Access tokenia levylle (lyhytikäinen, voidaan pitää muistissa tai salattuna)

### Datan säilytys ja minimointi

- **Raakoja kalenteritapahtumia ei tallenneta.** Käsittely tapahtuu muistissa.
- **Evidence-tiedot tallennetaan** `trip_candidates`/`trip_evidences` -tauluihin: event_id, summary (sanitoitu), start, end, location. Tämä mahdollistaa auditoinnin ilman PII-vuotoriskiä.
- **Description-kenttää ei haeta oletuksena** (fetched on-demand vain kandidaattitapahtumille).
- **Tokenien poisto:** Disconnect → kutsu Google revocation endpoint (`POST oauth2.googleapis.com/revoke`) → poista refresh_token_ciphertext → pysäytä synkronointi → merkitse trip_candidates lähteen revokoiduksi.
- **Käyttäjän poisto (GDPR Art. 17):** Kaskadoiva poisto: tokens, sync state, trip candidates, evidence.

### Kalenteridata ja LLM

Jos kalenteritapahtumia syötetään LLM:lle:
- Minimoi ennen prompt-konstruktiota (vain summary, location, start/end)
- Ei raaka-descriptioneja
- Ei osallistujien sähköpostiosoitteita
- Asiakaskohtaiset AI-datankäsittelyehdot (subprocessor-dokumentaatio)

## 5. MVP-toteutuksen scope

### Vaihe 1: OAuth + perusluku
- Google Cloud Console -projekti + OAuth credentials
- Minimaalinen web-endpoint OAuth-callbackille (redirect_uri)
- OAuth 2.0 consent flow: PKCE, state-hallinta, `prompt=consent`
- Token-tallennus PostgreSQL:iin (salattu)
- Token-elinkaaritilat (connected/needs_reauth/revoked)
- `Events.list` haku aikavälillä + `fields` minimointi
- Disconnect-flow (token revocation)

### Vaihe 2: Matkatunnistus
- Käyttäjäprofiiliin kotikaupunki/toimisto-osoite
- Deterministinen heuristiikka: location-vertailu, avainsanat, monipäiväisyys
- Confidence scoring + evidence trail
- TripCandidate-mallin tallennus
- AI-agentti esittää kandidaatit käyttäjälle ja kysyy tarkat ajat

### Vaihe 3: Jatkuva synkronointi
- PostgreSQL-pohjainen työjono (`FOR UPDATE SKIP LOCKED`, jitter, backoff)
- `syncToken`-pohjainen inkrementaalinen haku
- 410 Gone -käsittely (full resync)
- Per-user ja globaali rate limiting
- Exponential backoff 429/403/5xx -virheissä

### Vaihe 4 (post-MVP): Optimointi
- Push-notifikaatiot (watch channels + webhook endpoint)
- Usean kalenterin tuki (CalendarList + kalenteri-valitsin)
- Microsoft 365 -tuki (CalendarProvider-traitin Microsoft Graph -toteutus)

### Arkkitehtuurihahmotus

```
Käyttäjä
  ↓ "Yhdistä kalenteri" (web-endpoint)
  ↓ OAuth consent → Google (PKCE + state + prompt=consent)
  ↓ redirect + auth code
  ↓
grooveserve-agent (services/email tai erillinen services/calendar)
  ↓ vaihda code → tokens (code_verifier)
  ↓ tallenna salatut tokenit PostgreSQL:iin
  ↓
  ↓ PostgreSQL-pohjainen työjono (ei tokio::interval)
  ↓ Events.list + syncToken + fields-minimointi
  ↓
  ↓ heuristinen matkatunnistus → TripCandidate (confidence + evidence)
  ↓ LLM ambiguiteettitilanteissa
  ↓ tallenna trip_candidates + trip_evidences
  ↓
AI-agentti
  ↓ esittää kandidaatit käyttäjälle
  ↓ kysyy tarkat lähtö-/paluuajat
  ↓ yhdistää kuitit + vahvistetut matkat → matkalaskuehdotus
```

### Palvelun sijoitus

**Vaihtoehto A: Moduuli `services/email`-binäärissä** (eristetty tokio-taskilla, oma DB-pool). Pragmaattinen yhden kehittäjän MVP:lle. Jos valitaan tämä, nimeä palvelu uudelleen (esim. `services/agent`) koska se käsittelee muutakin kuin sähköpostia.

**Vaihtoehto B: Erillinen `services/calendar/`** Puhtaampi eristys (eri failure modet, eri credentials, pienempi blast radius). Lisää operatiivista työtä (toinen kontti, CI/CD, monitorointi).

Molemmat vaihtoehdot ovat perusteltuja. Tärkeintä on puhdas moduulirajaus — refaktorointi erilliseksi palveluksi on mekaanista myöhemmin.

### Kriittisen polun elementit

1. **Google-verifiointi** — aloita heti kun OAuth-flow toimii (2-6 viikkoa). Ilman tätä max 100 manuaalisesti lisättyä testkäyttäjää + pelottava "unverified app" -varoitus.
2. **OAuth callback -endpoint** — vaatii web-pinnan jota projektissa ei vielä ole. Minimaalinen HTTP-handler riittää, mutta on rakennettava.
3. **Privacy policy -päivitys** — Google API Limited Use -kielioppi lisättävä.

### Tietokantaskeema (yhteenveto)

```sql
-- OAuth state (CSRF + PKCE)
CREATE TABLE oauth_states (
    state_hash bytea PRIMARY KEY,
    user_id uuid NOT NULL,
    provider text NOT NULL,
    redirect_uri text NOT NULL,
    code_verifier_enc bytea NOT NULL,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz
);

-- Kalenteriyhteydet (salatut tokenit)
CREATE TABLE calendar_connections (
    id uuid PRIMARY KEY,
    tenant_id uuid NOT NULL,
    user_id uuid NOT NULL,
    provider text NOT NULL,
    provider_account_email text,
    scopes text[] NOT NULL,
    refresh_token_ciphertext bytea NOT NULL,
    refresh_token_key_id text NOT NULL,
    access_token_ciphertext bytea,
    access_token_expires_at timestamptz,
    status text NOT NULL,
    last_sync_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    UNIQUE (tenant_id, user_id, provider)
);

-- Sync-tila per kalenteri
CREATE TABLE calendar_sync_states (
    id uuid PRIMARY KEY,
    connection_id uuid NOT NULL REFERENCES calendar_connections(id) ON DELETE CASCADE,
    calendar_id text NOT NULL,
    sync_token text,
    query_version int NOT NULL,
    last_full_sync_at timestamptz,
    last_incremental_sync_at timestamptz,
    last_error text,
    UNIQUE (connection_id, calendar_id)
);

-- Synkronointityöjono
CREATE TABLE calendar_sync_jobs (
    id uuid PRIMARY KEY,
    connection_id uuid NOT NULL REFERENCES calendar_connections(id) ON DELETE CASCADE,
    calendar_id text NOT NULL,
    run_after timestamptz NOT NULL,
    locked_by text,
    locked_until timestamptz,
    attempts int NOT NULL DEFAULT 0,
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Matkakandidaatit (ei raaka-kalenteridata)
CREATE TABLE trip_candidates (
    id uuid PRIMARY KEY,
    tenant_id uuid NOT NULL,
    user_id uuid NOT NULL,
    source text NOT NULL,
    starts_at timestamptz NOT NULL,
    ends_at timestamptz NOT NULL,
    destination_text text,
    confidence numeric NOT NULL,
    requires_confirmation boolean NOT NULL DEFAULT true,
    confirmed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Matkaevidenssi (auditoitava, minimoitu)
CREATE TABLE trip_evidences (
    id uuid PRIMARY KEY,
    trip_candidate_id uuid NOT NULL REFERENCES trip_candidates(id) ON DELETE CASCADE,
    source text NOT NULL,  -- 'calendar_event', 'receipt', 'email', 'user'
    event_id text,
    summary_sanitized text,
    location text,
    event_start timestamptz,
    event_end timestamptz,
    signal_type text NOT NULL,
    signal_strength numeric NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
```
