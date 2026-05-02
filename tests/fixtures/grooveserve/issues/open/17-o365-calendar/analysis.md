# O365/Outlook Calendar -integraatio: analyysi

## Yhteenveto

Microsoft Graph API tarjoaa REST-rajapinnan O365-kalenteritapahtumien lukemiseen. Rustille on olemassa kolmannen osapuolen SDK (`graph-rs-sdk`), mutta suoran `reqwest`-pohjaisen toteutuksen riski on pienempi ja hallittavampi MVP:ssä. OAuth 2.0 Authorization Code + PKCE (defense-in-depth confidential clientille `client_secret`-autentikoinnin lisäksi) sopii delegated-käyttöön. Google Calendar -integraatio noudattaa samaa arkkitehtuuria, joten yhteinen abstraktio on mahdollinen myöhemmin — mutta MVP:ssä toteutetaan vain Microsoft konkreettisesti.

---

## 1. Microsoft Graph API — kalenteritapahtumat

### Endpointit

Kaksi pääendpointia kalenteritapahtumien lukemiseen:

| Endpoint | Käyttötarkoitus |
|---|---|
| `GET /me/calendar/events` | Kaikki tapahtumat (single instance + series masters) |
| `GET /me/calendarView?startDateTime=...&endDateTime=...` | Tapahtumat aikaväliltä, toistuvat tapahtumat laajennettuina |

**Grooveserve-käyttöön `calendarView` on oikea valinta** — haetaan aina tietyn aikavälin tapahtumat (esim. viimeiset 30 päivää tai matkan ajankohta).

**Rajoitus:** `/me/calendarView` lukee vain oletuskalenterin. Käyttäjillä voi olla useita kalentereita (työ, henkilökohtainen, jaetut, matka/projekti). MVP:ssä luetaan vain oletuskalenteri — tämä on tietoinen rajoitus. Myöhemmin voidaan laajentaa käyttämällä `/me/calendars` ja `/me/calendars/{id}/calendarView`.

### Pyyntöesimerkki

```http
GET https://graph.microsoft.com/v1.0/me/calendarView
  ?startDateTime=2026-04-01T00:00:00Z
  &endDateTime=2026-04-30T23:59:59Z
  &$select=id,iCalUId,seriesMasterId,type,subject,start,end,location,isAllDay,showAs,sensitivity,lastModifiedDateTime
  &$orderby=start/dateTime
  &$top=100
Authorization: Bearer {access_token}
Prefer: outlook.timezone="UTC"
```

**Huomioita:**
- Aikavyöhyke aina UTC (`Prefer: outlook.timezone="UTC"`) — aikavyöhykemuunnokset tehdään sovelluslogiikassa käyttäjäprofiilin tai tapahtuman sijainnin perusteella
- `id` ja `iCalUId` tarvitaan deduplikaatioon ja delta-synkronointiin
- `sensitivity` tarvitaan yksityisten tapahtumien suodattamiseen
- `attendees` ja `organizer` jätetään pois MVP:ssä tietosuojasyistä — lisätään myöhemmin jos liiketoiminnallinen tarve (edustuskulujen kohdistus)

### Vastauksen rakenne (oleelliset kentät)

```json
{
  "value": [
    {
      "id": "AAMkADB...",
      "iCalUId": "040000008200...",
      "subject": "Asiakastapaaminen Helsinki",
      "start": { "dateTime": "2026-04-15T06:00:00.0000000", "timeZone": "UTC" },
      "end": { "dateTime": "2026-04-15T08:00:00.0000000", "timeZone": "UTC" },
      "location": { "displayName": "Pasilan Visa, Helsinki" },
      "isAllDay": false,
      "showAs": "busy",
      "sensitivity": "normal",
      "lastModifiedDateTime": "2026-04-10T12:00:00Z"
    }
  ]
}
```

**Graph `dateTime` -kenttä ei ole RFC3339** — se on naivi paikallinen aika ilman offsetia. Kun `Prefer: outlook.timezone="UTC"`, arvot ovat UTC:tä mutta ilman `Z`-suffiksia. Tarvitaan dedikoitu parsija.

### Grooveservelle hyödylliset kentät

