# Netvisor API — matkalaskuintegraation analyysi

## Yhteenveto

Netvisor tarjoaa REST-tyylisen XML-API:n matkalaskujen vientiä varten. Valmista Rust-kirjastoa ei ole — client täytyy toteuttaa itse. Toteutus on suoraviivainen: HMAC-SHA256-allekirjoitus HTTP-headereissa + XML-payload POST-pyyntönä. MVP:ssä riittää yksi endpoint (`tripexpense.nv`).

## 1. API:n yleiskuva

### Perusarkkitehtuuri

- **Tyyppi**: REST (ei varsinaisesti RESTful — kaikki on POST/GET, ei PUT/DELETE)
- **Dataformaatti**: XML (ei JSON)
- **Base URL**: `https://integration.netvisor.fi/` (tuotanto), testiympäristö erikseen
- **Vastausformaatti**: XML
- **Markkinat**: Suomi (ensisijaisesti)
- **Kohderyhmä**: PK-yritykset, tilitoimistot

### Matkalaskuun liittyvät endpointit

| Endpoint | Metodi | Kuvaus |
|----------|--------|--------|
| `tripexpense.nv` | POST | Matkalaskun vienti (kulurivit, km-korvaukset, päivärahat) |
| `workday.nv` | POST | Työpäiväkirjaus (tunnit) |
| `getrecordtypelist.nv` | GET | Kirjaustyyppien haku |

MVP:lle riittää `tripexpense.nv`. Muut endpointit (laskutus, asiakasrekisteri, dimensiot) ovat hyödyllisiä myöhemmin.

## 2. Autentikointi

### HMAC-SHA256 (uusi tapa, käytettävä)

Jokaisessa HTTP-pyynnössä lähetetään MAC-koodi headereissa. MAC lasketaan yhdistelemällä headerien arvot ja avaimet, ja laskemalla HMAC-SHA256 (tai SHA256 hash) tuloksesta.

**Vaaditut headerit:**

| Header | Kuvaus |
|--------|--------|
| `X-Netvisor-Authentication-Sender` | Integraation nimi (vapaasti valittava) |
| `X-Netvisor-Authentication-CustomerId` | Asiakkaan tunniste |
| `X-Netvisor-Authentication-PartnerId` | Kumppanin tunniste |
| `X-Netvisor-Authentication-Timestamp` | UTC-aikaleima ANSI-muodossa |
| `X-Netvisor-Authentication-TransactionId` | Uniikki GUID joka pyynnölle |
| `X-Netvisor-Authentication-MAC` | Laskettu MAC-koodi |
| `X-Netvisor-Authentication-MACHashCalculationAlgorithm` | `HMACSHA256` |
| `X-Netvisor-Authentication-UseHTTPResponseStatusCodes` | `1` |
| `X-Netvisor-Interface-Language` | `FI` tai `EN` |
| `X-Netvisor-Organisation-ID` | Organisaation Y-tunnus |

**MAC-laskenta:**

```
MAC = SHA256(url & sender & customerId & timestamp & language & organizationId & transactionId & customerKey & partnerKey)
```

Kentät yhdistetään `&`-merkillä. URL:n kirjainkoko täytyy täsmätä täsmälleen. Skandinaaviset merkit URL:ssa koodataan ISO-8859-1:llä.

**Avaimet:**

| Avain | Lähde |
|-------|-------|
| `customerId` + `customerKey` | Asiakas luo Netvisorissa: Yritys → API-tunnisteet |
| `partnerId` + `partnerKey` | Saadaan kumppanirekisteröinnin yhteydessä Vismalta |

### Vanha tapa (MD5)

Vanhempi Python-kirjasto (`fastmonkeys/netvisor.py`) käyttää MD5-hashia. **Ei käytetä** — uudet integraatiot käyttävät HMACSHA256:ta.

## 3. Matkalaskun rakenne (`tripexpense.nv`)

TypeScript-kirjaston (`rantalainen/netvisor-api-client`) rajapinnoista johdettu rakenne. Tämä on kattavin saatavilla oleva dokumentaatio.

### XML-runko

```xml
<root>
  <tripexpense>
    <header>Matkalaskun otsikko</header>
    <description>Vapaaehtoinen kuvaus</description>
    
    <customlines>
      <customline>...</customline>
    </customlines>
    
    <travellines>
      <travelline>...</travelline>
    </travellines>
    
    <dailycompensationlines>
      <dailycompensationline>...</dailycompensationline>
    </dailycompensationlines>
  </tripexpense>
</root>
```

### 3.1 Kulurivit (customLines)

Yleiskäyttöiset kulurivit — majoitus, ruokailu, pysäköinti, jne.

