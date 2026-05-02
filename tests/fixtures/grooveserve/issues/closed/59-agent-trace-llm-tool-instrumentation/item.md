---
created: 2026-04-29
updated: 2026-04-30
closed: 2026-04-30
type: task
reporter: jari
assignee: jari
status: done
priority: high
epic: 57
related: ["#56", "#57", "#58", "#60"]
labels: [agent, observability]
commits:
  - hash: 40389ef
    summary: "feat(server/ingest): capture Anthropic request-id from response headers"
  - hash: 4ff04bb
    summary: "feat(tools): plumb trace_id through ToolContext to OpContext"
  - hash: b1cff93
    summary: "feat(server/ingest): add agent_trace helper module"
  - hash: f14139e
    summary: "feat(server/ingest): instrument process_with_tools with start_run + record_step + finalize_run"
  - hash: ee0770d
    summary: "docs(server): document agent-trace instrumentation"
  - hash: 8c39f12
    summary: "docs(issues): close #59 done; update #56 worktree-loki + #57 Phase 2"
  - hash: 5a9e261
    summary: "docs(issues): backfill #59 commits + #56 D3 hash range"
  # Post-LLM-review fix-up (2026-04-30, same D3 worktree, pre-merge):
  - hash: 11797d4
    summary: "fix(server/ingest): cap tool_input symmetrically with tool_output"
  - hash: 6b9b298
    summary: "fix(server/ingest): include cache tokens in agent_run totals"
  - hash: b4e43a3
    summary: "fix(server/ingest): handle Unknown stop_reason and empty content blocks"
  - hash: eb658fc
    summary: "fix(server/ingest): UTF-8-safe truncate of Anthropic error bodies"
  - hash: 7e2d786
    summary: "feat(server/ingest): expose request-id on ApiError::HttpError"
  - hash: b45b046
    summary: "fix(server/ingest): record agent_run trace_id on message span"
  - hash: 14142bb
    summary: "docs(ops): clarify agent_trace transactional contract is opt-in"
  - hash: cdbb189
    summary: "fix(server/ingest): repair render_user_md for post-A3 schema"
  - hash: 6abbadf
    summary: "test(server/ingest): integration test for agent_trace happy path"
---

# 59. process_with_tools instrumentointi (llm_call + tool_use)

_Source: #57 Phase 2, alaissue 2/5_

## Description

