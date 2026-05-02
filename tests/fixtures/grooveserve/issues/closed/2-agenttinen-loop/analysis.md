# Agenttinen loop — teknologiatutkimus

_Tutkimus 2026-04-25. Vertailu Rust AI -kirjastoista ja arkkitehtuurivaihtoehdoista matkalaskupalvelun agenttiseen looppiin._

## Yhteenveto

**Suositus: oma toteutus `reqwest` + `serde` -pohjalla**, Anthropic Messages API:n päälle. Tämä on sama malli jota goose (Block) ja muut tuotantopalvelut käyttävät. Framework (rig) on vaihtoehto jos halutaan nopeampi prototyyppi tai monimallituki.

**Malli: Claude Haiku 4.5** (edullisin riittävän älykäs malli) tai **Claude Sonnet 4.6** (parempi päättely). Kustannus ~$0.01–0.04/sähköposti.

---

## 1. Rust AI -kirjastot

### Aktiiviset ja käyttökelpoiset

| Kirjasto | Stars | Tyyppi | Tool use | Streaming | Agenttinen loop | Ylläpito |
|----------|-------|--------|----------|-----------|-----------------|----------|
| **rig** | ~7 050 | Framework | Kyllä | Kyllä | Kyllä | Erittäin aktiivinen |
| **genai** | ~750 | Client-kirjasto | Kyllä | Kyllä | Ei (rakennat itse) | Aktiivinen |
| **async-openai** | ~1 850 | Client-kirjasto | Kyllä | Kyllä | Ei | Aktiivinen |
| **anthropic-rs** | ~75 | Client-kirjasto | Kyllä | Kyllä | Ei | Aktiivinen |
| **misanthropy** | ~35 | Client-kirjasto | Kyllä | Kyllä | Ei | Aktiivinen |
| **swiftide** | ~690 | Framework (RAG) | Kyllä | Kyllä | Kyllä | Aktiivinen |
| **goose** | ~43 000 | Sovellus | Kyllä | Kyllä | Kyllä | Erittäin aktiivinen |
| **aichat** | ~9 900 | Sovellus | Kyllä | Kyllä | Kyllä | Aktiivinen |

### Hylätyt / ei-sopivat

| Kirjasto | Syy |
|----------|-----|
| **llm-chain** | Hylätty, viimeinen commit 10/2024 |
| **langchain-rust** | Hidastunut kehitys, vanhat riippuvuudet |
| **swarms-rs** | Varhainen vaihe, ei tuotantovalmis |
| **mistral.rs** | Vain lokaali inferenssi, ei cloud-API |
| **agentrs** | Uusi, nolla käyttäjiä |

### Huomioita

- **Virallista Anthropic Rust SDK:ta ei ole.** Kaikki Rust-clientit ovat yhteisön ylläpitämiä.
- **goose ja aichat ovat sovelluksia**, eivät kirjastoja. Niiden arkkitehtuurista voi oppia, mutta niitä ei voi importata.
- **rig** on selvästi kypsyin Rust AI -framework: 25+ provideria, MCP-tuki, derive-makrot tool-määrittelyihin.

---

## 2. Arkkitehtuurivaihtoehdot

### Vaihtoehto A: Suora API-kutsu (reqwest + serde)

Oma `AnthropicClient` joka kutsuu Messages API:a suoraan.

```
reqwest::Client → https://api.anthropic.com/v1/messages
  + omat serde-tyypit (Message, ContentBlock, ToolUse, ToolResult)
  + agenttinen loop: while stop_reason == "tool_use" { ... }
```

**Työmäärä**: ~300–400 riviä (tyypit + client + streaming + loop).

**Plussat:**
- Täysi kontrolli: prompt caching, tool-määrittelyt, virheenkäsittely
- Ei riippuvuutta yhteisöcrateihin jotka voivat vanhentua
- Goose (43k stars, Block/Square) käyttää tätä mallia tuotannossa
- Anthropic Messages API on yksinkertainen: yksi endpoint, hyvin dokumentoitu JSON

**Miinukset:**
- Enemmän aloitustyötä kuin framework
- Streaming-parsinta (SSE) vaatii toteutusta
- Jos halutaan myöhemmin vaihtaa provideria, vaatii muutoksia

**Riippuvuudet:**
```toml
reqwest = { version = "0.12", features = ["json", "stream"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio-stream = "0.1"  # SSE-parsinta
```

### Vaihtoehto B: Kevyt framework (rig)

```rust
let agent = anthropic_client
    .agent("claude-haiku-4-5-20251001")
    .preamble("Olet matkalaskuassistentti...")
    .tool(ParseReceiptTool)
    .tool(LookupCalendarTool)
    .build();

let response = agent.chat("Tässä kuittini...", chat_history).await?;
```

**Plussat:**
- Nopein tie toimivaan prototyyppiin
- Sisäänrakennettu tool use, streaming, agenttinen loop
- 25+ provideria jos halutaan vaihtaa
- Derive-makrot tool-skeemoille (`#[derive(Tool)]`)
- MCP-tuki (rmcp-integraatio)