| Kenttä | Tyyppi | Pakollinen | Kuvaus |
|--------|--------|------------|--------|
| `employeeIdentifier` | string + type | Kyllä | Työntekijän tunniste (`number` tai `finnishpersonalidentifier`) |
| `ratio` | string + type=name | Kyllä | Kululajin nimi (esim. "Majoituskulut") |
| `amount` | number | Kyllä | Määrä |
| `customLineUnitPrice` | number | Kyllä | Yksikköhinta (tukee valuuttaa: `iso4217currencycode`, `currencyrate`) |
| `vatPercentage` | number | Ei | ALV-prosentti |
| `lineDescription` | string | Kyllä | Kuvaus |
| `beginDate` | string | Kyllä | Alkupäivä |
| `endDate` | string | Kyllä | Loppupäivä |
| `expenseAccountNumber` | number | Ei | Kirjanpitotilin numero |
| `lineStatus` | enum | Ei | `open`, `confirmed`, `contentsupervisored`, `accepted`, `paid` |
| `dimension` | object | Ei | Dimensio (kustannuspaikka, projekti) |
| `tripExpenseAttachments` | array | Ei | Liitteet (kuittikuvat) |
| `crmProcessIdentifier` | string | Ei | CRM-prosessin tunniste |
| `customerIdentifier` | object | Ei | Asiakkaan tunniste |

### 3.2 Kilometrikorvaukset (travelLines)

| Kenttä | Tyyppi | Pakollinen | Kuvaus |
|--------|--------|------------|--------|
| `employeeIdentifier` | string + type | Kyllä | Työntekijän tunniste |
| `travelType` | enum | Kyllä | Kulkuneuvotyyppi (ks. alla) |
| `passengerAmount` | number | Kyllä | Matkustajien määrä |
| `kilometerAmount` | number | Kyllä | Kilometrit |
| `unitPrice` | number | Ei | Yksikköhinta (yliajetaan Verohallinon oletusmäärä) |
| `lineDescription` | string | Kyllä | Kuvaus |
| `travelDate` | string | Kyllä | Matkapäivä |
| `routeDescription` | string | Kyllä | Reitin kuvaus |
| `lineStatus` | enum | Ei | Status |
| `dimension` | array | Ei | Dimensiot |
| `tripExpenseAttachments` | array | Ei | Liitteet |

**Kulkuneuvotyypit (`travelType`):**

| Arvo | Kuvaus |
|------|--------|
| `car` | Henkilöauto |
| `car_with_trailer` | Auto + perävaunu |
| `car_with_caravan` | Auto + asuntovaunu |
| `car_with_heavy_cargo` | Auto + raskas kuorma |
| `car_with_big_machinery` | Auto + kone/laite |
| `car_with_dog` | Auto + koira |
| `car_travel_in_rough_terrain` | Maastoajo |
| `motorboat_max_50hp` | Vene ≤50 hv |
| `motorboat_over_50hp` | Vene >50 hv |
| `snowmobile` | Moottorikelkka |
| `atv` | Mönkijä |
| `motorbike` | Moottoripyörä |
| `moped` | Mopo |
| `other` | Muu |
| `carbenefit` | Autoetu |

### 3.3 Päivärahat (dailyCompensationLines)

| Kenttä | Tyyppi | Pakollinen | Kuvaus |
|--------|--------|------------|--------|
| `employeeIdentifier` | string + type | Kyllä | Työntekijän tunniste |
| `compensationType` | enum | Kyllä | `domesticfull`, `domestichalf`, `foreign` |
| `amount` | number | Kyllä | Päivärahojen lukumäärä |
| `unitPrice` | number | Ei | Yksikköhinta (yliajetaan oletusmäärä) |
| `lineDescription` | string | Kyllä | Kuvaus |
| `timeOfDeparture` | string | Kyllä | Lähtöaika |
| `returnTime` | string | Kyllä | Paluuaika |
| `lineStatus` | enum | Ei | Status |
| `dimension` | array | Ei | Dimensiot |

### 3.4 Liitteet

```xml
<tripExpenseAttachments>
  <tripExpenseAttachment>
    <mimeType>image/jpeg</mimeType>
    <attachmentDescription>Hotellikuitti</attachmentDescription>
    <fileName>kuitti.jpg</fileName>
    <documentData>BASE64-ENCODED-DATA</documentData>
  </tripExpenseAttachment>
</tripExpenseAttachments>
```

Liitteet lähetetään base64-koodattuina XML:n sisällä.

## 4. Olemassa olevat kirjastot

### Kolmannen osapuolen Netvisor API -clientit

