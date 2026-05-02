---
created: 2026-04-29
updated: 2026-04-29
type: readiness
owner: jari
status: draft
related: ["#56", "#57"]
---

# D1 — Phase 2 -valmius

Mitä #57 Phase 2 (tallennuksen toteutus) tarvitsee voidakseen alkaa,
ja mitä alaissueita kannattaa luoda kun design on hyväksytty.

---

## Riippuvuudet ennen Phase 2:n aloitusta

1. **#56 Phase 1 (yhdistetty schema + ops) valmis.** Trace-taulut
   kuuluvat yhteiseen DB:hen — sekä email-puoli (kirjoittaa) että
   web-puoli (lukee Phase 3/4:ssä) tarvitsevat saman skeeman.
   **2026-04-30 status: VALMIS.** A3-worktree toteutti
   `agent_runs`/`agent_steps`-skeeman migraatiossa
   `crates/ops/migrations/018_create_agent_trace.sql`
   `schema-draft.md`-luonnoksen mukaisesti, ja A4b-worktree lisäsi
   pinnan jonka päälle writerit nojautuvat:
   - `OpContext.trace_id: Option<String>` kaikissa ops-kutsuissa
     (`crates/ops/src/context.rs`).
   - `ops::ingest::process_message(db, ctx, ProcessMessageInput)` —
     dokumentoitu DB-puolen entry inbound-viestille
     (`crates/ops/src/ingest/mod.rs`). Tänään tekee vain
     sender-resolutionin (`SenderResolved` / `UnknownSender`); D-aalto
     laajentaa sen kirjoittamaan `agent_runs`-rivin ja palauttamaan
     `run_uuid`:n kutsujalle.
   - `crates/server/src/tools/context.rs::op_context_from_tool` siirtää
     `ToolContext`:in `message_id`:n `OpContext.trace_id`:ksi
     väliaikaisena placeholderina kunnes #58 mintaa oikeat run_uuid:t.

   D-wave-worktreet (#58–#62) voivat siis aloittaa heti — Phase 1 on
   valmiiksi maaliin viety, Phase 1 -checkboxit ovat täynnä.
2. **Tämän designin (D1) päätökset hyväksytty:** retentio, PII-
   politiikka, päätösten taksonomia, erillisyys `audit_events`:ista.
   Käyttäjän vastaus `analysis.md` §7:n avoimiin kysymyksiin riittää.

Ei ennen näitä:

- #38 (receipt-revision-history) toteutus — samanaikainen Phase 2:n
  kanssa, ei estävä.
- #26 multi-tenant-kayttajahallinta toteutus — `agent_runs` käyttää
  `tenants`/`users`-tauluja jotka jo ovat olemassa, joten tarkka
  Phase 2 -ajoitus suhteessa #26:n etenemiseen ei ole pullonkaula.

---

## Phase 2:n työ tiivistettynä

1. **~~Migraatio:~~** **toteutettu A3:ssa**
   (`crates/ops/migrations/018_create_agent_trace.sql`, 2026-04-29).
   Phase 2 ei tarvitse luoda uutta migraatiota — kirjoitusrajapinta
   voi suoraan INSERT-aikaa olemassa olevaan skeemaan. (#58
   alaissueen scope kapenee tähän: ei migraatiota, vain skeleton
   writer + integraatiotesti.)
2. **Kirjoitusrajapinta:** `services/email/src/agent_trace.rs`
   (uusi moduuli) joka tarjoaa `AgentTraceWriter`-rajapinnan:
   - `start_run(...) -> RunId` — INSERT `agent_runs`
   - `record_step(...)` — INSERT `agent_steps`
   - `finalize_run(run_id, status, ...)` — UPDATE `agent_runs`
3. **Instrumentointi:** `process_with_tools` ottaa `&AgentTraceWriter`
   ja kutsuu sitä loopin reunoilla:
   - LLM-iteraation jälkeen → `record_step(kind=llm_call, ...)`
   - Tool-suorituksen jälkeen → `record_step(kind=tool_use, ...)`
   - Loopin lopussa → `finalize_run(...)`
4. **Päätösten kirjaus** `process_message_inner`:n reunoilla:
   - `spam_skip` (ennen LLM:ää)
   - `unknown_sender`
   - `policy_reply` (#46)
   - `permanent_skip` per liite (extraction.rs:n `persist_permanent_skip`
     reuna)
   - `reply_sent` lopussa
   - `reply_truncated` (#49) MaxTokens-haaralla
   Kaikki kulkevat saman `AgentTraceWriter`:in läpi, mutta osa kirjoittaa
   `agent_runs`-rivin ilman LLM-iteraatioita (esim. `spam_skip`,
   joka ei ole agent-suoritus mutta käyttää samaa tracea
   konsistenssin vuoksi — `iterations=0`, `model='none'`).

---

## Phase 2 -alaissueet (luotu 2026-04-29)

Phase 2 on pilkottu viideksi peräkkäiseksi alaissueksi, jotta jokainen
askel on testattava ja PR-koko pieni. Järjestys = riippuvuus.

| # | Issue | Sisältö |
|---|-------|---------|
| 1 | **#58** agent_trace migration + AgentTraceWriter skeleton | Migraatio + tyhjä writer + integraatiotesti, ei vielä kytketty agent-loopiin. Verifioi schema. |
| 2 | **#59** process_with_tools instrumentointi (llm_call + tool_use) | Agent-loop kirjoittaa run+steps. Vain happy path. |
| 3 | **#60** agent_trace decision-rivit pre-LLM + post-LLM reunoilla | spam_skip, unknown_sender, policy_reply, permanent_skip, reply_sent, reply_truncated. |
| 4 | **#61** agent_trace error-/abort-rivit ja status-mappaus | AgentError-luokat → agent_runs.status. MaxTokens, MaxIterations, WallClock. |
| 5 | **#62** Manuaalisen runin pseudoluonti (Phase 4 ennakointi) | `record_manual_run(...)` — käytetään myöhemmin asiantuntijan undossa. Ei UI:ta vielä. |

Kaikki viisi blokattu **#56 Phase 1:n** valmistumiseen (yhteinen DB).

---

## Kytkennät muihin worktreihin (informatiivinen)

- **#38 (receipt-revision-history):** Phase 2:n alaissue 2 +
  #38:n migraatio — `receipt_revisions.created_by_run_id` voidaan
  lisätä yhdessä tai eri PR:nä. Ei estävä.
- **A1 (#56 Phase 1):** kirjasi tähän designiin "yhden DB:n malli on
  selvästi parempi tämän designin näkökulmasta" (`analysis.md` §6).
  A1 päättää lopullisesti, mutta tämä input pitää näkyä #56:n decision
  logissa.
- **Phase 3 / 4 / 5:** käyttävät tätä schemaa lukemiseen, eivät
  vaikuta Phase 2:n schemaan. Mutta Phase 4:n "manuaalinen revert"
  hyödyntää alaissue 5:ttä — pidä `record_manual_run`-rajapinta
  Phase 2:n scope:ssa, vaikka UI tuleekin myöhemmin.

---

## Mittaus ja tarkistus Phase 2:n jälkeen

Kun Phase 2 on käytössä, ennen Phase 3:n UI-toteutusta:

- Ajetaan demoja, varmistetaan että `agent_runs`-rivejä syntyy
  jokaisesta käsittelystä (tarkista `count(*) FROM agent_runs WHERE
  started_at > '...'` vs. `email_processing` saman ajan rivit).
- Verifioidaan että #46:n permanent skip + policy reply -tapaukset
  ovat löydettävissä:
  - `SELECT * FROM agent_steps WHERE kind = 'decision' AND
    decision_type = 'permanent_skip'`
  - `SELECT * FROM agent_steps WHERE kind = 'decision' AND
    decision_type = 'policy_reply'`
- Verifioidaan #49:n MaxTokens-tapaus:
  - `SELECT * FROM agent_runs WHERE status = 'truncated_max_tokens'`

Jos nämä haut palauttavat odotetut rivit ilman ad-hoc lokin
grep-pausta, design on osoittanut arvonsa ja Phase 3 voi alkaa.
