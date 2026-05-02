---
created: 2026-04-29
updated: 2026-05-01
closed: 2026-05-01
type: task
reporter: jari
assignee: jari
status: done
priority: high
epic: 57
related: ["#56", "#57", "#58", "#59", "#49", "#80"]
labels: [agent, observability, error-handling]
---

# 61. agent_trace error-/abort-rivit ja status-mappaus

_Source: #57 Phase 2, alaissue 4/5_

## Description

Mappaa `AgentError`-tyypit ja loopin abort-tilanteet `agent_runs.status`-
arvoiksi, ja lisää `agent_steps.kind='error'`-rivi kun stepin sisällä
syntyy palautettava virhe (esim. transient tool-virhe joka silti jatkaa
loopia).

Tämä on välttämätön asiantuntijan UI:lle: ilman tätä epäonnistuneet
runit eivät erotu onnistuneista, ja "miksi tämä viesti ei vastannut"
-kysymys jää vastaamatta.

## Scope

### Status-mappaus `finalize_run(...)`:ssä

| Tilanne `process_with_tools`:ssa                | `agent_runs.status`         | `error_class`       |
|--------------------------------------------------|------------------------------|---------------------|
| `StopReason::EndTurn` / `StopSequence`          | `completed`                  | NULL                |
| `StopReason::MaxTokens`                         | `truncated_max_tokens`       | `max_tokens`        |
| `MAX_TOOL_ITERATIONS` ylittyi                   | `aborted_max_iterations`     | `max_iterations`    |
| `MAX_AGENT_WALL_CLOCK` ylittyi                  | `aborted_wall_clock`         | `wall_clock`        |
| `AgentError::Transient(...)`                    | `failed_transient`           | `transient`         |
| `AgentError::Permanent(...)`                    | `failed_permanent`           | `permanent`         |
| `AgentError::Database(...)`                     | `failed_database`            | `database`          |
| `StopReason::Unknown` + tool_use                | `failed_transient`           | `unknown_stop_reason` |

`error_message` on aina alkuperäisen virheen `to_string()`.

### `agent_steps.kind='error'`-rivit

Lisätään silloin kun:
- Tool-suoritus palauttaa `is_error=true` mutta loopi jatkaa (tämä on
  jo tool_use-rivin `tool_is_error=true` kentässä — **älä toista**;
  pelkkä `kind='error'`-rivi syntyy vain jos virhe ei kuulu tooliin
  vaan looppiin itseensä)
- `tools::execute`:n timeout (30 s) — tämä on tool-virhe → tool_use-rivi
  `tool_is_error=true`. Ei erillistä error-riviä.

**Päätös:** `kind='error'`-riviä käytetään **vain loopin sisäisille
tilanteille jotka eivät kuulu LLM-callin tai tool-callin alle** (esim.
`render_user_md` epäonnistuu mid-loop). Jos sellaisia ei ole, tämä
kategoria voi jäädä käyttämättä — riittää että `agent_runs.status` +
`error_message` kattavat virheet.

### Trace-id-bridging lokin kanssa

Varmistetaan että `agent_runs.trace_id` = `tracing`-spanin
`trace_id`-kenttä (`main.rs:575`:n `Uuid::new_v4()`). Tämä on
välttämätön debugille: lokirivien grep:llä saadaan run, ja vice
versa.

## Out of scope