| Kieli | Repo | Matkalaskutuki | Ylläpito |
|-------|------|----------------|----------|
| TypeScript | `rantalainen/netvisor-api-client` | Kyllä (`tripExpense()`) | Aktiivinen (146 committia) |
| Python | `fastmonkeys/netvisor.py` | Ei | Vanha (MD5-auth) |
| Python | `Heltti/netvisor-api-client` | ? | ? |
| Ruby | `eficode/netvisor` | ? | ? |
| PHP | `xi-project/xi-netvisor` | Laskutus | Vanha |
| **Rust** | **— ei ole —** | — | — |

### TypeScript-kirjaston arkkitehtuurihuomiot

`rantalainen/netvisor-api-client` tarjoaa hyödyllisiä arkkitehtuurioppeja:

1. **XML-elementtien järjestys on kriittinen** — Netvisor vaatii elementit tietyssä järjestyksessä. TypeScript-kirjasto generoi järjestyskartan käännösaikana TypeScript-rajapinnoista.
2. **Geneerinen XML-fallback** — `saveByXmlData()` ja `getXmlData()` metodit raaka-XML:lle
3. **Vastauksen jäsennys** — `tripExpense()` palauttaa `inserteddataidentifier`-arvon vastauksesta

## 5. Rust-toteutuksen realistisuus

### Tarvittavat cratet

| Crate | Käyttö | Kypsyys |
|-------|--------|---------|
| `reqwest` | HTTP-client | Erittäin kypsä |
| `hmac` + `sha2` | HMAC-SHA256-allekirjoitus | Erittäin kypsä (RustCrypto) |
| `quick-xml` | XML-serialisointi/deserialisointi | Kypsä, serde-tuki |
| `uuid` | TransactionId-generointi | Erittäin kypsä |
| `chrono` | Aikaleima-muotoilu | Erittäin kypsä |
| `base64` | Liitteiden koodaus | Erittäin kypsä |

Kaikki tarvittavat cratet ovat kypsiä ja laajasti käytettyjä. Ei tarvetta mihinkään kokeelliseen.

### Arkkitehtuuri

```
netvisor/
├── client.rs       # NetvisorClient — HTTP + auth
├── auth.rs         # MAC-laskenta (HMAC-SHA256)
├── models/
│   ├── tripexpense.rs  # TripExpense, CustomLine, TravelLine, DailyCompensation
│   └── common.rs       # EmployeeIdentifier, Dimension, Attachment
├── error.rs        # Virheenkäsittely
└── xml.rs          # XML-serialisointi (serde + quick-xml)
```

### Työmääräarvio

| Komponentti | Monimutkaisuus | Huomiot |
|-------------|----------------|---------|
| Auth (MAC-laskenta) | Pieni | ~100 riviä, suoraviivainen hash |
| HTTP-client | Pieni | reqwest + headerit |
| XML-serialisointi | Keskisuuri | Elementtijärjestys kriittinen, attribuutit |
| TripExpense-mallit | Keskisuuri | Paljon kenttiä, mutta mekaaninen työ |
| Virheenkäsittely | Pieni | XML-vastauksen jäsennys |
| Testit | Keskisuuri | Integraatiotestit vaativat testiympäristön |

**Kokonaisarvio**: Noin 1000–1500 riviä Rust-koodia MVP:lle. Suurin haaste on XML-elementtien oikea järjestys ja testaus oikeaa API:a vasten.

### Riskit

1. **XML-elementtien järjestys** — Netvisor hylkää pyynnön jos elementit ovat väärässä järjestyksessä. `quick-xml` + serde ei takaa järjestystä oletuksena. Ratkaisu: `#[serde(rename)]` ja manuaalinen järjestys tai `BTreeMap`.
2. **Dokumentaation puutteet** — Virallinen dokumentaatio on JS-renderoityjä tukiartikkeleita joita ei voi lukea ohjelmallisesti. TypeScript-kirjasto on käytännössä paras "dokumentaatio".
3. **Testiympäristö** — Kumppanirekisteröinti (lomake Vismalle) vaaditaan testiavainten saamiseksi. Tämä voi kestää.
4. **ISO-8859-1-koodaus URL:ssa** — MAC-laskennassa skandinaaviset merkit URL:ssa koodataan ISO-8859-1:llä. Harvinainen erikoistapaus, mutta huomioitava.

## 6. Hinnoittelu

Netvisorin hinnoittelu 2026 (ilman ALV):

### Pakettihinnat (kuukausimaksu, liikevaihtoluokittain)

Grooveserven asiakkaiden Netvisor-paketin valinta vaikuttaa integraation mahdollisuuksiin:

