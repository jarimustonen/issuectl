---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#80", "#57"]
labels: [agent, observability, sweeper, schema, post-mvp]
---

# 87. agent_runs sweeper enhancements (post-MVP)

_Source: #80:n `/llm-review` + `/assess-findings` SPIN-OFF + DISCUSS -kohteet, niputettuna yhteen koska ne aktivoituvat samasta laukaisimesta — kun MVP/pilotti-vaihe siirtyy tuotantoskaalaan tai Phase 4:n asiantuntija-UI vaatii enemmän._

## Description

`#80` toi `crates/server/src/ingest/sweeper.rs`:n joka finalisoi
vuotaneet `agent_runs`-rivit 60s tikityksellä, 20 min stale-thresholdilla.
Toimii MVP/pilotti-vaiheen single-node-deploylla ja täyttää #80:n
acceptance-kriteerit. `/llm-review`-kierros (4 mallia) nosti
kuitenkin esiin viisi parannusta jotka eivät ole #80:n korrektius-asioita
mutta jotka pitää ratkaista ennen kuin järjestelmä siirtyy
tuotantoskaalaan tai Phase 4:n UI vaatii tighter-latenssia.

Tämä issue niputtaa ne yhteen koska ne **aktivoituvat samasta laukaisimesta**:
joko false-cancellation alkaa esiintyä tuotannossa, tai operaattorit
tarvitsevat dashboard-dataa, tai cost-reconciliation törmää
"unknown cost"-aukkoihin. Yksittäin mikään ei ole tarpeeksi iso
oma worktree-investointiinsa MVP-vaiheessa.

## Scope

### 1. Observability-pinta (alkup. SPIN-OFF #16)

Sweeper on canary-signaali sille että agent-loopit pudotetaan —
kestävä piikki `swept`-määrässä on voimakkain operatiivinen hälytys
ingestin epäterveellisyydestä. Tällä hetkellä signaali on JSON-loki
(`INFO swept = N` per onnistunut sweep). Operaattori ei voi
graphata sitä yli aikaikkunan eikä asettaa hälytystä ilman
log-aggregaattorin grep:iä.

Tarvittavat metriikat (kun binäärille tuodaan metric-pinta):
- Counter `agent_runs_sweeper_swept_total`
- Counter `agent_runs_sweeper_errors_total`
- Gauge `agent_runs_running_count` (nykyhetki)
- Histogram `agent_runs_sweeper_duration_seconds`
- WARN-kynnys `swept > 100/min` -tilanteelle

**Riippuvuus:** binäärillä ei ole metric-pintaa — tämä työ pitäisi
landata yhteistyössä jonkin muun mittauksen kanssa (Anthropic
API-call latency, HTTP request-latency, jne.).

### 2. Heartbeat / lease liveness signal (alkup. SPIN-OFF #21)

Nykyisen sweeperin korrektius nojaa oletukseen *"`started_at + 20
min` on aina past kaikkien live-runien"*. Pitää tällä hetkellä
koska `MAX_AGENT_WALL_CLOCK = 10 min` rikkoo cap:in — mutta se on
**toteutuksen sattuma, ei rakenteellinen invariantti.** Pathological
skenaariot (roikkuva Anthropic-call ilman timeoutia, tokio scheduler
stall, host suspend backupille, joku nostaa cap:in 12 minuuttiin)
voivat ylittää 20 min ja sweeper finalisoi live-loop:in väärin.
Audit-fideliteetin hiljainen rappeutuminen.

Toteutus:
```sql
ALTER TABLE agent_runs
    ADD COLUMN heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

CREATE INDEX idx_agent_runs_running_heartbeat
    ON agent_runs (heartbeat_at)
    WHERE status = 'running';
