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
related: ["#56", "#57", "#59"]
labels: [agent, observability, schema, db]
commits:
  - hash: d9fce2e
    summary: "feat(ops): agent_trace writer skeleton + lifecycle guards (#58)"
---

# 58. agent_trace migration + AgentTraceWriter skeleton

_Source: #57 Phase 2, alaissue 1/5_

## Description

Toteuta `agent_runs` + `agent_steps` -taulujen migraatio ja Rust-puolen
`AgentTraceWriter`-rajapinnan kuori. **Ei vielä kytketä agent-loopiin
tässä issuessa** — tarkoitus on saada schema validoitua ja kirjoitus-
rajapinta olemassa, jotta seuraavat issuet voivat kytkeä sen sisään
ilman että migraatio + writer-design häiritsee niiden review:ta.

Schema-päätökset ja perustelut: `issues/open/57-…/analysis.md` ja
`schema-draft.md` (D1-design, 2026-04-29).

## Scope

- **Migraatio** `migrations/NNN_agent_trace.sql`:
  - `agent_runs` taulu kaikki kentät, indeksit, FK:t
    `tenants`/`users`/`threads`-tauluihin
  - `agent_steps` taulu kaikki kentät, indeksit, FK:t
    `attachments`/`extractions`/`receipts`/`thread_messages`-tauluihin
  - `pgcrypto` extension jos puuttuu
  - Migraationumero valitaan PR:ssä #56 Phase 1:n migraatioiden
    lopputilan mukaan
- **Rust-moduuli** `services/email/src/agent_trace.rs`:
  - `AgentTraceWriter` -trait/struct
  - `start_run(...) -> RunId`
  - `record_step(...)`
  - `finalize_run(run_id, status, ...)`
  - Tyyppejä: `RunId`, `StepKind` (`LlmCall`, `ToolUse`, `Decision`, `Error`),
    `RunStatus`-enum (peilaa CHECK-rajoitusta)
- **Integraatiotesti** `tests/agent_trace.rs` (sqlx):
  - Verifioi että migraatio onnistuu
  - Pyöräyttää writer-rajapintaa: insertoi run + N steppiä, finalisoi,
    lukee ja vertaa
  - Cascade-delete kun user poistetaan

## Out of scope

- **Agent-loopin kytkentä** — issue #59
- **Decision-rivit** — issue #60
- **Error-/abort-rivit** — issue #61
- **Manuaaliset runit** — issue #62

## Riippuvuudet

- **Estyy:** odottaa #56 Phase 1:tä (yhteinen DB-skeema). Kun A1 on
  saanut yhdistetyn skeeman valmiiksi, tämä migraatio ajetaan siihen.
- **Estää:** #59, #60, #61, #62 (kaikki tarvitsevat writer-rajapinnan).

## Acceptance criteria

- `cargo sqlx migrate run` onnistuu sekä tyhjälle että olemassa olevalle
  DB:lle ✅ (A3:n migraatio 018 ajetaan `MIGRATOR.run` -kutsulla
  jokaisessa sqlx-testissä)
- Integraatiotesti vihreä ✅ (`crates/ops/tests/agent_trace.rs`,
  19 testiä — happy-path, cascade run→steps, blocked user-delete,
  failed-status roundtrip, duplicate-seq, double-finalize rejected,
  post-finalize step rejected, finalize-with-Running rejected,
  aborted_max_iterations roundtrip, cross-tenant RunRef tampering
  rejected by composite-FK, system-context Forbidden,
  message_id length validation, empty model rejected,
  Completed-with-error_message rejected, negative iterations rejected,
  oversized tool_input rejected, empty tool_use_id rejected,
  negative seq rejected client-side)
- `agent_trace::start_run` + `record_step` + `finalize_run` löytyvät
  free-funktioina ja palauttavat odotetut tyypit ✅
- FK-rajoitukset toimivat ✅ — käytännön cascade A3:n schemassa on
  `agent_runs → agent_steps` (ON DELETE CASCADE composite-FK:n yli),
  ei `users → agent_runs`. Schema design valitsi tarkoituksellisesti
  NO ACTION käyttäjien deletoinnille (ks. `002_create_users.sql`:n
  kommentti) — taloussovelluksen audit-trail ei saa kadota
  käyttäjähallinnan oheisvaikutuksena. Testi todistaa molemmat:
  cascade run→steps onnistuu, user-delete blokkautuu.

## Tuotos (D2, 2026-04-30)

### Pinta

- `crates/ops/src/agent_trace.rs` — free-funktiot
  `start_run`/`record_step`/`finalize_run` (jokainen
  `db: impl PgExecutor<'_>`) + tyypit (`RunRef`, `StepKind`,
  `RunStatus`, `StopReason`) + per-kind input-variantit
  (`LlmCallStep`, `ToolUseStep`, `DecisionStep`, `ErrorStep`) jotka
  peilaavat migraation 018 strict-CHECK-rajoituksia (vaadittu kenttä
  per kind on osa enum-varianttia, ei flat-funktion `Option`-parametri).