- **Professional** (81–3160 €/kk): API-käyttö sisältyy
- **Premium** (118–3625 €/kk): API-käyttö sisältyy
- **Palkat** (12 €/kk): Pelkkä palkat, API-käyttö sisältyy
- Light/Basic/Starter/Core: API ei välttämättä saatavilla

### Käyttäjäkohtaiset maksut (matkalaskumoduuli)

| Henkilöstömäärä | Hinta |
|-----------------|-------|
| 1–5 | 6,50 €/hlö/kk |
| 6–100 | 4,00 €/hlö/kk |
| 101–200 | 3,00 €/hlö/kk |
| 200+ | 2,30 €/hlö/kk |

Tämä on Netvisorin oma kustannus asiakkaalle — ei Grooveserven lisämaksu. Asiakkaalla täytyy olla "Invoicing for travel and related expenses" -moduuli käytössä.

### API-transaktiomaksut

API-kutsuille ei näy erillistä transaktiomaksua hinnastossa. API-käyttö sisältyy Professional/Premium/Palkat-paketteihin.

### Kumppanuus

Integraatiokumppanuuden (Software Partnership) rekisteröinti on ilmainen. Testiympäristö ja -tunnukset saadaan kumppanilomakkeen kautta.

## 7. Kumppaniprosessi

1. **Rekisteröidy kumppanilomakkeella** — Visma Developer -portaalissa
2. **Saat testiympäristön** — tunnukset sähköpostiin
3. **Kehitä integraatio** — testiympäristössä
4. **Netvisor Community** — keskustelupalsta API-muutoksista ja -uutisista
5. **Marketplace** — valmis integraatio voidaan listata Netvisorin MarketPlaceen

## 8. Suositus MVP:lle

### Minimaalisin toteutus

1. **Yksi endpoint**: `tripexpense.nv` (POST) — kattaa kaikki matkalaskun osat
2. **Kolme rivityyppiä**:
   - `customLines` — kulurivit (majoitus, ruokailu, muut)
   - `travelLines` — km-korvaukset
   - `dailyCompensationLines` — päivärahat
3. **Liitteet**: base64-koodatut kuitit
4. **Auth**: HMAC-SHA256

### Mitä EI tarvita MVP:ssä

- Netvisorin asiakasrekisterin synkronointia
- Dimensioiden hallintaa (voidaan kovakoodata aluksi)
- Matkalaskujen lukemista Netvisorista (vain kirjoitus)
- Hyväksyntäkiertoa Netvisorissa (käytetään omaa)
- Workday-kirjauksia

### Seuraavat askeleet

1. **Rekisteröidy Netvisor-kumppaniksi** — lomake Visma Developer -portaalissa
2. **Hanki testiympäristö** ja API-tunnukset
3. **Toteuta auth-moduuli** — HMAC-SHA256 + headerit (~100 riviä)
4. **Toteuta tripexpense-mallit** — Rust-structit + XML-serialisointi
5. **Testaa testiympäristössä** — lähetä yksinkertainen matkalaskun
6. **Integroi email-serviceen** — AI-agentti koostaa matkalaskun → Netvisor-vienti

### Arkkitehtuuripäätös

Netvisor-client toteutetaan omana cratenaan (`netvisor-client`) joka on riippumaton email-servicestä. Tämä mahdollistaa:
- Yksikkötestauksen erillään
- Uudelleenkäytön muissa palveluissa
- Selkeän vastuunjaon

## Lähteet

- [Netvisor API -portaali](https://support.netvisor.fi/en/support/solutions/77000205228)
- [Visma Developer — Netvisor API](https://developer.visma.com/api/netvisor)
- [API-autentikointi](https://support.netvisor.fi/en/support/solutions/articles/77000557880-api-authentication)
- [tripexpense.nv-dokumentaatio](https://support.netvisor.fi/en/support/solutions/articles/77000554279-import-travel-expense-tripexpense-nv)
- [rantalainen/netvisor-api-client (TypeScript)](https://github.com/rantalainen/netvisor-api-client) — kattavin avoin lähdekoodi, käytetty rajapintamäärittelyjen referenssinä
- [fastmonkeys/netvisor.py (Python)](https://github.com/fastmonkeys/netvisor.py) — auth-logiikan referenssi (MD5, vanha tapa)
- [Netvisor-hinnasto 2026](https://netvisor.fi/download/pricing/eng/netvisor-pricelist-1-2026.pdf)
- [Kumppanirekisteröinti](https://support.netvisor.fi/en/support/solutions/articles/77000466610-implementing-new-integration-and-netvisor-software-partnership)
- [Postman-kokoelma (HMACSHA256)](https://www.postman.com/postman-enterprise-for-visma-group-1/visma-solutions-oy-netvisor-api-flow-examples/collection/f33bufy/netvisor-api-hmacsha256-example)