| Kenttä | Käyttö matkalaskussa |
|---|---|
| `id` / `iCalUId` | Deduplikaatio, synkronointi |
| `subject` | Matkan tarkoitus / selite |
| `start` / `end` | Matkan ajankohta, päivärahalaskenta |
| `location` | Matkakohde |
| `isAllDay` | Signaali (ei yksinään riitä matkan tunnistamiseen) |
| `showAs` | `oof` (out of office) = vahva signaali matkasta |
| `sensitivity` | Yksityisten tapahtumien suodatus |
| `seriesMasterId` | Toistuvien tapahtumien deduplikaatio |

### Sivutus

Graph API palauttaa oletuksena max 10 tapahtumaa per pyyntö. `$top` nostaa max 999:ään. Jos enemmän, vastaus sisältää `@odata.nextLink`-URL:n. Sivutus on pakollinen tuotannossa.

**Turvamekanismit:**
- Maksimi sivumäärä (esim. 20 sivua)
- Maksimi tapahtumamäärä (esim. 500)
- `nextLink`-URL:n validointi (`https://graph.microsoft.com/` -alkuinen) — estää token-vuoto jos Graph palauttaa odottamattoman URL:n
- Kiertosilmukan tunnistus (seen-links set)

### Delta query

`GET /me/calendarView/delta` palauttaa vain muuttuneet tapahtumat edellisen synkronoinnin jälkeen. Hyödyllinen jos kalenteridata synkronoidaan säännöllisesti — vähentää API-kutsuja ja datamäärää merkittävästi.

**MVP:** On-demand `calendarView`-haku rajatulle aikavälille. Delta query ei tarvita.
**Taustasynkronointi (myöhemmin):** Jos AI-agentti hakee kalenteridataa taustalla toistuvasti, delta query on välttämätön API-rajoitusten vuoksi. Vaatii erillisen suunnittelun (delta linkit, vanheneminen, poistettujen tapahtumien käsittely).

---

## 2. Oikeudet ja autentikointi

### Permission-tyypit

| Tyyppi | Scope | Käyttö | Permission metadata: admin consent |
|---|---|---|---|
| **Delegated** | `Calendars.Read` | Käyttäjä kirjautuu, lukee omaa kalenteriaan | Ei |
| **Delegated** | `Calendars.ReadBasic` | Kuten yllä, mutta vähemmän kenttiä (ei body/attendees) | Ei |
| **Application** | `Calendars.Read` | Sovellus lukee kaikkien käyttäjien kalentereita | Kyllä |

**MVP:ssä delegated `Calendars.Read`** on oikea valinta:
- Käyttäjä antaa luvan omaan kalenteriinsa
- Permission-metatiedoissa ei vaadi admin consentia

**Tärkeä huomio B2B-kontekstissa:** Vaikka permission itsessään ei vaadi admin consentia, monet yritys-tenantit estävät käyttäjien consent-oikeuden kokonaan tai vaativat admin-hyväksynnän kaikille kolmannen osapuolen multi-tenant-sovelluksille. Tämä ei ole poikkeustapaus vaan yleinen tilanne suomalaisissa yrityksissä.

**Mitigaatio:**
- OAuth-flow:n tulee tunnistaa `AADSTS90094` (Admin consent required) -virhe
- Tarjotaan admin-consent-endpoint ja -linkki (`https://login.microsoftonline.com/{tenant}/adminconsent?client_id=...`)
- Dokumentoidaan tenant-admineille miten sovellus hyväksytään
- Verified publisher -rekisteröinti (Microsoft Partner Center) parantaa luottamusta merkittävästi

Application permissions tulee kyseeseen myöhemmin jos yrityksen admin haluaa ottaa palvelun käyttöön kaikille kerralla. Tämä on eri koodipohja (service principal, ei käyttäjäkohtainen OAuth) ja vaatii erillisen suunnittelun. Schema suunnitellaan yhteensopivaksi molemmille.

### OAuth 2.0 Authorization Code + PKCE

Grooveserve on **confidential client** (server-side backend `client_secret`-autentikoinnilla). PKCE on defense-in-depth authorization code injection -hyökkäyksiä vastaan — `client_secret` on ensisijainen autentikointimekanismi token-endpointissa.

```
┌─────────┐     ┌──────────┐     ┌──────────────────┐
│ Käyttäjä │────>│ Grooveserve │────>│ Microsoft Entra  │
│ (selain) │     │ (backend)   │     │ (login.microsoft │
│          │<────│             │<────│  online.com)     │
└─────────┘     └──────────┘     └──────────────────┘
```