Kytke `AgentTraceWriter` (#58) `services/email/src/agent/mod.rs`:n
`process_with_tools`-funktioon, jotta jokainen LLM-iteraatio ja
tool_use kirjoittaa rivin `agent_steps`-tauluun ja yksi
`agent_runs`-rivi syntyy per kutsu.

**Vain happy path tässä issuessa.** Decision-rivit (#60) ja
error-/abort-rivit (#61) tulevat omina issueinaan, jotta tämä PR
pysyy yksinkertaisena.

Schema + päätökset: `issues/open/57-…/analysis.md` §3.1, §5.4 (PII).

## Scope

- `process_with_tools` ottaa `&AgentTraceWriter` (tai hakee sen
  kontekstista — päätetään PR:ssä) ja:
  - **Loopin alussa:** `start_run(...)` (model, message_id, thread_id,
    tenant_id, user_id, trace_id sama kuin tracing-spaniin)
  - **Jokaisen LLM-vastauksen jälkeen:** `record_step(kind=LlmCall,
    iteration, stop_reason, input_tokens, output_tokens, api_request_id,
    duration_ms)`
  - **Jokaisen tool-suorituksen jälkeen:** `record_step(kind=ToolUse,
    iteration, tool_name, tool_use_id, tool_input, tool_output,
    tool_ok, tool_is_error, duration_ms)`
  - **Loopin lopussa (happy path):** `finalize_run(run_id,
    status=Completed | TruncatedMaxTokens, iterations,
    total_input_tokens, total_output_tokens)`
- **PII (D1 §5.4):** LLM-iteraation `input_json`/`output_json` **ei
  tallenneta**. Vain stop_reason + tokens + iteraationumero + kesto.
- **Cap:** tool_input/tool_output käyttävät samaa
  `cap_tool_result_json`:ia kuin agent-loopissa (~64 KB).
- **API-request-id:** jos Anthropic-API palauttaa response-headereissa
  request-id:n, kapseloi se llm-clientiin ja välitä `record_step`:lle.

## Out of scope

- Decision-rivit (`spam_skip`, `policy_reply`, `permanent_skip`,
  `reply_sent`, …) — #60
- Error-/abort-status-mappaus (failed_transient, aborted_max_iterations
  jne.) — #61
- Manuaalisten runien kirjaus — #62

## Riippuvuudet

- **Estyy:** #58 (writer-rajapinta + migraatio)
- **Estää:** #60, #61

## Acceptance criteria

- Demoa ajaessa (Roundcube round-trip) jokaisesta `assistant@`-
  viestistä syntyy yksi `agent_runs`-rivi statuksella `completed` tai
  `truncated_max_tokens`
- Stepien lukumäärä vastaa lokin `Agent iteration` + `Executing tool`
  -rivien yhteismäärää
- `agent_steps.tool_input` / `tool_output` ovat luettavissa ja capatuvat
  ~64 KB:hen
- `agent_steps.iteration` sarake yhtyy `agent_runs.iterations`-kentän
  maksimiin
- LLM-iteraation rivissä `tool_input` / `tool_output` ovat NULL (D1
  §5.4 — PII)

## Toteutus (2026-04-30, D3-worktree)

Implementoitu `crates/server/src/ingest/agent/`-tasolla:

- **`trace.rs`** (uusi) — kokoaa `grooveserve_ops::agent_trace`-pinnan
  agent-loopin tarpeisiin. `TraceHandle` + neljä helperia:
  `start`, `record_llm_call`, `record_tool_use`, `finalize`.
  Trace-virheet logataan `warn`-tasolla, eivät katkaise agent-loopia.
- **`mod.rs`** — `process_with_tools` kutsuu loopin alussa
  `trace::start`:n, jokaisen LLM-iteraation jälkeen
  `record_llm_call`:n (PII-vapaa: vain stop_reason + tokens +
  iteration + duration + api_request_id), jokaisen tool-suorituksen
  jälkeen `record_tool_use`:n (cap 4 KB sis. `cap_tool_result_json`),
  ja loopin lopussa `finalize`:n happy-path-statusmappingilla
  (`EndTurn`/`StopSequence` → `Completed`,
  `MaxTokens` → `TruncatedMaxTokens`).
- **`llm/mod.rs` + `llm/types.rs`** — `MessagesResponse.request_id`
  -kenttä lisätty (`#[serde(skip)]`), `do_send` poimii Anthropic-API:n
  `request-id`-headerin ja propagoi sen `LlmCall`-stepille.
- **`tools/context.rs`** — `ToolContext`:iin lisätty
  `trace_id: Option<String>`. `op_context_from_tool` välittää sen
  `OpContext.trace_id`:hin, joten kaikki run-aikaiset ops-kutsut
  (receipt-write, audit-rivit) saavat saman korrelointiavaimen.
  `process_with_tools` täyttää sen `RunRef::run_uuid()`:lla heti
  `start_run`:n jälkeen. ToolContext-rakentajat päivitetty kaikkialla
  (`runner.rs` 3 callsite-paikkaa, dispatch/read_skill testit,
  `extraction_rescue`-integraatio-fixture).

**Status-mappaus pidetty tiukasti happy pathissa.** Failure-haarat
(`Permanent`, `Transient`, `Database`, max-iterations,
wall-clock-budget overruns, `unknown_had_tool_use`) jättävät
`agent_runs`-rivin `running`-tilaan ja kirjaavat warn-rivin —
#61:n työnä on mappaus `RunStatus::FailedTransient` /
`FailedPermanent` / `AbortedMaxIterations` / `AbortedWallClock` jne.

**Out-of-scope vahvistus:** ei decision-rivejä (#60), ei
manuaalisia runeja (#62), ei error-/abort-status-mappausta (#61).

## Smoke

- `cargo build --workspace` puhdas
- `cargo test -p grooveserve-server --lib` 277/277 vihreä
  (uudet 5 unit-testiä `trace.rs`:ssä)
- `cargo test -p grooveserve-ops --tests` 105/105 vihreä
  (D2:n 19 agent_trace-integraatiotestiä mukana)
- 26 pre-existing failure (`claim_with_thread`/`extraction_rescue`/
  `unknown_sender`) toistuvat sekä baseline-commitilla 28462f0 että
  D3-worktreessä — `tenants.slug NOT NULL` -fixture-bug, ei D3:n
  aiheuttama. Ei korjata tässä worktreessä.
- **Lokaali Roundcube round-trip** on käyttäjän tehtävä: kun
  `gsdev` -stack on käynnissä ja `assistant@`-tilille lähetetään
  viesti, DB:stä pitäisi näkyä yksi `agent_runs`-rivi tilassa
  `completed`/`truncated_max_tokens` ja sarja `agent_steps`-rivejä,
  joista `LlmCall`-rivit ovat ilman `tool_input`/`tool_output`:ia
  ja `ToolUse`-rivit kapatun JSON:in kanssa. `gs-dev dev trace
  <run_uuid>` näyttää käsittelyn.

## Post-LLM-review fix-up (2026-04-30, sama D3-worktree)

`/llm-review`-kierros (Gemini, OpenAI, Anthropic, DeepSeek × 2 kierrosta)
ja `/assess-findings`-arvio tuotti 8 korjausta + 1 sisarbugin paljastuksen.
Kaikki landanneet samaan worktreeseen ennen mergeä:

- **`tool_input` capping symmetric `tool_output`:n kanssa** (`11797d4`).
  Pre-fix: model voi emittoida >64 KB JSON:in, D2:n writer torjuu sen ja
  D3 droppaa step-rivin warn-lokilla. Cap nyt 4 KB symmetrisesti,
  oversize → stub-Value byte-koolla.
- **Cache-tokenit summattu input-totaaleihin** (`6b9b298`). Block 1 on
  cached, joten `usage.input_tokens` oli pre-fix lähes nolla joka
  iteraatiolla; total_input_tokens oli systemaattisesti väärä.
- **Unknown stop_reason ilman tool_use → `Completed`** (`b4e43a3`).
  Pre-fix: happy-path-haara joka jätti runin ikuisesti `running`-tilaan,
  rikkoi #59:n acceptance-kriteerin. Sama commit lisää
  `drop_empty_text_blocks` joka torjuu Anthropic-API:n 400-virheen
  forward-compat unknown-block-tyypeille (`thinking` etc.).
- **`truncate` UTF-8-turvallinen** (`eb658fc`). Pre-fix: byte-slice
  paniikoi non-ASCII error-bodyihin (Cloudflare/Stalwart proxy-sivut).
- **`request-id` `ApiError::HttpError`:lle** (`7e2d786`). Pre-fix:
  Anthropic-incidenttien debuggaus ilman failed-attempt request-id:tä.
- **Span-trace_id-kytkentä** (`b45b046`). Pre-fix: `agent_runs.trace_id`
  oli random UUID jota ei kirjattu mihinkään lokiin → log↔DB-join-avain
  rikki. Nyt msg-spanin `trace_id`-kenttä saa run_uuid:n
  `Span::current().record`-kautta. Sama commit lisää defensive
  `tool_ctx_owned.trace_id = None;` -clearin.
- **D2 transaktio-doc reconciled** (`14142bb`). Pre-fix: D2:n
  moduulidoc lupasi transaktionaalisuutta jota D3 ei käytä —
  pehmennetty MVP-best-effort -reaalitilanteeseen.
- **`render_user_md` repair** (`cdbb189`). Sivu-bugi joka paljastui
  integraatiotestin pohjustuksessa: SQL-kysely viittasi
  `users.tenant_id`/`u.email`/`u.role`-sarakkeisiin jotka A3:n
  schema-rework siirsi `tenant_users`:lle ja `user_emails`:lle. Pre-fix
  agent-loop **kaatui jokaisella oikealla viestillä**
  (`column u.tenant_id does not exist`). Out-of-strict-scope mutta
  blokkasi integraatiotestin ja oli iso prod-bugi joka tarvitsi
  korjauksen.

**Integraatiotesti** (`6abbadf`): `crates/server/tests/agent_trace.rs` —
3 testitapausta wiremock-pohjaisella Anthropic-mockilla, ajavat
todellista `process_with_tools`:ia ja assertaavat `agent_runs`/
`agent_steps`-rivien shape:in. Sis. happy path (1 LLM + 1 tool +
EndTurn), MaxTokens-haara, multi-tool-batch yhdellä iteraatiolla.
Acceptance-kriteerit nyt automaattisesti todistettavissa:

- 1 `agent_runs` per `process_with_tools`-kutsu, status=`completed`
  tai `truncated_max_tokens`
- N+1 step-rivit (1 LlmCall per iteraatio + N ToolUse per tool)
- `step_seq` monotonic
- LlmCall-rivit `tool_input/output IS NULL` (PII)
- ToolUse-rivit `tool_input/output` populated capatulla JSON:illa
- `agent_runs.iterations` = MAX(`agent_steps.iteration`)
- `request-id` propagoituu `agent_steps.api_request_id`:ksi

Smoke päivitetty:
- `cargo test -p grooveserve-server --lib` 285/285 vihreä (4 uutta)
- `cargo test -p grooveserve-server --test agent_trace` 3/3 vihreä
- `cargo test -p grooveserve-ops --tests` 105/105 vihreä
- 26 pre-existing failure (`tenants.slug` -fixture-bug) ennallaan

Review-raportti: `history/review-d3-agent-trace-process-with-tools.md`.
