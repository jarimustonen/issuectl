---
created: 2026-04-29
updated: 2026-04-29
type: design-analysis
owner: jari
status: draft
related: ["#56", "#57", "#38", "#26", "#46", "#49"]
---

# D1 — Agent-trace tietomallin analyysi

Tämä dokumentti perustelee `schema-draft.md`:n päätökset. Toteutus
odottaa #56 Phase 1:tä (yhteistä DB:tä) — tämä dokumentti antaa
suunnitellun lähtöpisteen #57 Phase 2:lle.

---

## 1. Tavoite

Tehdä **agenttisen loopin yhden viestin käsittely jälkikäteen
kysyttäväksi DB:stä**, ei vain lokeista. Kaksi käyttäjäryhmää:

1. **Käyttäjä:** "mitä agentti teki minun viestilleni" — selkokielinen
   yhteenveto: vastaanotetut liitteet, tunnistetut kuitit, hylätyt,
   lähetetyt vastaukset, virheet.
2. **Asiantuntija (me):** "mikä meni pieleen / mikä vaatii puuttumista"
   — kaikki epäselvät tapaukset, mahdollisuus peruuttaa, korjata,
   palauttaa, käynnistää uudelleen.

Tämä on MVP-vaiheen kriittinen työkalu: ilman tracea kehitämme
sokkona (CLAUDE.md: oikeellisuus = ainoa tavoite).

---

## 2. Mitä agenttinen looppi tällä hetkellä tekee

Lähde: `services/email/src/main.rs`, `agent/mod.rs`, `extraction.rs`,
`tools/dispatch.rs`, `handler.rs` ja olemassa olevat lokirivit
(`event_type = "..."`).

### 2.1 Tapahtumien sijainti tällä hetkellä