**Flow:**

1. Käyttäjä klikkaa "Yhdistä O365-kalenteri" Grooveserven UI:ssa
2. Backend generoi `state` + `code_verifier` + `code_challenge`, tallentaa ne väliaikaiseen tauluun (TTL 10 min)
3. Redirect → `https://login.microsoftonline.com/organizations/oauth2/v2.0/authorize`
   - `client_id`, `redirect_uri`
   - `scope=openid profile email offline_access Calendars.Read`
   - `response_type=code`, `response_mode=query`
   - `code_challenge` + `code_challenge_method=S256`
   - `prompt=select_account` (käyttäjä valitsee tilin)
   - `state` (CSRF-suoja)
4. Käyttäjä kirjautuu Microsoft-tilillään ja antaa suostumuksen
5. Microsoft redirect → Grooveserve callback URL + `code` + `state`
6. Backend validoi `state` (one-time use, TTL), hakee `code_verifier`
7. Backend vaihtaa code → `access_token` + `refresh_token`
   - POST `https://login.microsoftonline.com/organizations/oauth2/v2.0/token`
   - Sisältää `client_id`, `client_secret`, `code`, `code_verifier`, `redirect_uri`
8. Backend parsii ID tokenin claims: `oid` (user object ID), `tid` (tenant ID), `preferred_username`
9. Refresh token tallennetaan PostgreSQL:ään (salattu), access token cachetetaan muistiin

**Huomio tenant-endpointista:** Käytetään `/organizations/` endpointia, ei `/common/` — estää henkilökohtaisten Microsoft-tilien (live.com, outlook.com) käytön B2B-palvelussa.

### OAuth state -tallennus

```sql
CREATE TABLE oauth_states (
    state TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    code_verifier TEXT NOT NULL,
    redirect_after TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ
);
```

Callback hylkää: tuntemattoman staten, vanhentuneen staten, jo käytetyn staten.

### Entra ID App Registration

| Asetus | Arvo |
|---|---|
| Application type | Web |
| Redirect URIs | `https://app.grooveserve.com/auth/microsoft/callback`, `http://localhost:3000/auth/microsoft/callback` (dev), `https://staging.grooveserve.com/auth/microsoft/callback` (staging) |
| Supported account types | **Accounts in any organizational directory** (multi-tenant, ei personal) |
| API permissions | `Calendars.Read` (delegated), `User.Read` (delegated), `offline_access` (delegated) |
| Client credentials | Client secret (aluksi) tai certificate (tuotannossa) |
| Publisher verification | Verified publisher (Microsoft Partner Center) — tärkeä B2B-luottamukselle |

### Token-hallinta

| Token | Elinikä | Tallennus |
|---|---|---|
| Access token | ~60-90 min | Muistissa (in-process cache, TTL) |
| Refresh token | Politiikkariippuvainen | PostgreSQL, salattu |

**Refresh tokenin elinkaari on monimutkaisempi kuin "90 päivää":**
- Oletuksena sliding window, mutta tenant-politiikat (Conditional Access, security defaults) voivat lyhentää tai peruuttaa
- Salasanan vaihto peruuttaa kaikki refresh tokenit
- Admin voi peruuttaa consent/tokenit milloin tahansa
- **Microsoft palauttaa uuden refresh tokenin joka käyttökerralla** — uusi token pitää tallentaa atomisesti joka kerta

**Token refresh -strategia:**

```rust
struct TokenRefreshResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>, // tallenna AINA jos mukana
    scope: Option<String>,
}
```

Jos refresh epäonnistuu `invalid_grant` -virheellä:
- Merkitse yhteys tilaan `reauth_required`
- Älä yritä uudelleen
- Ilmoita käyttäjälle (UI/sähköposti)

**Single-flight refresh:** Jos kaksi rinnakkaista kutsua näkee vanhentunen access tokenin, vain yksi saa tehdä refreshin. Muut odottavat tulosta. Muuten toinen refresh epäonnistuu koska ensimmäinen jo invalidoi vanhan refresh tokenin.

---

## 3. Rust-toteutus — kirjastovaihtoehdot

### Vaihtoehto A: `graph-rs-sdk` (kokonaisvaltainen SDK)

```toml
[dependencies]
graph-rs-sdk = "3.0"
```

