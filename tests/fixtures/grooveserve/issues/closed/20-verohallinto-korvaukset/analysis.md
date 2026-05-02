# Verohallinnon korvausmäärät — analyysi

## Tausta

Matkalaskupalvelu tarvitsee Verohallinnon viralliset verovapaat matkakustannusten korvausmäärät. Verohallinto vahvistaa nämä vuosittain päätöksellään (esim. vuoden 2026 päätös: säädöskokoelma 970/2025). Päätös kattaa:

- **Kotimaan päivärahat**: osapäiväraha 25 €, kokopäiväraha 54 € (2026)
- **Km-korvaukset**: henkilöauto 0,55 €/km + lisät (perävaunu, koirankuljetus, metsäautotie jne.)
- **Muut ajoneuvot**: moottoripyörä, mopo, moottorikelkka, mönkijä, vene (eri luokat)
- **Ateriakorvaus**: 13,50 € (2026)
- **Yömatkaraha**: 16,00 €
- **Ulkomaan päivärahat**: ~200 maata, 29–134 €/vrk
- **Vapaa majoitus -alennus**: 50 % päivärahasta

## Tietolähteet

### 1. Finlex Open Data API (suositeltu)

Verohallinnon päätös julkaistaan Suomen säädöskokoelmassa ja on saatavilla Finlex-rajapinnasta Akoma Ntoso XML -muodossa.

**Endpoint-esimerkki (vuoden 2026 päätös):**
```
GET https://opendata.finlex.fi/finlex/avoindata/v1/akn/fi/act/statute/2025/970/fin@
Accept: application/xml
User-Agent: Grooveserve/1.0
```

**Edut:**
- Virallinen, auktoritatiivinen lähde (säädöskokoelma)
- Avoin API, ei rekisteröintiä tai autentikointia
- Akoma Ntoso XML on standardoitu rakenne
- Ilmainen, Creative Commons -lisenssi
- REST API, HTTPS, TLS 1.2+

**Haitat:**
- Akoma Ntoso on juridinen dokumenttistandardi — data on tekstimuotoista, ei taulukkomuotoista
- Korvausmäärätaulukot ovat XML:n sisällä tekstinä, vaatii parsimista
- Ulkomaan päivärahat ovat liitteenä (attachment), mahdollisesti eri rakenteella
- XML-rakenteen vakaus vuosien välillä ei ole taattu
- Ei erityistä endpointia korvausmäärille — koko päätösdokumentti haettava

**Tekninen arvio:** Haettavissa `reqwest`-kirjastolla, XML parsittavissa `quick-xml` tai `roxmltree` -crateilla. Varsinainen haaste on korvausmäärätaulukoiden erottaminen juridisesta tekstistä — vaatii joko regex/pattern matching tai XML-puun navigointia.

### 2. vero.fi HTML-sivu

Verohallinto julkaisee päätöksen myös vero.fi-sivustolla HTML-muodossa:
```
https://www.vero.fi/en/detailed-guidance/decisions/47405/tax-exempt-allowances-in-2026-for-business-travel/
```

**Edut:**
- Ihmisluettava, hyvin jäsennelty HTML
- Taulukot selkeässä muodossa
- URL-rakenne on ollut vakaa vuosien välillä

**Haitat:**
- Ei virallinen API — HTML-rakenne voi muuttua milloin tahansa
- Web scraping on hauras
- Ei koneluettavaa rajapintaa
- Rate-limiting ja käyttöehdot epäselviä

**Tekninen arvio:** Scrapettavissa `reqwest` + `scraper`/`select` -crateilla. Epäluotettava pitkällä aikavälillä.

### 3. Manuaalinen datasyöttö + JSON/TOML-tiedosto

Korvausmäärät muuttuvat kerran vuodessa (joulukuussa seuraavalle vuodelle). Data on suhteellisen pieni:
- ~10 kotimaan korvauslajia
- ~10 km-korvauslajia (eri ajoneuvot + lisät)
- ~200 maan ulkomaan päivärahat

**Edut:**
- Yksinkertaisin toteutus
- Täysi kontrolli datarakenteeseen
- Ei ulkoisia riippuvuuksia ajonaikana
- Helppo testata ja validoida
- Voidaan versioida gitissä