**Miinukset:**
- Abstraktiokerros vaikeuttaa hienosäätöä (prompt caching, custom headers)
- Framework-riippuvuus: jos rig:n kehitys hidastuu, paine vaihtaa
- v0.35 — API voi vielä muuttua
- Raskas riippuvuuspuu (schemars, async-stream, eventsource-stream...)

**Riippuvuudet:**
```toml
rig-core = { version = "0.35", features = ["anthropic"] }
```

### Vaihtoehto C: Anthropic-client-crate (misanthropy / anthropic-rs)

Käytetään valmista Anthropic-clientiä, mutta rakennetaan oma agenttinen loop.

**Plussat:**
- Valmiit tyypit ja streaming-parsinta
- Vähemmän boilerplatea kuin vaihtoehto A
- Voi keskittyä liiketoimintalogiikkaan

**Miinukset:**
- Pienet yhteisöprojektit (35–75 tähteä), hylkäämisriski
- Ei välttämättä tue uusimpia API-ominaisuuksia (prompt caching, batch)
- Rajapinta ei ole omassa hallinnassa

### Vaihtoehto D: Raskas framework (langchain-rust)

**Ei suositella.** Kehitys hidastunut, vanhat riippuvuudet, turhaa monimutkaisuutta yksinkertaiseen käyttötapaukseen.

---

## 3. Vertailutaulukko

| Kriteeri | A: reqwest+serde | B: rig | C: client-crate | D: langchain-rust |
|----------|-----------------|--------|-----------------|-------------------|
| Aloitustyö | Keskisuuri | Pieni | Pieni–keskisuuri | Keskisuuri |
| Kontrolli | Täysi | Rajoitettu | Hyvä | Rajoitettu |
| Ylläpitoriski | Matala (oma koodi) | Keskisuuri | Korkea | Korkea |
| Prompt caching | Helppo | Vaikea | Riippuu | Ei tukea |
| Monimallituki | Vaatii työtä | Sisäänrakennettu | Ei | Rajallinen |
| Tuotantovalmius | Korkea | Keskisuuri | Matala | Matala |

---

## 4. Agenttinen loop -arkkitehtuuri

### Perusloop (kaikille vaihtoehdoille sama)

```
1. Vastaanota sähköposti (IMAP IDLE, nykyinen email-service)
2. Lataa konteksti: käyttäjäprofiili, matkalaskuluonnos, ketjun historia
3. Kokoa viesti LLM:lle:
   - System prompt (cachetettava) + tools
   - Käyttäjäprofiili
   - Nykyinen matkalaskuluonnos (JSON)
   - Uusi sähköpostiviesti
4. Kutsu LLM:ää
5. Jos stop_reason == "tool_use":
   - Suorita tool(t) (OCR, kalenteri, verotaulukko...)
   - Lisää tulokset keskusteluun
   - Palaa kohtaan 4
6. Jos stop_reason == "end_turn":
   - Lähetä vastaus sähköpostilla
   - Tallenna tila tietokantaan
```

### Tool-määrittelyt matkalaskupalvelulle

| Tool | Kuvaus | Prioriteetti |
|------|--------|-------------|
| `parse_receipt` | Kuitin OCR + rakenteinen purku (myyjä, summa, ALV, päivä) | MVP |
| `get_user_profile` | Käyttäjän tiedot (kotipaikka, tyypilliset reitit) | MVP |
| `lookup_tax_rates` | Verohallinnon päiväraha- ja km-korvausmäärät | MVP |
| `calculate_per_diem` | Päivärahan laskenta (kohde + kesto) | MVP |
| `create_expense_report` | Matkalaskun generointi | MVP |
| `lookup_calendar` | Kalenteritiedot matkakontekstin päättelyksi | Post-MVP |
| `send_for_approval` | Hyväksyntäkierron käynnistys | Post-MVP |
| `query_expense_history` | Aiempien matkalaskujen haku oletusarvoiksi | Post-MVP |

### Keskusteluhistorian hallinta

Matkalaskukäyttötapauksessa paras strategia on **rakenteinen tilaobjekti**:

```json
{
  "thread_id": "abc123",
  "user_id": "jari",
  "status": "draft",
  "receipts": [
    {"vendor": "VR", "amount": 45.50, "date": "2026-04-20", "vat": 4.64}
  ],
  "trips": [
    {"destination": "Tampere", "start": "2026-04-20", "end": "2026-04-21"}
  ],
  "missing_info": ["paluupäivä Tampereen matkalta"],
  "expense_report": null
}
```

Tämä pitää token-käytön vakiona riippumatta sähköpostien määrästä ketjussa. LLM saa: system prompt + käyttäjäprofiili + luonnos-JSON + uusin viesti.

### Virheenkäsittely

- **Max iterations**: kovakoodattu raja (esim. 10) estää loputtomat loopit
- **No-progress detection**: jos sama tool-kutsu toistuu, lopeta
- **Tool-virheet**: palautetaan LLM:lle `is_error: true` -lipulla, LLM päättää miten jatkaa
- **Kustannusbudjetti**: max tokenit per sähköposti (esim. 50 000)

---

## 5. Mallin valinta ja kustannukset

