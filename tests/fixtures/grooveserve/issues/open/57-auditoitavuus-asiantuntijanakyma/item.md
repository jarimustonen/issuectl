---
created: 2026-04-29
updated: 2026-05-01
type: epic
owner: jari
status: in-progress
priority: high
related: ["#56", "#38"]
labels: [audit, expert-review, observability, ui]
---

# E57. Auditoitavuus + asiantuntijan arviointinäkymä

## Goal

Rakentaa läpinäkyvyys siihen mitä **agenttinen looppi** on tehnyt yhdelle viestille tai yhdelle käyttäjälle: työkalukutsut, päätökset, OCR-tulokset, DB-muutokset, virheet ja "permanent skip"-tapaukset. Kaksi pääkäyttäjäryhmää:

1. **Käyttäjä** — näkee oman viestinsä polun: "agentti otti vastaan 5 kuittia, tallensi 4, yhden hylkäsi syystä X". Voi pyytää korjausta.
2. **Asiantuntija (me)** — näkee kaikki epäselvät tapaukset, voi peruuttaa agentin tekemän muutoksen, vaihtaa kuitin kategorian, palauttaa aiemman OCR-version, ohjata mallin uudelleenajoa.

Tämä on **sisarepic [#56:lle](../56-toimiva-testattava-perusta/item.md)** — koordinointi tapahtuu siellä, mutta tämä epic omistaa toteutuksen yksityiskohdat. Tämän epicin worktreet käyttävät prefiksiä **`D`** (esim. `D1-agent-trace-design`).

---

## Riippuvuudet

- **Design-vaihe** voidaan tehdä **rinnakkain #56 Phase 1:n kanssa** — agent-trace-tietomallin suunnittelu ei ole kiinni siitä, onko schema vielä yhdistetty.
- **Toteutus** odottaa **#56 Phase 1:tä** — agent-trace-taulut kuuluvat jaettuun skeemaan, jotta sekä email-puoli (kirjoittaa) että web-puoli (lukee) näkevät samat rivit.
- **UI-toteutus** voidaan tehdä rinnakkain #56 Phase 3:n kanssa — molemmat ovat web-näkymiä, mutta eri kohderyhmille.

---

## Vaiheet

### Phase 1 — Tietomallin suunnittelu (rinnakkain #56 Phase 1:n)

- [x] Selvitä mitä agenttinen looppi tällä hetkellä jättää jälkeensä lokeissa ja DB:ssä (`email_processing`, `extractions`, `receipts`, `attachments`, sähköpostiviestit, log-rivit). — `analysis.md` §2
- [x] Suunnittele `agent_runs`-taulu: per viestin agent-suorituksen runko (run_id, message_id, started_at, finished_at, status, model, total_tokens). — `schema-draft.md`
- [x] Suunnittele `agent_steps`-taulu: per tool_use / per LLM-iteraatio (run_id, seq, kind, tool_name, input_json, output_json, duration_ms, error). Mahdollistaa "mitä agentti teki" -näkymän. — `schema-draft.md`
- [x] Suunnittele `agent_decisions`-taulu (tai vastaava): nimenomaiset päätöskohdat — "saved receipt", "skipped attachment", "policy reply" — selkeästi kysyttävässä muodossa. — Päätös: ei erillistä taulua, `agent_steps.kind = 'decision'` riittää (`analysis.md` §5.1).
- [x] Päätä retention/TTL. — Pysyvä MVP:ssä, ei TTL:ää (`analysis.md` §5.3).
- [x] **Tuotos**: `analysis.md` ja schema-luonnos `D1`-worktreessa. — `analysis.md`, `schema-draft.md`, `phase-2-readiness.md` (D1, 2026-04-29).

### Phase 2 — Tallennuksen toteutus (#56 Phase 1:n jälkeen)

- [x] Schema-migraatio yhdistettyyn DB:hen. _(2026-04-29 — A3-worktree, `crates/ops/migrations/018_create_agent_trace.sql` toteuttaa `schema-draft.md`-luonnoksen täysmittaisena. Käyttävä koodi #58–#62-alaissueissa.)_
- [x] **#58** AgentTraceWriter skeleton + integraatiotesti _(2026-04-30 — D2-worktree, `crates/ops/src/agent_trace.rs` + `crates/ops/tests/agent_trace.rs`. D-aallon foundation valmis; #59–#62 voivat aloittaa.)_
- [x] **#59** Email-palvelun agent-loop kirjoittaa `agent_runs`/`agent_steps`-rivejä jokaisesta käsittelystä. _(2026-04-30 — D3-worktree, `crates/server/src/ingest/agent/{mod.rs,trace.rs}` instrumentoitu. Happy path: `start_run` loopin alussa, `record_step(LlmCall)` per LLM-iteraatio (PII-vapaa metadatalla), `record_step(ToolUse)` per tool-suoritus (cap 4 KB), `finalize_run(Completed | TruncatedMaxTokens)` lopussa. Failure-haarat jättävät runin `running`-tilaan #61:tä varten. `MessagesResponse.request_id`-kenttä poimii Anthropic-API:n `request-id`-headerin. `ToolContext.trace_id`-kentän kautta `op_context_from_tool` levittää `RunRef::run_uuid()`:n alaspäin tool/op-kutsuihin.)_
- [x] **#60** Decision-rivit pre-/post-LLM reunoilla. _(2026-05-01 — `d4-agent-trace-decisions`-worktree, `crates/ops/src/agent_trace.rs::record_inline_decision_run` + `KnownDecisionType`-enum + 16 sqlx-integraatiotestiä. Wired sites: `pipeline::process_message` (spam_skip, policy_reply), `extraction::persist_permanent_skip` (per liite, evidence-linkit `linked_attachment_id`+`linked_extraction_id`), `agent::run_loop` MaxTokens-haara (`record_step(Decision)` elävälle runille — reply_truncated), `pipeline::finalize_after_smtp::AssistantThreadReply` (reply_sent, payload `{reply_message_id}`). Atomic CTE valittu vaihtoehto-(a):na issue:n §"Implementation Outline" §1:stä (vs. `start_run`+`record_step`+`finalize_run`-sekvenssi, joka jättäisi `running`-rikon ikkunan). **`unknown_sender` jätetty pois** — agent_runs.tenant_id/user_id ovat NOT NULL ja unknown-sender-haarassa kumpaakaan ei ole resoluvoituja; tapahtuma näkyy `email_processing.status='unknown_sender'`-rivinä. Decision-logissa kirjattu (`#56`). End-to-end -integraatiotesti `crates/server/tests/extraction_rescue.rs::permanent_skip_writes_decision_row`. **Phase 2 valmis, Phase 4 unblocked.**)_
- [x] **#61** Error-/abort-rivit ja `AgentError → RunStatus`-mappaus. _(2026-04-30 — `agent-trace-errors-aborts`-worktree, `crates/server/src/ingest/agent/{mod.rs,trace.rs}` finalisoivat nyt jokaisen failure-haaran. `process_with_tools` jaettu inner-`run_loop`:iin joka palauttaa `Result<(AgentReply, RunStatus), LoopFailure>`; wrapper kutsuu `trace::finalize_failure`:n yhdellä `(status, error_class, error_message)`-kolmikolla. Mappaus: `Transient`/`Permanent`/`Database` → `failed_*`, max-iters → `aborted_max_iterations`, wall-clock (pre+post-LLM) → `aborted_wall_clock`, `Unknown`+tool_use → `failed_transient` `error_class=unknown_stop_reason`. `agent_steps.kind='error'`-rivit jätetty käyttämättä — kaikki loop-sisäiset virheet surfacaavat joko `LlmCall`/`ToolUse`-stepin alla tai `agent_runs.error_class+error_message`-kombinaationa.)_
- [x] **#62** Manuaalisen runin pseudoluonti (Phase 4 ennakointi). _(2026-05-01 — `agent-trace-manual-run`-worktree, `crates/ops/src/agent_trace.rs::record_manual_run` + `ManualRunInput`. Yksi CTE-statement luo `agent_runs`-rivin (`status='completed'`, `model='manual:<email>'`, `iterations=0`) + yhden `agent_steps (kind='decision')`-rivin atomisesti — `db: impl PgExecutor<'_>` toimii sekä `&Pool`:n että `&mut Tx`:n kanssa, joten kutsuja voi folddata `audit_events`-rivin samaan transaktioon. 32 sqlx-integraatiotestiä kattaa happy-pathin, mixed-timeline-lookupin (`WHERE message_id = ?` näyttää sekä LLM- että manuaaliset runit), placeholder-message_id-syntetisoinnin, validation-matrixin (sis. trim-bug-fixin: `decision_type` ja `actor_email` trimmataan ennen tallennusta jotta `WHERE = 'reverted'` ei missaa `' reverted '`-rivejä), cross-tenant-FK-rejection ja tx-rollback-atomicityn. **`/llm-review`** + `/assess-findings` (`history/review-record-manual-run.md`) tuotti 3 FIX (sisällytetty PR:ään) + 3 SPIN-OFF kerätty `#82`-issueeksi (`agent_runs`-skeemapuhdistus ennen Phase 4:ää: actor_user_id-sarake + nullable message_id, korvaa `manual:`-prefix-konvention ja `<manual-{uuid}@…>`-placeholderin).)_
- [x] **#80** Cancellation safety — vuotavat `running`-rivit. _(2026-05-01 — `agent-runs-cancellation-safety`-worktree, vaihtoehto B (sweeper) toteutettu. Migraatio 022: `aborted_cancelled`-status + partial-indeksi `idx_agent_runs_running_started`. `crates/server/src/ingest/sweeper.rs::run_agent_runs_sweeper` (60s tikitys, 20 min stale-threshold = 2 × `MAX_AGENT_WALL_CLOCK`) folded `main.rs`:n `tokio::select!`-supervisoriin (paniikki tappaa binäärin → systemd restart, ei hiljaista degradaatiota). 4 integraatiotestiä sis. finalize-after-sweep race -testin joka lukitsee audit-immutability-invariantin (`finalize_run` → `Conflict` swepatulle riville). **Phase 4 unblocked**: "live runs"-näkymä voi nyt erottaa todelliset ja vuotaneet rivit. `/llm-review` 1 kierros (Gemini, GPT-5.5, Claude Opus, DeepSeek) — kaikki FIX-tason löydökset käsitelty: select!-supervisio, 20 min margin (15 → 20 reviewerien yksimielisestä huolesta), token-kentät jätetty koskematta, race-testi, `MissedTickBehavior::Delay`, `GREATEST(NOW(), started_at)`, sentinel-msg ilman threshold-arvoa. Raportti: `history/review-issue-80-sweeper.md`. Latenssi enintään ~21 min; Drop-guard jää follow-up:iksi jos UI vaatii nopeampaa siivoamista. #61:n caveat-osio AGENTS.md:ssä korvattu kuvauksella korjatusta tilanteesta.)_
- [x] **#82** agent_trace schema cleanup — actor_user_id-sarake + nullable message_id ennen Phase 4:ää. _(2026-05-01 — `agent-trace-schema-cleanup`-worktree, mergetty mainiin. Migraatio 021: `agent_runs.actor_user_id` (FK `users(id)`), `message_id` + `model` nullable, `agent_runs_anchor_check` + `agent_runs_model_required_for_llm_check`-CHECKit, partial-indeksi `idx_agent_runs_actor_user`, `idx_agent_runs_message` partial:ksi. `ManualRunInput.actor_email` poistettu, `record_manual_run` käyttää `ctx.actor_user_id`:tä. **Phase 4 -ennakkoehto täytetty.**)_
- [x] Yhteistyö **#38 receipt-revision-history**:n kanssa — kuittien versiot näkyvät revision-näkymässä.

### Phase 3 — Käyttäjän tapahtumaloki (DEFERRED → post-PoC/pilot)

- [ ] Käyttäjän web-UI:hin näkymä "viimeisimmät viestini" → klikkaamalla aukeaa "mitä agentti teki tälle viestille".
- [ ] Selkokielinen renderöinti: "Saimme viestin → tunnistettiin 5 liitettä → luettiin 4 kuitiksi → 1 ohitettu (ei kuitti)".
- [ ] Linkki yksittäiseen kuittiin (vie #56:n tositenäkymään).
- [ ] Pyyntö korjausta varten — joko uudelleenajo tai manuaalinen korjaus käyttäjältä.

### Phase 4 — Asiantuntijan arviointinäkymä (DONE 2026-05-01)

- [x] Lista "huomiota vaativista" tapauksista: matala OCR-confidence, agent abort, tool error, permanent skip.
- [x] Yksittäisen viestin täysi trace-näkymä (kaikki `agent_steps`).
- [x] Toiminnot:
  - Peruuta yksittäinen muutos (`undo`)
  - Palauta aiempi kuittiversio (`#38`)
  - Käynnistä viestin käsittely uudelleen (`reprocess`)
  - Merkitse "tarkastettu" / "ei korjausta tarpeen"
- [x] Audit-trail: jokainen asiantuntijatoiminto kirjautuu `audit_events`-tauluun (#26 §4.2:n malli).

Implemented in D1-expert-dashboard worktree:
- `ops::expert::list_queue` — cross-tenant attention queue with QueueReason annotations (abnormal_status, low_confidence, permanent_skip)
- `ops::expert::get_run_trace` — full timeline: run header + all agent_steps
- `ops::expert::mark_reviewed` — mark run reviewed (audit + manual_run)
- `ops::expert::request_reprocess` — flip email_processing.status (audit + manual_run)
- `ops::expert::revert_step` — revert tool_use step from latest receipt_revision
- `ops::expert::list_receipt_revisions` — revision history for a receipt
- `ops::expert::restore_receipt_from_revision` — restore receipt to any revision version
- HTTP routes: `/expert`, `/expert/runs/:uuid`, `/expert/runs/:uuid/review`, `/expert/runs/:uuid/reprocess`, `/expert/runs/:uuid/revert/:seq`, `/expert/receipts/:id/revisions`, `/expert/receipts/:id/restore/:version`
- 30 integration tests, 32 i18n strings (en/fi/sv)
- Migration 028: partial index `idx_agent_runs_unreviewed_attention`

### Phase 5 — Korjauskanavat (DEFERRED → post-PoC/pilot)

- [ ] Käyttäjä voi pyytää korjausta sähköpostilla ("hei, tämä kuitti ei mennyt oikein") → agentti tunnistaa pyynnön ja tarjoaa asiantuntijalle review-jonoon.
- [ ] Asiantuntijan korjaus voi syntyä joko web-UI:sta tai sähköpostilla (suoraan threadiin) — sama tool-pinta (#56 Unified Tool Surface).

---

## Issues

**Olemassa olevat liitokset:**
- **#38** Receipt-revision-history — kytkeytyy tähän tiukasti (revisionhistoria on osa auditia)
- **#16** Observability — AI-metriikat ja kustannukset, voivat näkyä tämän näkymän yhteydessä (mutta erillinen issue)

**Phase 2 -alaissueet** (luotu 2026-04-29 D1-designin pohjalta, kaikki blokattu #56 Phase 1:een):
- **#58 (D-nro)** agent_trace migration + AgentTraceWriter skeleton — ✅ **done 2026-04-30** (D2-worktree). Migraatio A3:ssa, writer + tyypit + sqlx-integraatiotesti D2:ssa.
- **#59** process_with_tools instrumentointi (llm_call + tool_use) — ✅ **done 2026-04-30** (D3-worktree). `crates/server/src/ingest/agent/trace.rs` + `mod.rs`-wiring; happy-path `start_run` → `record_step(LlmCall|ToolUse)` → `finalize_run(Completed | TruncatedMaxTokens)`.
- **#60** agent_trace decision-rivit pre-LLM + post-LLM reunoilla — ✅ **done 2026-05-01** (`d4-agent-trace-decisions`-worktree). `record_inline_decision_run` + `KnownDecisionType` ops:iin; pipeline.rs / extraction.rs / agent/mod.rs wired (spam_skip, policy_reply, permanent_skip, reply_truncated, reply_sent). `unknown_sender` skipattu schema-rajoitusten vuoksi.
- **#61** agent_trace error-/abort-rivit ja status-mappaus — ✅ **done 2026-04-30** (`agent-trace-errors-aborts`-worktree). `process_with_tools` finalisoi jokaisen failure-haaran (`Transient`/`Permanent`/`Database` → `failed_*`, max-iters → `aborted_max_iterations`, wall-clock → `aborted_wall_clock`, `Unknown`+tool_use → `failed_transient`/`unknown_stop_reason`); `kind='error'`-rivit jätetty käyttämättä, virheet surfacaavat `agent_runs.error_*` -kenttinä.
- **#62** Manuaalisen runin pseudoluonti (Phase 4 ennakointi) — `start_run` hyväksyy `model: &str` ilman CHECK:iä, joten `manual-revert`-pseudoarvot toimivat suoraan.
- **#80** Cancellation safety — vuotavat `running`-rivit (open, high) — #61:n LLM-review SPIN-OFF, estää Phase 4:n.

**HUOM #58 numerokollisio**: B1 loi myös `#58 gsadmin-registrations-rikki`.
A3 ei ratkaise kollisiota (käyttäjä päättää renumeroinnin erikseen) —
viittaa tarvittaessa muodossa "#58 D-nro" / "#58 B-nro".

**Luotavat myöhemmin** (Phase 3–5):
- Käyttäjän tapahtumaloki-UI (Phase 3)
- Asiantuntijan review-näkymä (Phase 4)
- Korjauskanavat ja review-jono (Phase 5)

---

## Mitä EI ole scopessa

- Yleiset prod-metriikat (latenssi, throughput, kustannukset) — ne kuuluvat **#16 observability**:iin
- Hälytysjärjestelmät / paging — myöhempi vaihe
- Käyttäjien välinen vertailu / aggregaatit — käyttäjän omat tapahtumat riittävät MVP:ssä
- ML-pohjainen "epäilyttävien" tapausten priorisointi — yksinkertainen rule-based (low confidence, errors, skips) riittää aluksi

---

## Notes

### Phase 1 -tuotos (D1, 2026-04-29)

Design-dokumentit tässä hakemistossa:

- [`analysis.md`](analysis.md) — design-päätökset, vaihtoehdot, perustelut, retentio, PII, suhde #38/#26
- [`schema-draft.md`](schema-draft.md) — `agent_runs` + `agent_steps` taulut, indeksit, FK:t, esimerkkitrace
- [`phase-2-readiness.md`](phase-2-readiness.md) — Phase 2:n riippuvuudet, työvaiheet, suositellut alaissueet

Avoimet kysymykset käyttäjälle: `analysis.md` §7. Hyväksynnän jälkeen
voidaan luoda Phase 2:n alaissueet (`phase-2-readiness.md`:n suositus).

### Miksi tämä on tärkeää MVP:lle

CLAUDE.md:n mukaan olemme MVP-vaiheessa, jossa **toiminnallisuuden oikeellisuus** on ainoa tavoite. Jotta voimme arvioida toiminnallista oikeellisuutta, meidän pitää nähdä mitä agentti tekee — muuten kehitämme sokkona. Tämä epic ei ole "nice to have", vaan vahva työkalu sille että pystymme **iteroimaan business-logiikkaa nopeasti** kun #56 on saanut käyttäjät paikalle.

### Koordinointi #56:n kanssa

- D-prefiksi worktreissä (`D1-agent-trace-design`, `D2-...`)
- Kun spawnaat D-worktreen, mainitse molemmat: `#57` omistaa scope, `#56` koordinoi
- Phase 1 design ei riipu Phase 2 toteutuksesta → niitä ei tarvitse kytkeä peräkkäisiksi
- Worktreet raportoivat **#56:n** Worktree-lokiin, jotta kokonaiskuva pysyy yhdessä paikassa