**Haitat:**
- Manuaalinen päivitys kerran vuodessa
- Inhimillisen virheen riski
- Vaatii prosessin päivityksen seurantaan

**Tekninen arvio:** TOML tai JSON, deserializointi `serde`-crateilla. Triviaalisti toteutettavissa.

### 4. Hybridimalli: Finlex + staattinen data

Yhdistelmä jossa:
1. Korvausmäärät tallennetaan staattisena datana (JSON/TOML) repositorioon
2. Finlex API:a käytetään validointiin ja uuden vuoden datan haun automatisointiin
3. CI/CD-job tai admin-työkalu hakee Finlex:stä, parsii, ja generoi päivitetyn datatiedoston
4. Ihminen tarkistaa ja committaa

## Olemassaolevat kirjastot ja palvelut

### Rust-ekosysteemi

Ei löydy valmista Rust-cratea Suomen matkakorvausmäärille. Toteutus tehtävä itse.

### Kilpailijoiden ratkaisu

Suomalaiset matkalaskusovellukset (eTasku, Bezala, Rydoo) näyttävät käyttävän **manuaalista päivitystä** — ne tutkivat vuosittain Verohallinnon päätöksen ja päivittävät omat järjestelmänsä. Mitään julkista API-integraatiota ei mainita.

### Verohallinnon Vero API

Vero API tarjoaa 80+ rajapintaa, mutta ne koskevat verotusprosesseja (verokortti, ilmoitukset, kirjeet). **Korvausmäärille ei ole Vero API -rajapintaa.**

### Verohallinnon avoin data

Verohallinnon avoin data sisältää vain yhteisöjen verotustietoja (CSV). **Korvausmääriä ei ole avoimena datana.**

## Datarakenne

Ehdotettu Rust-tyyppijärjestelmä:

```rust
/// Vuosikohtaiset korvausmäärät
struct CompensationRates {
    year: u16,
    effective_from: NaiveDate,
    domestic: DomesticRates,
    mileage: MileageRates,
    international: Vec<CountryPerDiem>,
}

struct DomesticRates {
    partial_per_diem: Decimal,      // osapäiväraha (6+ h)
    full_per_diem: Decimal,         // kokopäiväraha (10+ h)
    meal_allowance: Decimal,        // ateriakorvaus
    night_travel: Decimal,          // yömatkaraha
    free_meal_reduction_pct: u8,    // 50%
}

struct MileageRates {
    car_per_km: Decimal,            // 0.55 €
    motorcycle_per_km: Decimal,     // 0.42 €
    moped_per_km: Decimal,          // 0.23 €
    // ... lisät
    trailer_addition: Decimal,      // +0.09 €
    caravan_addition: Decimal,      // +0.15 €
    heavy_load_addition: Decimal,   // +0.28 €
    passenger_addition: Decimal,    // +0.04 € per hlö
    dog_addition: Decimal,          // +0.04 €
    forest_road_addition: Decimal,  // +0.12 €
    company_car_per_km: Decimal,    // 0.11 €
}

struct CountryPerDiem {
    country_code: String,           // ISO 3166-1 alpha-2
    country_name_fi: String,
    per_diem: Decimal,
    /// Poikkeukset tietyille kaupungeille (esim. Lontoo ≠ muu UK)
    city_exceptions: Vec<CityException>,
}

struct CityException {
    city: String,
    per_diem: Decimal,
}
```

## Suositus MVP:lle

### Vaihe 1: Staattinen data (MVP)

**Toteutus:** YAML-tiedosto repositoriossa, Rust `serde` + `serde_yaml` -deserialisointi.

Perustelut:
- Nopein toteuttaa (1–2 päivää)
- Ei ulkoisia riippuvuuksia
- Täysin luotettava — ei parsimisvirheitä
- Korvausmäärät muuttuvat kerran vuodessa, joten automaattinen haku ei ole kriittinen MVP:ssä
- Alan vakiintunut käytäntö — myös kilpailijat (eTasku, Bezala, Rydoo) päivittävät korvausmäärät manuaalisesti