```

`agent_trace::record_step` (tai oma `touch_run`-funktio) päivittää
heartbeatin per LLM-iteraatio. Sweeperi vaihtaa kyselyn
`heartbeat_at < NOW() - INTERVAL '5 minutes'`-muotoon. Threshold
laskee ~21 min → muutamaan minuuttiin. Drop-guard
`TraceHandle`:lla (alkup. #80:n vaihtoehto A) voidaan harkita
tämän yhteydessä.

### 3. Drop-guard `TraceHandle`:lla (alkup. #80:n vaihtoehto A)

Pienentäisi latenssin ~21 min → muutaman sekunnin happy-path:n
runtime-droppille. Hyödyllinen jos heartbeat-vaihtoehto valitaan;
pelkkä Drop-guard ilman heartbeat:iä ei kata paniikkia
DB-pool-yhteyden aikana.

### 4. Cancelled token totals + cost-dashboard semantiikka (alkup. DISCUSS #19)

**Product-päätös, ei koodibugi.** Sweeperi jättää `total_input_tokens`/
`total_output_tokens` koskematta — ne ovat 0 (schema-default).
Mutta Anthropic laskutti tokenien käytöstä joka tapahtui ennen
droppia. Audit-rivi sanoo "0 tokens", lasku sanoo "N tokens".

Vastaus määrittää schema-suunnan, dashboard-SQL:n, ja
asiakaslaskutuksen:
- **Vaihtoehto A:** "unknown cost" — dashboards filteröivät
  `WHERE error_class != 'cancelled'`. MVP-paras.
- **Vaihtoehto B:** inkrementaalinen token-persistointi per
  iteraatio — agent-loop UPDATE:aa `total_*_tokens` jokaisen
  LLM-kutsun jälkeen. Sweepatu rivi kantaa partial truth:in.
  Lisää DB-load:ia ja kompleksisuutta.

### 5. Synteettinen `agent_steps`-error-rivi sweep-aikaan (alkup. DISCUSS #20)

**Phase 4 UI -valinta.** Kun sweeper finalisoi rivin, olemassa
olevat `agent_steps` vain päättyvät — viimeinen onnistunut
instrumentointi, sitten ei mitään. Tarkkaa, mutta forensisesti
ohutta. Phase 4:n trace-aikajana renderöi tämän "ajo hiljaa
pysähtyi"-näkymänä.

Vaihtoehdot:
- **Eksplisiittinen markeri:** sweeper kirjoittaa myös
  synteettisen `kind='error'`-rivin `seq = MAX(seq) + 1`-positioon.
  UI luettavampi, mutta synteesoi tapahtuman jota ei oikeasti
  tapahtunut.
- **Rehellinen tyhjä häntä:** UI renderöi siististi puuttuvan
  hännän. Default kunnes UI:n suunnittelu konkretisoituu.

### 6. `MAX_AGENT_WALL_CLOCK`-enforcement-malli (alkup. DISCUSS #29)

**Verifioitava ennen heartbeat-päätöstä.** Onko cap:
- (a) ulompi `tokio::time::timeout(process_with_tools, ...)`? Kova
  10 min → 20 min sweeper-threshold on reilusti puskurin kanssa,
  false-cancellation cosmic ray.
- (b) per-iteraatio-tarkistus? Roikkuva Anthropic-call ylittää
  cap:in → false-cancellation OCCASIONAL → heartbeat (kohta 2)
  nousee tarpeellisemmaksi nopeasti.

Verifikaatio on triviaali (yksi grep `process_with_tools`-modulista),
mutta vastaus määrittää investointijärjestyksen.

## Acceptance criteria

Tämä issue suljetaan kun pilotti-vaihe siirtyy tuotantoskaalaan ja
**joko**:

1. Yksittäiset kohdat ovat aktivoituneet (esim. false-cancellation
   havaittu prod-datassa → kohta 2 toteutettava heti),
2. Phase 4:n UI:n suunnittelu konkretisoituu ja vaatii kohdat 1, 4,
   5 päätettyinä,
3. Operaattorit tarvitsevat dashboard-dataa kohdan 1 mukaisesti,

…tai issue split:taan takaisin omiin issueihinsa kun yksi kohta
osoittautuu kiireelliseksi.

## Riippuvuudet

- **Estyy:** Kohdan 1 toteuttaminen vaatii että binäärillä on
  metric-pinta (oma issue tarvittaessa).
- **Estää:** ei mitään tällä hetkellä. Sweeperi täyttää #80:n
  acceptance-kriteerit MVP/pilotti-vaiheessa nykyisellään.
- Phase 4 -UI:n suunnittelu voi nostaa kohtien 1, 4, 5
  prioriteettia.

## Out of scope

- Yleinen tokio-cancellation-säädäntö agent-loopissa.
- `pg_try_advisory_lock` / `SKIP LOCKED` -batching — multi-instance
  -deployssa ratkaistava asia, ei single-node MVP.
- Sweeper-modulin siirtäminen pois `ingest::*`-hierarkiasta —
  mekaaninen churn.

## Konteksti

`/llm-review` 1 kierros (Gemini 3.1 Pro, GPT-5.5, Claude Opus 4.7,
DeepSeek v4 Pro) raportti `history/review-issue-80-sweeper.md`.
`/assess-findings`-päätökset raportti
`history/assess-findings-issue-80.md`. 33 löydöstä → 11 FIX
(toteutettu #80:ssä), 2 SPIN-OFF + 3 DISCUSS (kerätty tähän
issueeseen), 17 DROP.

Päätös niputtaa SPIN-OFFit + DISCUSS-kohdat yhteen tehtiin
2026-05-01: nykyinen #80:n sweeperi täyttää MVP/pilotti-vaiheen
vaatimukset, ja kaikki viisi yllä olevaa kohtaa aktivoituvat samasta
laukaisimesta (skaalan kasvu tai Phase 4 -UI). Yksittäiset kohdat
voidaan split:tää takaisin omiin issueihinsa jos yksi osoittautuu
muita kiireellisemmäksi.