| + | - |
|---|---|
| Kattaa Graph API:n laajasti | Suuri dependency (koko Graph API) |
| Sisältää OAuth-flowt | Yhden kehittäjän projekti (sreeise) |
| Async/tokio-tuki | ~2400 latausta/kk — pieni yhteisö |
| Tyypitetyt vastaukset | Saattaa sisältää bugeja reunatapauksissa |
| | Vähän calendar-esimerkkejä dokumentaatiossa |

### Vaihtoehto B: `reqwest` + `oauth2` (suora REST)

```toml
[dependencies]
reqwest = { version = "0.12", features = ["json"] }
reqwest-middleware = "0.4"
reqwest-retry = "0.7"
oauth2 = "5"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.10"
```

| + | - |
|---|---|
| Täysi kontrolli HTTP-kutsuihin | OAuth-flow käsin (mutta `oauth2`-crate hoitaa) |
| Pienet, hyvin ylläpidetyt dependencyt | Tyypit kirjoitettava itse (serde) |
| `reqwest` ja `oauth2` ovat de facto standard | Sivutus, virheenkäsittely, retry itse |
| Helppo debugata ja testata | Ylläpitokustannus korkeampi kuin "pari sataa riviä" |
| Sama pattern toimii Google Calendarille | |

**Käyttöesimerkki:**

```rust
use reqwest::Client;
use reqwest::Url;
use serde::Deserialize;

// --- Graph API -tyypit ---

#[derive(Debug, Deserialize)]
struct GraphCalendarEvent {
    id: String,
    #[serde(rename = "iCalUId")]
    ical_uid: Option<String>,
    subject: Option<String>,
    start: GraphDateTimeTimeZone,
    end: GraphDateTimeTimeZone,
    location: Option<GraphLocation>,
    #[serde(rename = "isAllDay")]
    is_all_day: bool,
    #[serde(rename = "showAs")]
    show_as: Option<String>,
    sensitivity: Option<String>,
    #[serde(rename = "lastModifiedDateTime")]
    last_modified: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphDateTimeTimeZone {
    #[serde(rename = "dateTime")]
    date_time: String,  // Naivi paikallinen aika, EI RFC3339
    #[serde(rename = "timeZone")]
    time_zone: String,
}

#[derive(Debug, Deserialize)]
struct GraphLocation {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphEventsResponse {
    value: Vec<GraphCalendarEvent>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

// --- Graph API -virhetyyppi ---

#[derive(Debug, Deserialize)]
struct GraphErrorResponse {
    error: GraphErrorBody,
}

#[derive(Debug, Deserialize)]
struct GraphErrorBody {
    code: String,
    message: String,
}

// --- Grooveserven sisäinen tyyppi ---

enum CalendarEventTime {
    Timed {
        start_utc: DateTime<Utc>,
        end_utc: DateTime<Utc>,
        source_timezone: String,
    },
    AllDay {
        start_date: NaiveDate,
        end_date_exclusive: NaiveDate,
    },
}

struct CalendarEvent {
    provider: String,          // "microsoft"
    external_id: String,       // Graph event id
    ical_uid: Option<String>,
    subject: Option<String>,   // None jos yksityinen tapahtuma
    time: CalendarEventTime,
    location: Option<String>,  // None jos yksityinen tapahtuma
    show_as: Option<String>,
    sensitivity: Option<String>,
}

// --- Haku ---

const MAX_PAGES: usize = 20;
const MAX_EVENTS: usize = 500;

async fn fetch_calendar_events(
    client: &Client,
    access_token: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<GraphCalendarEvent>, CalendarError> {
    let mut all_events = Vec::new();

    let mut url = Url::parse("https://graph.microsoft.com/v1.0/me/calendarView")?;
    url.query_pairs_mut()
        .append_pair("startDateTime", &start.to_rfc3339())
        .append_pair("endDateTime", &end.to_rfc3339())
        .append_pair("$select", "id,iCalUId,seriesMasterId,type,subject,start,end,location,isAllDay,showAs,sensitivity,lastModifiedDateTime")
        .append_pair("$orderby", "start/dateTime")
        .append_pair("$top", "100");

    let mut pages = 0;

    loop {
        let response = client
            .get(url.as_str())
            .bearer_auth(access_token)
            .header("Prefer", r#"outlook.timezone="UTC""#)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            let error: GraphErrorResponse = serde_json::from_str(&body)
                .unwrap_or(GraphErrorResponse {
                    error: GraphErrorBody {
                        code: status.to_string(),
                        message: "Unknown error".to_string(),
                    },
                });
            return Err(CalendarError::Graph {
                status,
                code: error.error.code,
                message: error.error.message,
            });
        }

        let page: GraphEventsResponse = serde_json::from_str(&body)?;
        all_events.extend(page.value);
        pages += 1;

        if all_events.len() >= MAX_EVENTS || pages >= MAX_PAGES {
            break;
        }

        match page.next_link {
            Some(next) if next.starts_with("https://graph.microsoft.com/") => {
                url = Url::parse(&next)?;
            }
            _ => break,
        }
    }

    Ok(all_events)
}
```