### Hinnat (huhtikuu 2026)

| Malli | Input/1M | Output/1M | Cache read/1M | Per sähköposti* |
|-------|----------|-----------|---------------|-----------------|
| Claude Haiku 4.5 | $1.00 | $5.00 | $0.10 | ~$0.013 |
| Claude Sonnet 4.6 | $3.00 | $15.00 | $0.30 | ~$0.040 |
| GPT-4o-mini | $0.15 | $0.60 | — | ~$0.002 |
| GPT-4.1-nano | ~$0.10 | ~$0.40 | — | ~$0.001 |

*Arvio: 3 kutsua × 2000 input + 500 output tokenia, ilman cachea.

### Kuukausikustannus (10 000 sähköpostia/kk)

| Malli | Ilman cachea | Prompt cachella |
|-------|-------------|-----------------|
| Claude Haiku 4.5 | ~$135 | ~$40 |
| Claude Sonnet 4.6 | ~$405 | ~$130 |
| GPT-4o-mini | ~$18 | — |

### Suositus

**Claude Haiku 4.5** on paras tasapaino älykkyyden ja hinnan välillä. Suomenkielinen kuittien ja matkatietojen ymmärtäminen vaatii enemmän kuin GPT-4o-mini tarjoaa, mutta Sonnetin taso ei ole välttämätön rutiinilaskuille.

Prompt caching on kriittinen optimointi: system prompt + tools + käyttäjäprofiili (yhteensä ~1500 tokenia) cachetettuna 90% halvemmalla.

Vaihtaminen mallien välillä on helppoa jos käyttää omaa API-clientiä (vaihtoehto A).

---

## 6. Suositus

### Ensisijainen: Vaihtoehto A (reqwest + serde)

**Perustelut:**

1. **Kontrolli** — matkalaskupalvelu tarvitsee tarkkaa kontrollia prompt caching -strategiasta, tool-skeemoista ja virheenkäsittelystä. Framework piilottaa nämä.

2. **Yksinkertaisuus** — Anthropic Messages API on yksi endpoint. "Framework" tähän on ~400 riviä omaa koodia. Tämä on vähemmän kuin rig:n opettelu ja sen rajoitusten kiertäminen.

3. **Ylläpidettävyys** — ei riippuvuutta yhteisön crate-ylläpitäjistä. Goose (43k stars) tekee saman ratkaisun.

4. **Integraatio** — email-service on jo Rust/tokio/reqwest. Uusi moduuli istuu luontevasti nykyiseen arkkitehtuuriin.

5. **Prompt caching** — 90% säästö input-tokeneista. Tämä vaatii `cache_control`-kenttien tarkkaa hallintaa jota frameworkit eivät välttämättä tue.

### Vaihtoehtoinen: Vaihtoehto B (rig) prototyyppivaiheessa

Jos halutaan nopeasti testata agenttista looppia ennen sitoutumista omaan toteutukseen, rig on hyvä valinta. Se voidaan myöhemmin korvata omalla toteutuksella kun tarpeet tarkentuvat.

### Toteutussuunnitelma (korkean tason)

```
services/email/src/
  ├── llm/
  │   ├── mod.rs          # AnthropicClient, tyypit
  │   ├── types.rs        # Message, ContentBlock, ToolUse, ToolResult...
  │   └── streaming.rs    # SSE-parsinta (valinnainen MVP:ssä)
  ├── agent/
  │   ├── mod.rs          # Agenttinen loop
  │   ├── tools.rs        # Tool-toteutukset
  │   └── state.rs        # Matkalaskuluonnoksen tila
  └── handler.rs          # Nykyinen routing → laajentuu kutsumaan agenttia
```

**Uudet riippuvuudet:**
```toml
reqwest = { version = "0.12", features = ["json", "stream"] }
serde_json = "1"
```

(`serde`, `tokio`, `anyhow` ovat jo projektissa.)

---

## Lähteet

- [Anthropic Messages API](https://docs.anthropic.com/en/api/messages)
- [Anthropic Tool Use](https://platform.claude.com/docs/en/agents-and-tools/tool-use/overview)
- [Anthropic Building Effective Agents](https://www.anthropic.com/research/building-effective-agents)
- [Anthropic Pricing](https://platform.claude.com/docs/en/about-claude/pricing)
- [rig (0xPlaygrounds)](https://github.com/0xPlaygrounds/rig) — 7k stars, aktiivinen
- [genai (jeremychone)](https://github.com/jeremychone/rust-genai) — 750 stars
- [goose (Block)](https://github.com/block/goose) — 43k stars, reqwest-pohjainen
- [aichat (sigoden)](https://github.com/sigoden/aichat) — 10k stars
- [async-openai](https://github.com/64bit/async-openai) — 1.9k stars
- [anthropic-rs](https://github.com/AbdelStark/anthropic-rs) — 75 stars
- [misanthropy](https://github.com/cortesi/misanthropy) — 35 stars
- [swiftide](https://github.com/bosun-ai/swiftide) — 690 stars
- [rmcp (MCP SDK)](https://github.com/modelcontextprotocol/rust-sdk) — 3.3k stars
