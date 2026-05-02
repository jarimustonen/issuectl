---
created: 2026-05-01
updated: 2026-05-01
closed: 2026-05-01
type: bug
reporter: jari
assignee: jari
status: done
priority: high
epic: 57
related: ["#56", "#57", "#58", "#59", "#61", "#62", "#82"]
labels: [agent, observability, error-handling, cancellation]
commits:
  - hash: ad2b7a7
    summary: "feat(server): agent_runs cancellation sweeper (#80)"
  - hash: 2226ab5
    summary: "docs(issues): close #80 done; file #87 sweeper-enhancements; log on #56/#57"
---

# 80. agent_runs cancellation safety — vuotavat `running`-rivit

_Source: #61:n LLM-review SPIN-OFF, `history/review-agent-trace-errors-aborts.md`._

## Description

`process_with_tools` (`crates/server/src/ingest/agent/mod.rs`) avaa
`agent_runs`-rivin `trace::start`-kutsulla loopin alussa ja finalisoi
sen joko `trace::finalize` (Ok-haara) tai `trace::finalize_failure`
(Err-haara) -kutsulla. Jos tulevaisuus pudotetaan ennen kuin
finalize-haara suoritetaan, **kumpikaan finalize ei aja** ja rivi jää
`running`-tilaan ikuisesti.

Tämä tapahtuu normaaliolosuhteissa:

- **Tuotantopuolen graceful SIGTERM (deploy):** tokio-runtime sammuu
  shutdown-aikakatkaisulla (oletuksena 30 s). Jos agent-loop on
  käynnissä deploy-hetkellä, sen tulevaisuus pudotetaan kesken — ja
  `MAX_AGENT_WALL_CLOCK = 10 minuuttia` tarkoittaa että aktiivisella
  postilaatikolla on **jokaisella deployssa** ajossa olevia runeja.
- **Paniikki spawnatussa taskissa:** jos `tools::execute` paniikoi
  (esim. unwrap JSON-arvolla, integer overflow), task purkautuu ja
  tulevaisuus pudotetaan ilman finalisointia.
- **IMAP-yhteyden katkos:** `runner.rs` voi peruuttaa agent-taskin
  kun IMAP IDLE -kontrolli huomaa yhteyden katkenneen.
- **Lokaali kehitys:** jokainen `cargo run`-restart pudottaa kaikki
  ajossa olevat agent-loopit.

Audit-pinnan kannalta tämä on aktiivinen vaurio:

- Asiantuntijan UI:n (Phase 4) "live runs"-näkymä ei voi erottaa
  vuotaneita rivejä aidosti ajossa olevista.
- Audit-kyselyt jotka suodattavat `WHERE status != 'running'` aivan
  oikeutetusti pitäen sitä "kaikki valmistuneet runit"
  -merkityksessä jättävät vuodot huomaamatta — taulu degradoituu
  hiljaisesti.
- AGENTS.md:n nykyinen kuvaus listaa tämän tunnettuna rajoitteena
  mutta ratkaisu puuttuu.

## Scope

Toteutusvaihtoehdot:

### Vaihtoehto A — `Drop`-guard `TraceHandle`:lla

`TraceHandle` saa `Drop`-impl:in joka:

1. Tarkistaa onko run jo finalisoitu (uusi `finalized: Cell<bool>`
   tai vastaava lippu).
2. Jos ei, käyttää `tokio::runtime::Handle::try_current()`:tä
   spawnamaan detached `finalize_failure`-taskin uudella
   `RunStatus::AbortedCancelled`-statuksella.

Vahvuudet: rakenteellinen, ei vaadi schema-muutoksia, fix tapahtuu
heti pudotuksen yhteydessä.

Heikkoudet: spawn-from-Drop -kuvio on hankala
(`Handle::try_current()` voi epäonnistua jos runtime on jo
sammutettu); detached task voi itse pudota ennen finalisointia
runtime-shutdown-aikakatkaisun aikana; `RunStatus`-enum tarvitsee
uuden variantin (writer-puolella schema CHECK
+ migraatio).

### Vaihtoehto B — Sweeper-job

Periodinen tausta-tehtävä (esim. 1 kerta minuutissa) joka:

1. Hakee `agent_runs WHERE status='running' AND started_at < NOW()
   - INTERVAL '15 min'`.
2. Finalisoi nämä `RunStatus::AbortedCancelled`-statuksella + jokin
   `error_class='cancelled'`.

Tämä vaatii:

- Uusi indeksi: D2:n `idx_agent_runs_status_started` on **partial
  index** joka sulkee `running`-rivit pois ("`WHERE status != 'running'`").
  Sweeper tarvitsee oman indeksin, esim.
  `CREATE INDEX idx_agent_runs_running_started ON agent_runs (started_at) WHERE status = 'running'`.
- Uusi `RunStatus`-variantti (`AbortedCancelled`) ja CHECK-update
  migraatiossa.
- Sweeper-task `runner.rs` tai erillinen `crates/server/src/ingest/`
  -moduuli, käynnistettynä `main.rs`:stä `tokio::spawn`:lla.