### Suositus: Vaihtoehto B (`reqwest` + `oauth2`)

Perustelut:
1. **Pienempi riski** — `reqwest` ja `oauth2` ovat kypsät, laajasti käytetyt kirjastot
2. **Yhtenäinen arkkitehtuuri** — sama `reqwest` + `oauth2` -pattern toimii sekä Microsoft Graph:lle että Google Calendar API:lle
3. **Vähemmän dependencyjä** — ei tarvitse vetää koko Graph SDK:ta kun tarvitaan vain calendar endpoints
4. **Debugattavuus** — suora HTTP, näkee mitä menee sisään ja ulos
5. **Ylläpidettävyys** — ei riippuvuutta yhden henkilön ylläpitämään SDK:hon

**Realistinen koodimäärä:** ~800-1500 riviä sisältäen OAuth-flow, token refresh, virheenkäsittely, retry, sivutus, aikavyöhykemuunnokset, serde-tyypit, ja testit. Ei "200+150 riviä" kuten alun perin arvioitu.

---

## 4. Vertailu: Microsoft Graph vs Google Calendar API

| Ominaisuus | Microsoft Graph | Google Calendar API |
|---|---|---|
| **Base URL** | `graph.microsoft.com/v1.0` | `www.googleapis.com/calendar/v3` |
| **Auth** | OAuth 2.0 (Entra ID) | OAuth 2.0 (Google Cloud) |
| **App registration** | Entra ID portal | Google Cloud Console |
| **Delegated scope** | `Calendars.Read` | `calendar.readonly` |
| **Events endpoint** | `/me/calendarView` | `/calendars/{id}/events` |
| **Aikaväli-filtteröinti** | Query params (`startDateTime`, `endDateTime`) | Query params (`timeMin`, `timeMax`) |
| **Vastausmuoto** | OData JSON (`value[]`) | Google JSON (`items[]`) |
| **Sivutus** | `@odata.nextLink` | `nextPageToken` |
| **Rust SDK** | `graph-rs-sdk` (pieni yhteisö) | `google-calendar3` (Google-generoitu) |
| **Suositus Rustille** | `reqwest` + `oauth2` | `reqwest` + `oauth2` |
| **Token refresh** | Politiikkariippuvainen, rotaatio | Pitkäikäinen, mutta voi vanhentua (inaktiivisuus, salasanan vaihto, admin, Testing-tila) |
| **Multi-tenant** | Sisäänrakennettu (Entra) | Ei tarvita (Google accounts) |

### Arkkitehtuurisuositus

**MVP:ssä ei yhteistä traittia.** Toteutetaan `MicrosoftCalendarClient` konkreettisena modulina. Yhteinen `CalendarProvider`-trait ekstrahoidaan vasta kun Google Calendar toteutetaan ja nähdään todellinen duplikaatio (YAGNI).

`CalendarEvent` on Grooveserven oma tyyppi johon Microsoft-client mappaa:

```rust
struct CalendarEvent {
    provider: String,           // "microsoft"
    external_id: String,        // Graph event id
    ical_uid: Option<String>,
    subject: Option<String>,
    time: CalendarEventTime,
    location: Option<String>,
    show_as: Option<String>,    // Raakadata — AI-agentti päättelee matkan
    sensitivity: Option<String>,
}
```

**Ei `is_travel: bool` -kenttää.** Raakadata pidetään erillisenä liiketoimintalogiikasta. AI-agentti päättelee matkan todennäköisyyden yhdistämällä kalenteridatan kuitteihin, käyttäjäprofiiliin ja aiempiin matkoihin. `showAs=oof`, location, `isAllDay` ovat signaaleja, eivät luokittelua.