**Datatiedosto:** `data/rates/2026.yaml` — luotu tässä issuessa Verohallinnon päätöksestä 970/2025.

**Vuosittainen päivitysprosessi:**

1. Verohallinto julkaisee uuden päätöksen tyypillisesti joulukuussa seuraavalle vuodelle
2. Avaa uusi päätös: `https://www.vero.fi/en/detailed-guidance/decisions/47405/`
3. Kopioi `data/rates/2026.yaml` → `data/rates/2027.yaml`
4. Päivitä kotimaan ja km-korvausluvut (nämä ovat muutaman rivin muutoksia)
5. Käy läpi ulkomaan päivärahat — tyypillisesti 10–30 maata muuttuu vuosittain
6. PR, review, merge, deploy
7. Työmäärä: ~30–60 min kerran vuodessa

Mikään tutkimuksessa löytynyt kilpaileva ratkaisu ei automatisoi tätä. Verohallinnolla ei ole koneluettavaa rajapintaa korvausmäärille, joten manuaalinen tarkistus on alan standardi.

### Vaihe 2: Finlex-validointi (post-MVP)

Kun palvelu on tuotannossa ja luotettavuus kriittistä:
1. CI-job joka hakee Finlex Open Data API:sta tuoreimman päätöksen
2. Parsii Akoma Ntoso XML:stä korvausmäärät
3. Vertaa staattiseen dataan ja hälyttää eroista
4. Optionaalisesti generoi uuden datatiedoston tarkistettavaksi

### Ei suositella MVP:lle

- **Web scraping** (vero.fi/muu) — liian hauras, ei lisäarvoa manuaaliseen verrattuna
- **Täysautomaattinen Finlex-parsinta** — Akoma Ntoso -dokumentin rakenteen luotettava parsiminen on merkittävä työ, ja hyöty on marginaalinen kerran vuodessa tapahtuvaan päivitykseen nähden

## Riskit ja avoimet kysymykset

1. **Finlex XML -rakenteen vakaus**: Akoma Ntoso on standardi, mutta Verohallinnon päätösten sisäinen rakenne (taulukot, liitteet) voi vaihdella vuosittain
2. **Ulkomaan päivärahojen kaupunkipoikkeukset**: Joidenkin maiden kohdalla on kaupunkikohtaisia poikkeuksia (esim. UK: Lontoo 89 €, muu 84 €). Nämä pitää mallintaa
3. **Historiallisten vuosien tarve**: Matkalaskut voivat kohdistua edelliseen vuoteen — tarvitaan vähintään kuluva + edellinen vuosi
4. **Päätöksen julkaisuaikataulu**: Tyypillisesti joulukuussa seuraavalle vuodelle. Tarvitaan prosessi jolla uudet määrät saadaan järjestelmään ajoissa
5. **Valtion virkamiesten korvaukset (KVTES/VES)**: Nämä ovat eri päätös (valtiovarainministeriö), mutta noudattavat samaa pohjaa. Tarvitaanko MVP:ssä?

## Lähteet

- [Verohallinnon päätös 2026 (vero.fi)](https://www.vero.fi/en/detailed-guidance/decisions/47405/tax-exempt-allowances-in-2026-for-business-travel/)
- [Säädöskokoelma 970/2025 (Finlex)](https://www.finlex.fi/en/legislation/collection/2025/970)
- [Finlex Open Data API](https://www.finlex.fi/en/open-data)
- [Finlex API Quick Guide](https://www.finlex.fi/en/open-data/integration-quick-guide)
- [Verohallinnon avoin data](https://www.vero.fi/tietoa-verohallinnosta/tilastot/avoin_dat/)
- [Vero API -rajapinnat](https://www.vero.fi/en/About-us/it_developer/vero-api/what-are-the-vero-api-interfaces/)
- [Ulkomaan päivärahat PDF (rahatieto.fi)](http://www.rahatieto.fi/upr2026.pdf)
- [Kilometrikorvaukset (Veronmaksajat)](https://www.veronmaksajat.fi/neuvot/henkiloverotus/tyo-elake-ja-etuudet/paivarahat-ja-kilometrikorvaukset/kilometrikorvaukset/)