Vahvuudet: periodinen, idempotentti, ei vaadi spawn-from-Drop -kuviota.
Toimii myös tilanteissa joissa runtime sammuu kokonaan ennen kuin
Drop ajetaan (sweeper havaitsee jälkeenpäin, kun seuraava prosessi
käynnistyy ja sweeper ajaa).

Heikkoudet: 15 min latenssi (audit "live runs"-näkymä näyttää
vanhentuneita rivejä siihen asti); vaatii migraation + uuden
taustatehtävän.

### Vaihtoehto C — Molemmat

Drop-guard yrittää best-effort-finalisointia välittömästi, sweeper
toimii safety-net:nä jos Drop epäonnistui (runtime-shutdown,
panic-during-Drop, jne.). Maksimi-luotettavuus, eniten koodia.

## Decision needed

Käyttäjän päätös vaihtoehtojen välillä. Suositus implementoinnin
jälkeen: **vaihtoehto B (sweeper)** ensin foundation-tasona —
ratkaisee 100 % vuotavista riveistä 15 min latenssilla. Drop-guard
voidaan lisätä myöhemmin jos latenssi osoittautuu ongelmaksi
asiantuntijan UI:ssa.

## Resolution (2026-05-01)

Toteutettu **vaihtoehto B (sweeper)**. Threshold nostettu reviewerien
yksimielisestä huolesta 15 min → 20 min (= 2 × `MAX_AGENT_WALL_CLOCK`)
jotta hidas finalize-polku (DB pool acquisition, NTP step, scheduler
stall) ehtii landata ennen kuin sweeper claimaa rivin. Latenssi
enintään ~21 min (sweep + 20 min). Drop-guard jää follow-up:iksi.

**Tuotos:**

- `crates/ops/migrations/022_agent_runs_cancellation.sql` —
  `aborted_cancelled` lisätty `agent_runs.status` CHECK:iin +
  partial-indeksi `idx_agent_runs_running_started ON agent_runs
  (started_at) WHERE status = 'running'`. (D2:n
  `idx_agent_runs_status_started` on partial `WHERE status !=
  'running'`, joten ei kata sweeperin kohderiviä.)
- `crates/ops/src/agent_trace.rs::RunStatus::AbortedCancelled` —
  variantti olemassa täydellisyyden vuoksi; writer ei kosketa, sweeper
  kirjoittaa raw SQL:llä.
- `crates/server/src/ingest/sweeper.rs` — `run_agent_runs_sweeper`
  (60s tikitys, `MissedTickBehavior::Delay`) + `sweep_stale_runs`
  (UPDATE rows WHERE status='running' AND started_at < NOW() - 20min,
  `finished_at = GREATEST(NOW(), started_at)` defensiivisenä,
  token-kenttiä ei kosketa). Sentinel `error_message` ei sisällä
  threshold-arvoa (audit-kuluttajien on filteroitava `error_class =
  'cancelled'`:llä, ei tekstillä).
- `crates/server/src/main.rs` — sweeper folded `tokio::select!`-
  supervisoriin axumin ja ingestin rinnalle. Paniikki tai odottamaton
  exit fataali (consistent muiden rinnakkaisten futurein kanssa) →
  systemd `Restart=always` → ei hiljaista degradaatiota.
- `crates/server/tests/agent_runs_sweeper.rs` — 4 integraatiotestiä:
  stale-row swept, fresh-row untouched, completed-row untouched,
  *finalize-after-sweep race* (audit-immutability-invariantti:
  `finalize_run` swepatulle riville → `OpError::Conflict`).
- AGENTS.md (`crates/ops/AGENTS.md` + `crates/server/AGENTS.md`)
  päivitetty: migraatio 022, sweeper-strategia, status-mappaustaulu
  laajennettu `aborted_cancelled`-rivillä. **#61:n caveat-osio
  poistettu** ja korvattu kuvauksella korjatusta tilanteesta.

**Acceptance criteria — kaikki täytetty:**

- ✅ Migraatio 022 lisätty (uusi `RunStatus`-variantti CHECK-arvossa +
  partial-indeksi).
- ✅ `RunStatus::AbortedCancelled`-variantti `crates/ops/src/agent_trace.rs`:ssä.
- ✅ Sweeper-task `crates/server/src/ingest/sweeper.rs`-moduulissa.
- ✅ Sweeper supervisoidaan `main.rs`:n `tokio::select!`:ssä (paniikki
  fataali, ei detached `tokio::spawn`).
- ✅ Integraatiotestit: stale sweepataan, fresh ei, completed ei,
  finalize-after-sweep palauttaa `Conflict`.
- ✅ `cargo test --workspace` puhtaana (537 testiä).
- ✅ AGENTS.md-päivitykset (ops + server).
- ✅ Kysely `SELECT count(*) FROM agent_runs WHERE status='running'
  AND started_at < NOW() - INTERVAL '1 hour'` palauttaa 0:n
  acceptance-tilanteessa (sweeper finalisoi rivit < 21 min sisällä).