---

## 5. Token-tallennus ja tietoturva

### Tietokantarakenne

```sql
CREATE TABLE calendar_connections (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL CHECK (provider IN ('microsoft', 'google')),

    -- Microsoft-identiteetti
    provider_tenant_id TEXT,                    -- Entra tenant ID (tid claim)
    provider_account_id TEXT NOT NULL,           -- Entra object ID (oid claim)
    provider_email TEXT,                         -- preferred_username (informatiivinen)

    -- Token-tallennus (vain refresh token)
    refresh_token_ciphertext BYTEA NOT NULL,
    refresh_token_nonce BYTEA NOT NULL,
    refresh_token_key_id TEXT NOT NULL,

    -- Tila
    scopes TEXT[] NOT NULL,
    status TEXT NOT NULL DEFAULT 'connected'
        CHECK (status IN ('connected', 'reauth_required', 'error', 'disconnected')),
    reauth_reason TEXT,
    last_successful_sync_at TIMESTAMPTZ,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (user_id, provider, provider_account_id)
);
```

**Huomioita:**
- Ei `access_token`-saraketta — access tokenit cachetetaan muistissa (TTL ~50 min)
- `provider_tenant_id` + `provider_account_id` yhdessä yksilöivät Microsoft-identiteetin (oid on uniikki vain tenantin sisällä)
- `UNIQUE (user_id, provider, provider_account_id)` — sallii useita tilejä per provider
- `status` seuraa yhteyden tilaa: `connected` → `reauth_required` (refresh epäonnistui) → `connected` (uudelleenkirjautumisen jälkeen)
- `ON DELETE CASCADE` — käyttäjän poisto poistaa myös kalenteriyhteydet

### Salaus

Token-salaus sovellustasolla AEAD-salauksella:

- **Algoritmi:** AES-256-GCM tai XChaCha20-Poly1305
- **Nonce:** Uniikki satunnainen nonce per token, tallennetaan `refresh_token_nonce`-sarakkeeseen
- **Associated data:** `connection_id`, `provider`, `user_id` — estää ciphertextin siirtämisen toiseen yhteyteen
- **Avainten hallinta:** Master key ladataan SOPS/age-salatusta konfiguraatiosta käynnistyksessä. `refresh_token_key_id` mahdollistaa avainrotaation: vanha avain purkaa, uusi avain salaa.
- **Avainrotaatio:** Tuki decrypt-old/encrypt-new -patternille. Batch-migraatio kun avain rotoidaan.

---

## 6. Tietosuoja ja GDPR

Kalenteridata on henkilötietoa GDPR:n alaisuudessa. Kalenteritapahtumat voivat paljastaa arkaluontoista tietoa: asiakastapaamisia, terveyskäyntejä, HR-keskusteluja, liiketoimintatietoja.

### Periaatteet

1. **Datan minimointi:** Haetaan vain tarvittavat kentät. MVP:ssä ei haeta `attendees`-tietoja.
2. **Yksityiset tapahtumat:** `sensitivity: private` tai `confidential` -tapahtumat käsitellään erikseen:
   ```rust
   match event.sensitivity.as_deref() {
       Some("private") | Some("confidential") => CalendarEvent {
           subject: None,        // ei paljasteta
           location: None,       // ei paljasteta
           // vain aika ja busy/oof -tila säilytetään
           ..
       },
       _ => { /* normaali käsittely */ }
   }
   ```
3. **Tallennus vs. on-demand:** MVP:ssä kalenteridata haetaan on-demand eikä tallenneta pysyvästi. Lyhytaikainen cache (TTL 5 min) per käyttäjä/aikaväli.
4. **AI-dataminimointi:** Kalenteritapahtumia ei lähetetä LLM:lle sellaisenaan. Muunnetaan minimaaliseksi kontekstiksi:
   ```
   2026-04-15 09:00-11:00, Helsinki, kokous, busy
   ```
5. **Irtikytkentä:** Käyttäjä voi irrottaa kalenteriyhteyden milloin tahansa → refresh token poistetaan, cached data poistetaan.
6. **Audit trail:** Kalenteridatan haut lokitetaan (käyttäjä, aikaväli, tapahtumamäärä — ei sisältöä).
7. **Tietosuojaseloste:** Päivitettävä ennen kalenteriominaisuuden julkaisua.
8. **LLM-prosessoinnin sijainti:** Dokumentoitava onko LLM-palvelu EU:ssa vai sen ulkopuolella.