| Tieto                                          | Missä nyt                                    |
|------------------------------------------------|----------------------------------------------|
| Viestin vastaanotto + identifiointi            | `email_processing` (status, spam_verdict)    |
| Liitteen raaka data                            | `attachments` (BYTEA, sha256)                |
| Liitteen OCR-tulos (per liite)                 | `extractions.extracted_data` (JSONB)         |
| Permanent skip (size, MIME, 4xx)               | `extractions` stub-rivi `content_type='extraction_skipped'` + `extracted_data.skip_reason` |
| Policy-skip (liikaa liitteitä / liikaa tavuja) | **Vain lokissa** (`extraction.skipped`)      |
| Policy-reply ("liitteitä liikaa")              | **Vain templates + SMTP-loki**               |
| LLM-iteraatio: input/output, stop_reason       | **Vain lokissa** (`Agent iteration`)         |
| Tool_use input/output                          | **Vain lokissa** (`Tool result`, tool_results-blokit `conversations.content_json`:ssä) |
| Agentin tekstivastaus käyttäjälle              | `thread_messages` + lähetetyn replyn `Message-Id` |
| Tallennettu kuitti                             | `receipts` (idempotenssi `idempotency_key`)   |
| Kuitin nykyversion tausta (mistä OCR-passista) | `receipts.extraction_id` (vain pointer)       |
| Aiemmat kuittiversiot                          | **Ei missään** (overwrite — ks. #38)         |
| Wall-clock-/iteration-budgetin ylitys          | **Vain lokissa** (`AgentError::Transient`)   |
| MaxTokens-truncation                           | **Vain lokissa + scrub** (`#49`)              |
| Retry: minkä koodi syyn takia retryyn meni     | `email_processing.error` (yksi merkkijono)    |

Yhteenveto: **kaikki taitepisteet ovat lokissa**. DB tietää että
viesti tuli + mitä lopulta tallentui, mutta ei sitä polkua mitä
agentti kävi siellä välissä — eli nimenomaan asiantuntijan tarvitseman
kontekstin.

### 2.2 Avainklassifikaatiot joita pitää voida kysyä

Konkreettiset asiantuntija-kysymykset jotka design-mallin pitää tukea:

- **"Miksi käyttäjälle ei vastattu?"** — onko spam, onko reply-loop-suoja
  (Auto-Submitted), onko policy-skip (liikaa liitteitä), onko Anthropic
  4xx joka paljasti permanent-skipin, onko MaxTokens-truncation, onko
  agentin wall-clock-budget paukkunut?
- **"Miksi tämä kuitti tallentui väärin?"** — mikä tool_use, millä
  inputilla, mikä extraktio sen pohjana — tarvitsee linkityksen
  `agent_steps → extractions → receipts`.
- **"Miksi tämä liite jätettiin huomiotta?"** — permanent skip (mistä
  syystä), policy skip (size/count cap), transient failure (kuinka monta
  kierrosta yritetty)?
- **"Mitä agentti teki muuten kuin tallensi kuitteja?"** — esim.
  `update_user_*`-kutsut, `read_skill`-luvut, `restore_receipt_revision`
  (tuleva, #38), web-haut.

Kaikki yllä olevat saadaan jos talletamme **per LLM-kierros yksi rivi
+ per tool_use yksi rivi**, ja kytkemme rivit nykyisiin tauluihin (FK).

---

## 3. Mitä trace-mallin pitää kattaa

Karkeasti: yksi `agent_run` per viestin käsittely, alla N steppiä
jotka kuvaavat mitä iteraatioita agent ajoi ja mitä tooleja se
kutsui.

### 3.1 Granulariteetti

**Yksi rivi per LLM-iteraatio** (= yksi `messages.create`-kutsu) sekä
**yksi rivi per tool_use** (= yksi tool-suoritus). Tämä on samalla
karkeudella kuin nykyinen `tracing::info!("Agent iteration", ...)` ja
`tracing::info!("Executing tool", ...)`.

Tarkempaa per-tool-kohtaa (esim. SQL-kysely sisällä) ei tarvita —
tool-tuotos riittää. Karkeampi (vain "yksi rivi per koko run")
hukkaa juuri sen mitä asiantuntija haluaa nähdä.

### 3.2 Mitä rivien pitää sisältää

`agent_runs` (kompakti pää):

- run_id (UUID), tenant_id, user_id, message_id, thread_id
- status (`running`, `completed`, `failed_transient`, `failed_permanent`,
  `aborted_max_iterations`, `aborted_wall_clock`, `truncated_max_tokens`)
- model
- iterations, total_input_tokens, total_output_tokens
- started_at, finished_at, duration_ms
- error_class + error_message (NULL onnistuneilla)
- trace_id (sama kuin `tracing`-spaniin lisätty `Uuid::new_v4()`,
  jotta loki + DB sidotaan toisiinsa)

`agent_steps` (yksi rivi per LLM-iteraatio TAI tool_use):

- run_id, seq (juokseva 1..N saman runin sisällä)
- kind: `llm_call` | `tool_use` | `decision` | `error`
- iteration (LLM-kierroksen numero — sama kuin nykyinen
  `iteration`-kenttä lokissa)
- For `llm_call`: stop_reason, input_tokens, output_tokens,
  request_id (Anthropic-puolen pyynnön id, jos saatavilla)
- For `tool_use`: tool_name, tool_use_id (sama kuin Anthropic-API:n
  `tu_*`), input_json, output_json, ok, is_error
- duration_ms
- error (yksittäisen stepin virhe — runin error on `agent_runs.error_*`)
- created_at

### 3.3 Päätökset eksplisiittisinä riveinä

Tärkeät "agentti teki päätöksen" -kohdat ovat saatavilla
`agent_steps.kind = 'tool_use'`-rivien avulla (tool_name = `save_receipt`
→ päätös oli "tallenna tämä kuitti"). Mutta:

- Pelkkä `tool_name` ei riitä päätösten **listaamiseen UI:ssa**:
  asiantuntija haluaa nähdä "mitä päätöksiä syntyi" ilman että hän
  joutuu poimimaan ne stepeistä.
- Osa päätöksistä **ei tule agentilta** (esim. `policy_reply`,
  `permanent_skip`, `unknown_sender`, `spam_skip` syntyvät
  `process_message_inner`:ssa).

Päätös: **käytetään `agent_steps.kind = 'decision'`-riviä** sen
sijaan että tehtäisiin erillinen `agent_decisions`-taulu. Ks. §5.1
perustelu.

---

## 4. Mitä on jo nyt — älä kahdenna

| Olemassa            | Trace ei toista                                  |
|---------------------|--------------------------------------------------|
| `email_processing`  | spam-verdict + lopullinen status — trace osoittaa siihen FK:lla |
| `attachments`       | raaka data + sha256 — trace viittaa `attachment_id`:llä |
| `extractions`       | OCR-tulos + permanent skip stub — trace viittaa `extraction_id`:llä |
| `receipts`          | tallennetut kuitit — trace viittaa `receipt_id`:llä |
| `thread_messages`   | viestihistoria (myös tool_use-blokit `content_json`:ssä) — trace **toistaa hieman** mutta jäsennellympänä |
| `conversations.content_json` | LLM-rooli-/tool_use-blokit raakana — trace tallentaa erikseen jäsenneltyä metadataa (stop_reason, tokens, durations) |

**Päätös:** trace-rivit eivät kopio raakaa pohjadataa. `agent_steps`
viittaa `attachments.id`, `extractions.id`, `receipts.id` -tauluihin
FK:llä. `tool_use`-stepien `input_json`/`output_json` voi sisältää
agentin näkemän JSON:in mutta isot dataset (raw_text, vision-OCR
extract) ovat jo `extractions`-taulussa eikä niitä toisteta.

---

## 5. Vaihtoehdot ja päätökset

### 5.1 Kaksi taulua (`agent_runs` + `agent_steps`) vai kolme (lisäksi `agent_decisions`)?

**Vaihtoehdot:**

- **A — kaksi taulua, päätökset stepeissä:** `agent_steps.kind = 'decision'`
  -rivi joka kantaa `decision_type`-koodia + JSON-payloadin.
- **B — kolme taulua, oma `agent_decisions`:** asiantuntija-UI
  voi tehdä `WHERE decision_type IN (...)` ilman steppien selaamista.
- **C — pelkkä `agent_runs` + JSONB-array stepeistä:** koko trace
  yhdessä JSONB-blobissa.

**Päätös: A.** Perustelut:

- Päätösten määrä per run on pieni (1–10), joten `WHERE
  kind = 'decision'`-haku `agent_steps`:istä on yhtä nopea kuin
  oma taulu, eikä lisää schema-pintaa.
- Päätökset säilyttävät **järjestyksen** (esim. ensin `policy_reply`,
  sitten `message_finalized`) joka on tärkeä asiantuntijan ymmärtää —
  oma taulu kadottaisi sen järjestyksen tai joutuisi toistamaan
  `seq`-kentän.
- Liittyvät evidenssin perimissuhteet (decision viittaa stepin
  `attachment_id`:hen tai `extraction_id`:hen) ovat luonnollisia
  saman taulun riveissä.

C hylätään koska JSONB-blobin sisäkenttiin ei voi laittaa indeksejä
joustavasti, eikä saa FK-rajoitteita.

### 5.2 JSON-blob vs. normalisoidut kentät tool input/output:ille

**Vaihtoehdot:**

- **A — JSONB sellaisenaan** (`input_json`, `output_json`).
- **B — normalisoidaan tunnetut kentät** (esim. `tool_input_vendor`,
  `tool_input_amount`, ...).

**Päätös: A.** Perustelut:

- Tool-pinta laajenee aktiivisesti (#33 skill-based, #34 compound),
  per-kenttä-normalisointi rikkoutuu joka muutoksessa.
- Kaikki olemassa oleva tool-data on jo JSON (Anthropic-API:n
  raakamuoto), ei muunnostarvetta.
- PostgreSQL:n `jsonb_path_ops`-indeksi riittää harvinaisiin
  kohdistettuihin hakuihin (esim. "kaikki `save_receipt`-kutsut joiden
  vendor = X") — lisätään vasta jos tarve syntyy.

### 5.3 Retention / TTL

**Asiantuntijan tarpeet:** ainakin 90 päivää aktiivista, jotta
demoja ja edellisten viikkojen säätöjä voi tarkastella.

**Levytila:** karkea arvio per run = 1 LLM-iteraatio (~2 KB JSON) +
3–5 tool_use (~1–4 KB / kpl JSON) + 1 decision = **~10–25 KB / run
JSON-rivinä**. 100 viestiä/päivä → **~1 MB/päivä → 30 MB/kk → 360 MB/v**.
Hyvin maltillinen. Liitteiden raaka data dominoi joka tapauksessa
(`attachments.data BYTEA`).

**Päätös (vahvistettu 2026-04-29):** **pysyvä säilytys, ei TTL:ää.**
Taloussovelluksen traceability on tärkeämpi kuin levytilan
optimointi. Verolainsäädäntö (#36) saattaa muutenkin pakottaa
pysyvyyden, ja levytila ei ole pullonkaula MVP:ssä.

Jos myöhemmin (esim. `parse_message`-tyylinen tool joka tallentaa koko
sähköpostin tekstin tool_input:iin) tilankäyttö muuttuu merkittävästi,
TTL voidaan lisätä silloin — mutta MVP-pohjana lähdetään pysyvästä.

### 5.4 PII

**Mitä trace tallentaa joka voi olla henkilötietoa?**

- `agent_runs.message_id` — sähköpostin Message-Id (header-arvo, ei
  henkilötieto sinänsä, mutta yhdistää käyttäjään).
- `agent_steps.input_json` (LLM-iteraatio) — käyttäjän viestin teksti
  on `messages.create`:n inputissa. Iso PII-pinta jos se tallennetaan
  jokaiselle iteraatiolle.
- `agent_steps.input_json` (tool_use) — esim. `update_user_address` voi
  sisältää käyttäjän kotiosoitteen.
- `agent_steps.output_json` (tool_use) — tool-vastaus voi sisältää
  käyttäjän aiempia kuitteja, profiilitietoja jne.

**Päätös (vahvistettu 2026-04-29):**

- LLM-iteraation `input_json` **ei tallenneta sellaisenaan**. Riittää:
  iteraation numero, stop_reason, tokens, kesto. Itse viestin teksti
  on jo `thread_messages.body_plain`-puolella, ei tarvitse toistaa.
- LLM-iteraation `output_json` (vastauksen content-blokit ennen
  tool-suoritusta) — sama: ei tallenneta, koska tool_use-stepit
  kantavat samat tiedot rakenteellisesti.
- Tool_use-stepien `input_json`/`output_json` **tallennetaan
  sellaisenaan** koska se on koko trace-mallin pointti.
  Capatuu samalla `cap_tool_result_json`-funktiolla (~64 KB) joka jo
  on käytössä `agent/mod.rs`:ssä — trace ei tallenna enempää kuin
  agentti todella näki.
- **Henkilötietokenttiä ei hashata MVP:ssä.** Taloussovelluksen
  traceability vaatii että agentin näkemä data pysyy luettavana —
  anonymisointi tappaisi käyttötarkoituksen. Pääsynhallinta tehdään
  `OpContext`:n kautta (#26 §5.1): vain saman tenantin käyttäjä +
  admin näkevät runin.
- GDPR-poistopyyntö: kun käyttäjä poistetaan, `agent_runs` poistuu
  cascade:lla (FK `user_id REFERENCES users(id) ON DELETE CASCADE` —
  tämä on Phase 1:n politiikka).

Jos myöhemmin halutaan "sanitized"-näkymä asiantuntijoille jotka
eivät ole asianomaisen tenantin admineja (esim. ulkoinen review),
tarvitaan erillinen sanitization-funktio. Ei MVP:ssä.

### 5.5 Suhde `audit_events`-tauluun (#26 §4.2)

`audit_events` on **manuaalisten** muutosten audit-trail: kuka kutsui
käyttäjän, kuka muutti roolin, kuka peruutti kuitin asiantuntijan UI:sta.
Se on **pitkän elämän rikosoikeudellinen jälki**.

`agent_runs`/`agent_steps` on **automaattisten** agentti-suoritusten
trace: mitä LLM teki, mitä tooleja kutsuttiin, miksi se päätyi
johonkin lopputulokseen.

**Päätös: erilliset taulut, kaksi eri tarkoitusta.**

- `audit_events` rekisteröi *manuaalisen* `revert_save_receipt` (#57
  Phase 4) — siinä on `actor_user_id`, ei ole `agent_run_id`.
- `agent_runs`/`agent_steps` rekisteröi *agentin* `save_receipt` —
  siinä on `agent_run_id`, ei `actor_user_id` (LLM ei ole user).

**Yhteys:** kun asiantuntija peruuttaa agentin tekemän muutoksen
(`audit_events`-rivi), siinä viitataan `metadata.agent_run_id` -kentällä
siihen runiin jota peruutettiin. Yksisuuntainen pointer riittää — ei
tarvita erillistä join-taulua.

### 5.6 Suhde #38:aan (receipt-revision-history)

#38 on kuittien versionhistoria — kun sama kuitti päivittyy uudelleen
(retry, korjaus), aiemmat versiot säilyvät `receipt_revisions`-taulussa.

**Pelisääntö:**

- `receipt_revisions` säilöö **kuitin tilan eri ajanhetkinä**.
- `agent_runs`/`agent_steps` säilöö **mikä agentti-suoritus minkin
  revision teki**.

**Yhteys:** `receipt_revisions`-rivissä on `created_by_run_id BIGINT
REFERENCES agent_runs(id)` (tai `NULL` jos manuaalinen muutos webistä
— silloin `created_by_user_id` on asetettu). Tämä on Phase 2:ssa
toteutettava sidos — mainittu `schema-draft.md`:ssä.

#38 on omassa issuessa, ei tämän worktreen scope. Mutta päätös #57:n
tietomallista vaikuttaa siihen, joten avoin sidos pitää nimetä Phase
2:n alaissueessa.

---

## 6. Kytkentä #56 Phase 1:een (cross-cuts)

A1-worktree (#56 Phase 1) tekee schema-yhdistämisen päätökset (yksi
DB vai kaksi → yksi shared schema). Tämän designin näkökulmasta:

1. **Trace-taulut kuuluvat yhteiseen DB:hen.** Email-puoli kirjoittaa
   (jokainen agent-suoritus), web-puoli lukee (käyttäjän tapahtumaloki
   Phase 3, asiantuntijan UI Phase 4). Jos #56 päätyy "kahteen DB:hen
   joiden välillä replikoidaan", trace-taulut ovat ne joiden lukeminen
   web-puolelta on välitön vaatimus.
2. **`tenant_id` + `user_id` ovat aina FK:t #56:n yhdistettyihin
   `tenants`/`users`-tauluihin.** Email-puolen nykyinen
   `(tenant_id, user_id)` on jo yhteensopiva — Phase 1:n migraatio
   ei pakota muutoksia tähän designiin.
3. **`OpContext`-kenttä `agent_runs`-rivissä:** ei tallenneta tähän
   tauluun erikseen, koska `actor` on aina LLM (ei user). Sen sijaan
   web-puolen luvun yhteydessä `OpContext.tenant_id` on `WHERE`-
   ehto — sama politiikka kuin muilla user-data-tauluilla.

**Huomio A1:lle (lisätään #56 decision logiin):** trace-taulut
edellyttävät että FK-target-taulut (`users`, `tenants`,
`thread_messages`, `attachments`, `extractions`, `receipts`) ovat
samassa skeemassa kuin `agent_runs`. A1:n päätös "yksi binääri vai
kaksi" / "yksi DB vai kaksi" pitää huomioida tämä — kahden DB:n
malli pakottaisi joko (a) trace-taulut email-DB:hen (web ei näe →
Phase 3/4 estyy) tai (b) trace-taulut web-DB:hen (email-puoli
joutuu kirjoittamaan toiseen DB:hen → komplisoituu). **Yhden DB:n
malli on selvästi parempi tämän designin näkökulmasta.**

---

## 7. Päätökset (vahvistettu 2026-04-29)

Avoimina kysymyksinä esitetyt kohdat hyväksyttiin käyttäjän kanssa
2026-04-29.

1. **Retentio:** ✅ **Pysyvä säilytys, ei TTL:ää.** Taloussovelluksen
   traceability on tärkeämpi kuin levytilan optimointi.

2. **PII LLM-iteraation inputissa:** ✅ **Ei tallenneta
   `agent_steps.input_json`:ia LLM-iteraatioille.** Vain stop_reason +
   tokens + kesto + iteraationumero. Viestin teksti löytyy
   `thread_messages.body_plain`:ista jos tarvitsee yhdistää.

3. **Tool-output cap:** ✅ **Sama `cap_tool_result_json` (~64 KB) myös
   trace-tallennuksessa.** Trace ei tallenna enempää kuin agentti
   todella näki — yhteensopivuus + asiantuntija näkee saman datan.

4. **Anthropic `api_request_id`:** ✅ **Tallennetaan sarakkeena.**
   Hyödyllinen Anthropic-supportin kanssa, NULL-sallittu.

5. **PII-kenttien hashaus tool_input:eissa:** ✅ **Ei hashata.**
   Taloussovelluksen traceability vaatii että agentin näkemä data
   pysyy luettavana. Pääsynhallinta `OpContext`:n kautta.

6. **JSONB GIN-indeksit:** ✅ **Ei MVP:ssä.** Lisätään vasta jos
   konkreettinen kysely on toistuva (CLAUDE.md: ei optimointeja
   MVP-vaiheessa).

7. **Phase 2 -alaissueet:** ✅ **Luodaan tämän designin yhteydessä.**
   #58–#62 (ks. `phase-2-readiness.md`).

8. **Suositus #56:n A1:lle (yhden DB:n malli):** ✅ **Pidetään
   näkyvillä #56:n decision logissa.** Ei sido A1:tä, dokumentoitu
   input.

---

## 8. Yhteenveto

- Yksi `agent_runs` per käsittely + yksi `agent_steps` per
  LLM-iteraatio / tool_use / päätös.
- Ei erillistä `agent_decisions`-taulua — `kind = 'decision'`
  steppinä riittää.
- Ei kahdenneta nykyisiä tauluja, vaan FK:t `attachments`,
  `extractions`, `receipts`, `thread_messages`, `email_processing`.
- Retentio: pysyvä MVP:ssä.
- PII: LLM-iteraation inputtia ei tallenneta; tool_use-rivit
  tallennetaan ja näkyvät vain saman tenantin admineille.
- Erillinen `audit_events`:ista — eri tarkoitus, yksisuuntainen
  pointer toiseen suuntaan kun manuaalisia revertoja tehdään.
- Yhteensopiva #56 Phase 1:n yhdistetyn skeeman kanssa; suosittaa
  yhden DB:n mallia.