- ✅ #61:n landauksessa lisätty caveat-osio AGENTS.md:ssä korvattu
  kuvauksella korjatusta tilanteesta.

**Review:** `/llm-review` 1 kierros (Gemini 3.1 Pro, GPT-5.5, Claude
Opus 4.7, DeepSeek v4 Pro) + `/assess-findings` (33 löydöstä → 11
FIX, 2 SPIN-OFF, 3 DISCUSS, 17 DROP). Kaikki FIX-tason löydökset
käsitelty (sis. paluusyklissä löydetty `crates/server/src/ingest/agent/trace.rs`
warn-login inakkurra "row will remain in 'running' state" — väärin
post-sweep koska rivi on `aborted_cancelled` eikä `running`).
Raportit: `history/review-issue-80-sweeper.md` +
`history/assess-findings-issue-80.md`.

**Migraationumero:** valittu 022. #82-worktreessä rinnakkainen
migraatio 021 — jos #82 landaa ensin, tämä pysyy 022:ssa; jos tämä
landaa ensin, mainline-fix-up siirtää tämän 021:ksi ja #82:n 022:ksi.

**Follow-up:** SPIN-OFF + DISCUSS -kohdat (`/assess-findings`-päätös
2026-05-01) niputettu yhteen issueksi **#87 sweeper enhancements
(post-MVP)**, koska kaikki aktivoituvat samasta laukaisimesta —
skaalan kasvu single-node Hetzneristä eteenpäin tai Phase 4 -UI:n
vaatimukset. Yksittäin kukaan ei ole MVP/pilotti-vaiheessa kiireellinen.
Niputetut kohdat: observability-pinta (Prometheus-metriikat), heartbeat
liveness signal, Drop-guard, cancelled token totals -semantiikka,
synteettinen `agent_steps`-error-rivi, `MAX_AGENT_WALL_CLOCK`-
enforcement-mallin verifikaatio. Voidaan split:tää takaisin omiin
issueihinsa jos yksi nousee muita kiireellisemmäksi.

**Out of scope (ei #87:ssa):**

- `pg_try_advisory_lock` / `SKIP LOCKED` -batching — multi-instance
  -deployssa ratkaistava, ei single-node MVP.
- Sweeperi-moduulin siirtäminen pois `ingest::*`-hierarkiasta —
  mekaaninen churn.

## Acceptance criteria

- `process_with_tools`-tulevaisuuden droppaus (testattu joko Drop-
  guard:in unit-testillä tai sweeper:in integraatiotestillä) johtaa
  riviin joka ei ole `running`-tilassa kohtuullisen ajan kuluessa.
- Uusi `RunStatus::AbortedCancelled`-variantti + CHECK-migraatio
  + writer-validaatio + AGENTS.md status-mappaustaulun päivitys.
- Sweeper-vaihtoehdossa: uusi indeksi
  `idx_agent_runs_running_started`, periodinen task `main.rs`:ssä,
  metrics/loki sweeper-aktiivisuudesta.
- Kysely `SELECT count(*) FROM agent_runs WHERE status='running' AND
  started_at < NOW() - INTERVAL '1 hour'` palauttaa 0:n
  käyttöönoton jälkeen.

## Riippuvuudet

- **Estyy:** ei (voi aloittaa milloin vain D-trackin sisällä).
- **Estää:** **Phase 4 (#57:n asiantuntija-UI)** — UI ei voi näyttää
  "live runs"-näkymää vuotavalle taululle. **Kannattaa landata
  ennen Phase 4:n suunnittelua.**
- Voi rinnakkain #60 / #62 kanssa eri tiedostoissa.

## Out of scope

- Yleinen tokio-cancellation-säädäntö (jos paniikit ovat
  toistuvia, ne ovat oma bugi).
- `email_processing.status` -taulun puhdistus vastaavanlaisten
  vuotojen varalta.
- Agent-loopin retry-policy:n revisiointi.

## Konteksti

`/llm-review`-kierroksen yhteenveto: kolme reviewerie (Gemini,
Anthropic, DeepSeek) yksimielinen Critical-luokituksessa. DeepSeek
aluksi MVP-acceptable mutta upgrade:si High:ksi pushback:in jälkeen.
Anthropic: "tämä ei ole edge case, tämä on jokainen
tuotanto-deploy". Raportti: `history/review-agent-trace-errors-aborts.md`.

D2-writer:in (`crates/ops/src/agent_trace.rs`) moduulidoc jo lippaa
sweeper:in schema-follow-up:ina:

> "Stale-`running` reaper for crash recovery (the partial index
> `idx_agent_runs_status_started` excludes `running`, so a sweeper
> needs its own index — schema follow-up)."

#61:n landauksessa AGENTS.md:hen lisätty caveat-osio joka
dokumentoi rajoitteen julkisesti, ja `process_with_tools`:n
docstringissä mainitaan että tulevaisuuden pudotus jättää
`running`-rivin.