### AI-agentin rajoitukset

```rust
const MAX_LOOKUP_RANGE_DAYS: i64 = 45;
const MAX_EVENTS_RETURNED_TO_AGENT: usize = 50;
```

AI-agentti ei saa hakea rajattomia aikavälejä. Tapahtumat tiivistetään ennen LLM:lle lähettämistä.

---

## 7. Virheenkäsittely ja luotettavuus

### Graph API -virhekoodit

```rust
enum CalendarError {
    /// 401 — token vanhentunut tai peruutettu
    /// Yritä refresh kerran, sitten merkitse reauth_required
    InvalidAuth,

    /// 403 — consent puuttuu, admin policy estää, conditional access
    /// Älä yritä uudelleen
    AccessDenied { code: String, message: String },

    /// 404 — kalenteri poistettu tai käyttäjä ei ole Exchange Online -käyttäjä
    NotFound,

    /// 429 — throttling, kunnioita Retry-After
    Throttled { retry_after_secs: u64 },

    /// 5xx — väliaikainen virhe, exponential backoff
    ServerError { status: u16 },

    /// Serde/verkkovirhe
    Transport(String),
}
```

### Retry-politiikka

```rust
struct RetryPolicy {
    max_attempts: usize,        // 3
    base_delay: Duration,       // 1s
    max_delay: Duration,        // 30s
    jitter: bool,               // true
    retry_statuses: [429, 502, 503, 504],
    respect_retry_after: bool,  // true (429)
    total_timeout: Duration,    // 60s
}
```

- **429:** Kunnioita `Retry-After`-headeria (voi olla sekunteina tai HTTP-date)
- **401:** Refresh token kerran, jos se epäonnistuu → `reauth_required`
- **403:** Ei retryä — consent/politiikkaongelma
- **5xx:** Exponential backoff + jitter

### Observability

- **Metriikkoja:** yhteyksien määrä, refresh success/failure rate, Graph-kutsujen latenssi, 429-rate, haettujen tapahtumien määrä
- **Lokitus:** Ei koskaan logita tokeneita, authorization codeja, tai tapahtumien sisältöä (subject, location). Logita: käyttäjä-ID, provider, status code, tapahtumamäärä, aikaväli.

---

## 8. Arkkitehtuuri ja sijoittuminen

Kalenteriintegraatio **ei kuulu `services/email`-palveluun**. Email-palvelu monitoroi IMAP:ia ja lähettää vastauksia. Kalenterin OAuth-flow ja token-hallinta ovat käyttäjäkohtaisia web/API-toimintoja.

```
services/api (tai erillinen calendar-service)
  ├── OAuth-reitit (/auth/microsoft/*, /auth/microsoft/callback)
  ├── Calendar connection management (CRUD, disconnect)
  ├── MicrosoftCalendarClient (Graph API -kutsut)
  └── AI tool endpoint (lookup_calendar)

services/email
  └── Kutsuu AI/kontekstipalvelua tarvittaessa, EI omista kalenteritokeneita
```

### Konfiguraatio

```rust
struct MicrosoftCalendarConfig {
    client_id: String,
    client_secret: String,       // SOPS/age-salattu
    redirect_uri: String,        // ympäristökohtainen
    tenant_mode: String,         // "organizations"
    graph_base_url: String,      // "https://graph.microsoft.com/v1.0" (testissä mock)
    max_events_per_lookup: usize,
    max_pages: usize,
    request_timeout: Duration,
}
```

---

## 9. MVP-suunnitelma

### Vaihe 1: Infrastruktuuri ja OAuth (arvio: 1-1.5 viikkoa)

1. Entra ID app registration (multi-tenant organizations, `Calendars.Read`, verified publisher -prosessin aloitus)
2. DB-migraatiot: `oauth_states`, `calendar_connections`
3. Token-salausmoduuli (AEAD, key rotation)
4. OAuth 2.0 Authorization Code + PKCE endpointit
5. Callback endpoint — state-validointi, code → tokens, token-tallennus
6. Token refresh -mekanismi (single-flight, rotation, invalid_grant → reauth)
7. Admin-consent endpoint ja dokumentaatio