- `crates/ops/tests/agent_trace.rs` — sqlx-integraatiotesti
  (19 testitapausta).
- `crates/ops/AGENTS.md` ja `crates/ops/CLAUDE.md` päivitetty.
- `crates/ops/src/error.rs` — uusi `OpError::Conflict(String)` variantti
  lifecycle-rikkomuksia varten (double-finalize, post-finalize
  step-kirjoitus).
- `crates/ops/src/context.rs` — `OpContext.trace_id`-rustdoc korjattu
  (`agent_runs.run_id` → `agent_runs.run_uuid`; lisätty selkeä erottelu
  domain-level run-correlation ID:n ja distributed-tracing span UUID:n
  välillä).

### Lifecycle-invariantti

`finalize_run`:n UPDATE on suojattu `WHERE status = 'running'`-ehdolla
ja `record_step`:n INSERT käyttää `INSERT … SELECT … WHERE status =
'running'`-rakennetta. Kun run on finalizoitu, sekä toinen
`finalize_run` että `record_step` palauttavat `OpError::Conflict` —
audit-rivit eivät kirjoita yli toisiaan eivätkä jälkikäteen.
`finished_at = GREATEST(NOW(), started_at)` torjuu `started_at >=
finished_at`-CHECK-rikkomuksen jos NTP step-säätö heittäisi kelloa
taaksepäin start- ja finalize-tx:n välillä.

### Validation- ja security-pintaa

- `start_run` torjuu `OpContext::system()` (`tenant_id <= 0` tai
  `actor_user_id <= 0`) → `OpError::Forbidden`.
- `start_run` validoi `message_id` 3..=512 (mirror schema CHECK) ja
  ei-tyhjän `model`-arvon.
- `finalize_run` torjuu `RunStatus::Running` (`InvalidInput`),
  negatiiviset counterit, ja `Completed`-statuksen yhdessä
  `error_class`/`error_message`-arvojen kanssa.
- `record_step` validoi `seq >= 1`, `iteration >= 1`,
  `duration_ms >= 0`, ei-tyhjät `tool_name`/`tool_use_id`/
  `decision_type`/`ErrorStep.error`-arvot, ja JSON-payloadit
  cap-rajoissa: `MAX_TRACE_JSON_BYTES = 64 KB` ja
  `MAX_TRACE_TEXT_BYTES = 8 KB`.
- `RunRef`-fieldit `pub(crate)` (ei pub) + read-accessorit — ei
  external constructionia, joka mahdollistaisi within-tenantin run-
  id-spoofingin.
- `sqlx::Error::Database`-virheet mapataan `OpError`:iin:
  unique_violation (23505) → `AlreadyExists`, foreign_key_violation
  (23503) → `Forbidden`, check_violation (23514) → `InvalidInput`.
  Caller saa typed-virheen sen sijaan että kaikki konfliktit
  collapsoituisivat samaan `Database`-varianttiin.

### Observability

`#[tracing::instrument(target = "ops::agent_trace", skip_all,
fields(...))]` jokaisella kolmella funktiolla; payloadeja ei logiteta,
metadata (run_id, run_uuid, tenant_id, user_id, seq, kind, status)
on logirivissä.

### Suunnittelupäätökset

- `agent_runs.message_id` säilytetään `TEXT`-muodossa (RFC 5322
  Message-ID, angle-brackets); ei `Uuid` kuten issue-luonnoksen sketch
  ehdotti — schema voitti.
- `RunStatus`-enumin variantit peilataan migraation 018 CHECK-listaan
  (8 arvoa: `Running` + 7 terminal-tilaa) eikä issue-sketchin
  yleisempiin nimiin.
- `AgentTraceWriter`-struct **dropattu**: rakenne oli stateless
  pool-wrapper ja convention-rikkoja. Free-funktiot
  `db: impl PgExecutor<'_>`-parametrilla on yhteensopiva sekä
  `&PgPool`:n että `&mut Transaction`:n kanssa, joten #59 voi siirtää
  trace-rivit samaan transaktioon kuin tool-effectinsä.
- `seq` on caller-managed (single-threaded agent-loop, `UNIQUE (run_id,
  seq)`-constraint kiinniottaa counter-bugit). Ei writer-side state-
  hallintaa joka pakottaisi locking-mallin.
- `decision_type` ja `error_class` jäävät `&str`:iksi tähän writeriin
  (D2:n scope ei kirjoita kumpaakaan); #60 ja #61 lisäävät
  `KnownDecisionType` / `AgentErrorClass`-enumit kun ne kirjoittavat
  konkreettisia arvoja.

### LLM-review (4 mallia, 2 kierrosta)

LLM-review (`history/review-d2-agent-trace-writer.md`) tunnisti kolme
critical-tason puutetta: lifecycle-rikkomukset, executor-parametrin
puuttuminen ja unbounded JSONB. Kaikki kolme korjattu tässä toteutuksessa
ennen merget. Disputed-kohdat (server-side aggregaatit, type-state
pattern, run-scoped recorder) hylättiin perustelluilla syillä.