- Manuaaliset runit (#62)
- UI (Phase 4)
- Hälytykset Mattermostiin failed_transient-pohjalta (#16 obs)

## Riippuvuudet

- **Estyy:** #58, #59 (writer + happy-path instrumentointi)
- **Estää:** Phase 4 (asiantuntijan UI tarvitsee erottelun
  onnistunut/epäonnistunut)

## Acceptance criteria

- Kaikki `AgentError`-haarat mappautuvat oikeaan `agent_runs.status`-
  arvoon
- `#49`-tyylinen MaxTokens-tapaus näkyy `agent_runs.status =
  'truncated_max_tokens'`-rivinä
- Wall-clock-overrun + max-iterations -aborti tuottavat erilliset
  status-arvot (helppo erottaa retry-loop vs. todellinen LLM-
  jumitilanne)
- `agent_runs.trace_id` löytyy lokirivien `trace_id`-kentästä, ja
  käänteisesti
- Asiantuntijakysymys "tämän viikon failed_transient-runit" yhdellä
  SQL:llä

## Toteutus (2026-04-30, `agent-trace-errors-aborts`-worktree)

Refaktoroitu `crates/server/src/ingest/agent/`-tasolla:

- **`trace.rs`** — uudet helperit `map_failure_status(error: &AgentError)
  -> (RunStatus, &'static str)` (geneerinen `Transient`/`Permanent`/
  `Database` → `failed_*` / `transient`|`permanent`|`database`) ja
  `finalize_failure(handle, status, error_class, error_message,
  iterations, tokens)` (täydentää `finalize`:n stamppaamalla `error_*`-
  kentät). Vanha `happy_path_status` poistettu — terminal-status
  lasketaan nyt suoraan loopin break-haaroissa, joten happy-/failure-
  mappaus ei voi ajautua erilleen.
- **`mod.rs`** — `process_with_tools` sai uuden inner-funktion
  `run_loop`, joka palauttaa `Result<(AgentReply, RunStatus),
  LoopFailure>`. Wrapper kutsuu `trace::start`:n, ohjaa run_uuid:n
  `tool_ctx_owned.trace_id`:hen kuten ennenkin, ja Ok/Err-haarat
  finalisoivat tarkalleen yhdellä kutsulla — happy → `trace::finalize`,
  fail → `trace::finalize_failure`. `LoopFailure`-struct + 4
  konstruktoria (`from_error`, `aborted_max_iterations`,
  `aborted_wall_clock`, `unknown_stop_reason`) keskittävät
  `(status, error_class)`-mappauksen rakennusvaiheeseen, jotta
  yksittäiset failure-haarat ovat unit-testattavissa ilman
  mock-pinta. **Failure-haarat (mappaus issue §"Status-mappaus":n
  mukainen):**
  - `MAX_TOOL_ITERATIONS` ylittyi → `aborted_max_iterations` /
    `max_iterations` (`Permanent`-virhe IMAP-retry-jonolle, koska
    runaway-loop ei korjaudu uudelleenajolla)
  - `MAX_AGENT_WALL_CLOCK` ylittyi (pre- ja post-LLM-tarkistukset) →
    `aborted_wall_clock` / `wall_clock` (säilyttää
    `check_wall_clock_budget`:n palauttaman `Transient`-virheen)
  - `StopReason::Unknown` + tool_use-blokit → `failed_transient` /
    `unknown_stop_reason` (erottuu yleisistä transienteista)
  - `client.send` epäonnistuu → `Transient` tai `Permanent`
    `is_transient()`:in mukaan → `failed_transient` / `failed_permanent`
  - `render_user_md` epäonnistuu → `failed_database` / `database`
  - `tool_uses.is_empty()` → `failed_permanent` / `permanent`
  - `text.trim().is_empty()` → `failed_permanent` / `permanent`
  Wrapper passaa `error.to_string()`:n `error_message`-kentäksi.
- **`crates/server/AGENTS.md`** — "Agent-trace instrumentation"-osio
  laajennettu: täysi status-mappaustaulu (8 riviä), `LoopFailure`-
  struct mainittu, `kind='error'`-rivien tämänhetkinen ei-käyttö
  dokumentoitu (issue §"Päätös": loop-sisäiset virheet, jotka eivät
  kuulu `LlmCall`/`ToolUse`-stepin alle, surfacaavat tällä hetkellä
  vain `agent_runs.error_class + error_message`-kombinaationa eikä
  erillisellä error-step-rivillä).

**Trace-id-bridging** vahvistettu toimivaksi failure-haaroissa: D3:ssa
landannut `Span::current().record("trace_id", run_uuid)` tapahtuu
`trace::start`-vaiheessa, eli kaikki failure-haaran logit (mukaan
lukien wall-clock-error, LLM-error, render-error) sisältävät
`trace_id`-kentän. Sama UUID kirjoittuu `agent_runs.trace_id`-sarakkeeseen
`start_run`:n parametrina — log↔DB-join-avain pitää.

**`runner.rs` ei vaadi muutoksia.** `process_with_tools` omistaa
`agent_runs`-rivin koko elinkaaren ja finalisoi sen niin Ok- kuin Err-
haarassakin — ulkokuori (`process_message_inner`) näkee vain
`Result<AgentReply, AgentError>` eikä tiedä run_uuid:sta mitään.
Ennen `process_with_tools`:n kutsua agent-rivi ei ole vielä syntynyt,
joten ulkomman tason finalize-kutsulle ei ole kohdetta.

### Päätös: `agent_steps.kind='error'`-rivit jätetään käyttämättä

Issue §"Scope" antoi luvan: "Jos sellaisia ei ole, tämä kategoria voi
jäädä käyttämättä — riittää että `agent_runs.status + error_message`
kattavat virheet." Audit-pinnan kannalta:

- **LLM-tason virheet** (transient/permanent client.send, max_tokens-
  truncation) liittyvät meneillään olevaan `LlmCall`-stepin — sen
  iteraation tokenit ja stop_reason ovat jo riveissä. Erillinen
  error-rivi vain duplikoisi tiedon.
- **Tool-tason virheet** (tool_is_error=true, dispatcher-timeout)
  surfacaavat `tool_use_id`:n ToolUse-rivillä `tool_is_error=true`
  + `tool_output`:in `ok=false`. Ei tarvetta erillistä error-riviä.
- **Loop-pinnan abortit** (max-iters, wall-clock) eivät kuulu
  yksittäisen LLM-/tool-stepin alle — niistä syntyy *agent_runs*-rivi
  `aborted_*`-statuksella + `error_class`+`error_message`. Asiantuntija
  näkee abortin `agent_runs`-tasolla.
- **`render_user_md`-failure mid-loop** (ainoa loop-sisäinen virhe
  joka on aidosti irrallaan `LlmCall`/`ToolUse`-stepeistä) tapahtuu
  ennen seuraavan iteraation LLM-kutsua, ei sen jälkeen — joten
  myös tämä surfacaa vain `agent_runs`-tasolla.

Schema sallii `kind='error'`-rivit jatkossa, jos asiantuntijan UI
(Phase 4) tarvitsee erottelun "miksi mid-loop-virhe ohjasi loopin
abortille". MVP:ssä `agent_runs.error_*`-kentät kattavat tarpeen.

### Smoke

- `cargo build -p grooveserve-server` puhdas (ei warning-rivejä)
- `cargo test -p grooveserve-server --lib` 297/297 vihreä — uusia
  unit-testejä 7 (`map_failure_status_covers_every_agent_error_variant`,
  `loop_failure_aborted_max_iterations_maps_correctly`,
  `loop_failure_aborted_wall_clock_preserves_inner_error`,
  `loop_failure_unknown_stop_reason_maps_to_failed_transient`,
  `loop_failure_from_transient_maps_to_failed_transient`,
  `loop_failure_from_permanent_maps_to_failed_permanent`,
  `loop_failure_from_database_maps_to_failed_database`)
- `cargo test -p grooveserve-ops --lib` 140/140 vihreä
- D3:n integraatiotestit (`crates/server/tests/agent_trace.rs`) eivät
  tarvinneet muutoksia — happy-path-rakenne säilyi.

## Post-LLM-review fix-up (2026-05-01, sama worktree, ennen mergeä)

`/llm-review`-kierros (Gemini, Anthropic, DeepSeek × 2 kierrosta;
OpenAI quota-failed) ja `/assess-findings`-arvio tuotti 6 FIX +
1 SPIN-OFF + 1 alkujaan DISCUSS:ksi merkitty joka revisoitiin
FIX:ksi root-AGENTS.md:n uuden "Päätökset ovat ohjaavia, eivät
sitovia" -periaatteen myötä. Kaikki landanneet samaan worktreeseen
ennen mergeä:

- **`MaxTokens` → `error_class='max_tokens'`** (spec drift):
  alkuperäinen toteutus mappasi `MaxTokens` happy-path:n läpi
  `error_class=None` -arvolla, vaikka issue §"Status-mappaus"
  vaati `'max_tokens'`. `trace::finalize` sai
  `error_class: Option<&'static str>` -parametrin, `run_loop`
  palauttaa `(RunStatus, Option<&'static str>)`-parin, MaxTokens-
  break antaa `Some("max_tokens")`. Integraatiotesti laajennettu
  varmistamaan `agent_runs.error_class = 'max_tokens'`
  truncation-runille ja `NULL` happy-runille (writer-invariantti).
- **`agent_steps.kind='error'`-rivit jokaisessa loop-pinnan
  failure-haarassa** (§Päätös revisoitu):
  `trace::record_loop_error`-helper lisätty;
  `record_step(Error)`-rivi syntyy max-iters/wall-clock-pre-LLM-
  abortissa (`iteration=NULL`), `render_user_md`-failissa,
  `client.send`-failissa, post-LLM-wall-clockissa,
  `unknown_stop_reason`+tool_use:ssa, empty-tool-uses:ssa, ja
  empty-final-text:ssä (kaikissa `iteration=*iterations`).
  Rationaalit `crates/server/AGENTS.md`:ssä ja #56 decision-lokissa.
- **`Display`-prefiksin poisto `error_message`-kolumnista**:
  `AgentError::message()`-accessor lisätty. Aiemmin
  `error.to_string()` tuotti `"transient: <msg>"` -kentän, joka
  duplikoi `error_class`-sarakkeen sisällön ja oli ristiriidassa
  esim. `aborted_wall_clock`/`error_class=wall_clock`-rivin kanssa.
  Wrapper käyttää nyt `error.message()`:a.
- **`#[tracing::instrument(fields(trace_id = Empty))]`
  `process_with_tools`:lle**: aiemmin `trace::start` luotti siihen
  että caller-side span (msg_span `runner.rs`:ssä) oli deklaroinut
  `trace_id = Empty` -kentän. Refactor-haavoittuva (silent no-op
  jos kenttä poistuu), ja **retry-polku oli rikki** —
  `retry_assistant_message`:n span ei deklaroinut kenttää, joten
  trace_id-bridging ei toiminut retry-runeissa. Span-kenttä on nyt
  paikallinen `process_with_tools`:lle; runner.rs:n msg_span
  -deklaraatio poistettu redundanttina.
- **Iterations off-by-one**: `*iterations += 1` siirretty
  pre-flight-checkien jälkeen. Aiemmin max-iters-abortti tallensi
  `iterations = MAX+1` (= 201) vaikka vain 200 LLM-kutsua oli
  tehty; pre-LLM-wall-clock-abortti tallensi iteraation joka ei
  ollut suorittanut LLM-kutsua. Nyt `agent_runs.iterations` ja
  `MAX(agent_steps.iteration)` pitävät yhtä.
- **Doc-fixit AGENTS.md:hen ja koodikommentteihin**: token-record-
  väärä korjattu (LLM-error-iteraation tokenit eivät ole
  saatavilla, ei "tallennetaan"); pre-loop-failures-unaudited
  -caveat lisätty; `completed` ≠ "user got reply" -caveat lisätty
  (defer #60 reply_sent-decision-rivit); cancellation-leak
  -caveat lisätty (defer #80).
- **#80 cancellation safety filed SPIN-OFF -issueksi**: kolme
  reviewerie yksimielisiä että `process_with_tools`-tulevaisuuden
  droppaus (SIGTERM-deploy 10 min wall-clockin sisällä, panic
  spawnatussa taskissa, runtime-shutdown) jättää `agent_runs`-
  rivin `running`-tilaan. Korjausvaihtoehdot ja päätös on oma
  scope.

**Smoke päivitetty:**
- `cargo build -p grooveserve-server` puhdas (0 warning)
- `cargo test -p grooveserve-server --lib` 297/297 vihreä
- `cargo test -p grooveserve-ops --lib` 140/140 vihreä
- `cargo test -p grooveserve-server --test agent_trace` 3/3 vihreä
  (MaxTokens-test laajennettu `error_class`-assertiolla, happy-test
  laajennettu `error_class IS NULL`-assertiolla)

Review-raportti: `history/review-agent-trace-errors-aborts.md`.

**Periaatemuutos root-AGENTS.md:hen samalla:** uusi
"Päätökset ovat ohjaavia, eivät sitovia" -osio tekee selväksi
että §Päätös-osiot, decision-loki ja AGENTS.md-konventiot ovat
parhaita arvauksia silloisilla tiedoilla; vain commit-historia /
migraatiot / deployatut ratkaisut ovat sitovia. Tämä antoi luvan
revisoida #61:n alkuperäinen `kind='error'`-rivien §Päätös
review-löydösten valossa, ilman että uusi päätös rikkoo proseduuria.