### Vaihe 2: Kalenterin lukeminen (arvio: 1 viikko)

1. `MicrosoftCalendarClient` struct (konkreettinen, ei traittia)
2. `calendarView` endpoint -kutsu (reqwest, URL builder)
3. Graph virhevastausten käsittely (`GraphErrorResponse`)
4. Sivutuksen käsittely (max pages, max events, nextLink-validointi)
5. Aikavyöhykekäsittely (UTC request, `CalendarEventTime` enum, `chrono-tz`)
6. Yksityisten tapahtumien suodatus (`sensitivity`)
7. Retry/backoff -middleware (`reqwest-middleware`)

### Vaihe 3: Integrointi ja testaus (arvio: 1-1.5 viikkoa)

1. `lookup_calendar` tool AI-agentille (rajattu aikaväli, max tapahtumat)
2. Kalenteritapahtumien minimointi ennen LLM:lle lähettämistä
3. Irtikytkentä-endpoint (token poisto, cache tyhjennys)
4. Yhteyden tilan seuranta ja UX (connected/reauth_required)
5. Integraatiotestit (wiremock: OAuth callback, pagination, 429, 401→refresh, yksityiset tapahtumat)
6. Testaus vähintään 2 oikealla Microsoft-tenantilla
7. Tietosuojaselosteen päivitys

### Kokonaisarvio MVP: 3-4 viikkoa

Prototyyppi (happy path): 4-7 päivää.
Tuotanto-MVP (turvallisuus, virheenkäsittely, tietosuoja, testit, observability): 3-4 viikkoa yhdelle kehittäjälle.

---

## 10. Riskit ja huomiot

| Riski | Vaikutus | Mitigaatio |
|---|---|---|
| Microsoft throttling (429) | API-kutsut epäonnistuvat | `reqwest-retry`, `Retry-After`, circuit breaker |
| Refresh token peruutettu (salasanan vaihto, admin, CA policy) | Käyttäjä menettää yhteyden | `reauth_required`-tila, proaktiivinen ilmoitus, helppo re-auth |
| Tenant admin estää sovelluksen | Yritysasiakas ei saa käyttöön | Admin-consent endpoint, setup-dokumentaatio, verified publisher |
| Yksityiset tapahtumat (`sensitivity: private`) | Tietosuojaongelma | Suodatetaan: vain aika + busy/oof-tila käytetään |
| Token-vuoto | Turvallisuusriski | AEAD-salaus, key rotation, ei logitusta, lyhyet access tokenin elinajat |
| Aikavyöhykevirheet | Väärät päivärahat/matkapäivät | UTC-haku, `CalendarEventTime` enum, `chrono-tz`, erillinen all-day-käsittely |
| Oletuskalenteri ei riitä | Matkatapahtumat toisessa kalenterissa | Dokumentoitu rajoitus, myöhemmin `/me/calendars` -tuki |
| AI tulkitsee väärin | Väärät matkakulut automaattisesti | AI esittää ehdotuksia, ei luo kuluja automaattisesti. Käyttäjävahvistus. |
| Verified publisher -viive | Onboarding-este | Aloita prosessi heti, voi kestää viikkoja |
| `invalid_grant` hiljaisesti | Kalenteridata puuttuu, AI tuottaa heikompia raportteja | Yhteyden terveystarkistus, näkyvä status käyttäjälle |

---

## 11. Suositus

**Toteuta MVP `reqwest` + `oauth2` -crateilla suorana REST-integraationa.**

- `graph-rs-sdk` on mielenkiintoinen mutta liian riskikas MVP:lle (pieni yhteisö, yhden henkilön ylläpito)
- Suora REST-toteutus on ~800-1500 riviä Rustia (OAuth, client, tyypit, virheenkäsittely, testit)
- Sama arkkitehtuuri skaalautuu Google Calendariin — yhteinen trait ekstrahoidaan silloin
- Delegated permissions + PKCE + client_secret on turvallinen B2B-käyttöön
- Admin-consent flow dokumentoitava ja toteutettava B2B-onboardingia varten
- Kalenteridata on henkilötietoa — tietosuoja suunniteltava ennen toteutusta
- Aikavyöhykekäsittely on kriittinen matkalaskupalvelussa — UTC + chrono-tz + erillinen all-day-malli
